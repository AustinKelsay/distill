//! A hub pulling its spokes' new captures over SSH, and a spoke's half of it, the export
//! (ADR 0010, issue #138). Neither side listens: the hub runs `ssh <host> distill export --since
//! <bytes>`, where `<bytes>` is the length of its copy of that spoke's manifest, so the copy is its
//! own cursor and is rebuilt with it. [`crate::archive`] writes and reads the captures.

use std::fs;
use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::archive::{self, Transfer};
use crate::config::{Config, MachineConfig};
use crate::home::Home;
use crate::tick::Deadline;
use crate::util::{append_jsonl, now_rfc3339};

/// One line of a spoke's export log: what a pull took, and when.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportEntry {
    pub at: String,
    #[serde(flatten)]
    pub sent: Transfer,
    /// Why the export stopped short, if it did: the hub hung up, or the cursor wasn't this
    /// archive's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Export this spoke's captures from byte `since` of its manifest to `out`, and log it. Only a
/// spoke exports: its `[spoke]` names it to the hub.
pub fn export(home: &Home, since: u64, out: &mut dyn Write) -> Result<Transfer> {
    let Some(spoke) = home.spoke()? else {
        bail!(
            "only a spoke exports, and this home has no [spoke]: set one up with `distill init \
             --spoke <machine>`"
        );
    };
    let result = archive::export(home, &spoke.machine, since, out);
    let entry = ExportEntry {
        at: now_rfc3339(),
        sent: result.as_ref().cloned().unwrap_or(Transfer {
            since,
            to: since,
            ..Default::default()
        }),
        error: result.as_ref().err().map(|e| format!("{e:#}")),
    };
    append_jsonl(&home.export_log_path(), &entry)?;
    result
}

/// The `since` an SSH forced command was asked for. `original` is `SSH_ORIGINAL_COMMAND`, which
/// must end `export --since <bytes>`, as [`pull`] sends it; whatever else it asks for is not run.
pub fn forced_since(original: Option<&str>) -> Result<u64> {
    let words: Vec<&str> = original.unwrap_or_default().split_whitespace().collect();
    match words.as_slice() {
        [_, .., "export", "--since", n] => n
            .parse()
            .with_context(|| format!("{n:?} is not a byte count")),
        _ => bail!("this key may only run `distill export --since <bytes>`"),
    }
}

/// What a pull from one machine took.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pulled {
    pub machine: String,
    #[serde(flatten)]
    pub took: Transfer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PullReport {
    pub machines: Vec<Pulled>,
    /// Why nothing was pulled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<String>,
}

impl PullReport {
    pub fn ok(&self) -> bool {
        self.machines.iter().all(|p| p.error.is_none())
    }

    pub fn summary(&self) -> String {
        if let Some(why) = &self.skipped {
            return format!("pull skipped: {why}");
        }
        let parts: Vec<String> = self
            .machines
            .iter()
            .map(|p| match &p.error {
                Some(e) => format!("{}: failed ({e})", p.machine),
                None => format!(
                    "{}: {} new, {}{}",
                    p.machine,
                    crate::util::count(p.took.lines, "capture"),
                    crate::util::count(p.took.blobs, "blob"),
                    if p.took.stopped {
                        ", stopped at the tick's end"
                    } else {
                        ""
                    }
                ),
            })
            .collect();
        format!("pulled {}", parts.join("; "))
    }
}

/// Pull each machine in `[[machines]]`, or only the one named `only`, until `deadline`. A machine
/// that can't be reached, or whose archive isn't the one the hub has a copy of, fails alone.
pub fn pull(
    home: &Home,
    config: &Config,
    only: Option<&str>,
    deadline: Deadline,
) -> Result<PullReport> {
    if home.spoke()?.is_some() {
        bail!("a spoke pulls from no machine: it only captures");
    }
    if config.machines.is_empty() {
        return Ok(PullReport {
            skipped: Some("no [[machines]] are set up".into()),
            ..Default::default()
        });
    }
    if let Some(name) = only
        && !config.machines.iter().any(|m| m.name == name)
    {
        bail!("no machine is named {name:?} in [[machines]]");
    }
    // Two pulls of one machine at once would append to its copy together.
    let _lock = home.lock("pull")?;
    let mut report = PullReport::default();
    for m in config
        .machines
        .iter()
        .filter(|m| only.is_none_or(|n| n == m.name))
    {
        if deadline.passed() {
            break;
        }
        report.machines.push(pull_one(home, m, deadline));
    }
    Ok(report)
}

fn pull_one(home: &Home, m: &MachineConfig, deadline: Deadline) -> Pulled {
    let since = fs::metadata(home.manifest_path_of(Some(&m.name))).map_or(0, |f| f.len());
    let mut pulled = Pulled {
        machine: m.name.clone(),
        took: Transfer {
            since,
            to: since,
            ..Default::default()
        },
        error: None,
    };
    let command = m.command.as_deref().unwrap_or("distill");
    let spawned = Command::new("ssh")
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15", "--"])
        .arg(&m.host)
        .arg(format!("{command} export --since {since}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            pulled.error = Some(format!("could not run ssh: {e}"));
            return pulled;
        }
    };
    let mut stderr = child.stderr.take().expect("piped");
    let said = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = stderr.read_to_string(&mut s);
        s
    });
    let mut stdout = BufReader::new(child.stdout.take().expect("piped"));
    let took = archive::take(home, &m.name, &mut stdout, &|| deadline.passed());
    if !matches!(&took, Ok(t) if !t.stopped) {
        let _ = child.kill();
    }
    drop(stdout);
    let status = child.wait();
    let said = said.join().unwrap_or_default();
    let last = said
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(str::trim);
    match took {
        Ok(t) => {
            let failed = status.as_ref().map_or(true, |s| !s.success());
            if failed && !t.stopped {
                pulled.error = Some(match last {
                    Some(l) => format!("ssh {}: {l}", m.host),
                    None => format!("ssh {} failed", m.host),
                });
            }
            pulled.took = t;
        }
        // What the other side said says more than a stream that ended early.
        Err(e) => {
            pulled.error = Some(match last {
                Some(l) if status.as_ref().is_ok_and(|s| !s.success()) => {
                    format!("ssh {}: {l}", m.host)
                }
                _ => format!("{e:#}"),
            });
        }
    }
    pulled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_forced_command_runs_only_an_export_since_a_byte_count() {
        for (asked, since) in [
            ("distill export --since 0", 0),
            ("/opt/bin/distill export --since 412", 412),
        ] {
            assert_eq!(forced_since(Some(asked)).unwrap(), since, "{asked}");
        }
        for asked in [
            None,
            Some(""),
            Some("distill export"),
            Some("distill read claude-code/s1"),
            Some("distill export --since 5; rm -rf ~"),
            Some("distill export --since -1"),
        ] {
            assert!(forced_since(asked).is_err(), "{asked:?}");
        }
    }
}
