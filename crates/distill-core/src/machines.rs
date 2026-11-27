//! A hub pulling its spokes' new captures over SSH, and a spoke's half of it, the export
//! (ADR 0010, issue #138). Neither side listens: the hub runs `ssh <host> distill export --since
//! <bytes>`, where `<bytes>` is the length of its copy of that spoke's manifest, so the copy is its
//! own cursor and is rebuilt with it. [`crate::archive`] writes and reads the captures.

use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

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
    // A sweep here waits, so the manifest and the blobs it names stay as they were read.
    let _lock = home.lock_archive()?;
    // What went before an export stopped short (the hub hung up) is logged too.
    let mut sent = Transfer::default();
    let result = archive::export(home, &spoke.machine, since, out, &mut sent);
    let entry = ExportEntry {
        at: now_rfc3339(),
        sent: sent.clone(),
        error: result.as_ref().err().map(|e| format!("{e:#}")),
    };
    append_jsonl(&home.export_log_path(), &entry)?;
    result.map(|()| sent)
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
    /// When this machine's pull ended: a tick logs it only once all its steps are done.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
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
    // The archive lock, as the sweep and a backup take it: two pulls at once would append to one
    // copy together, and a backup would read a copy a pull is changing. The tick doesn't wait.
    let _lock = if deadline == Deadline::NONE {
        home.lock_archive()?
    } else {
        match home.try_lock("archive")? {
            Some(lock) => lock,
            None => {
                return Ok(PullReport {
                    skipped: Some("a sweep, a backup or another pull holds the archive".into()),
                    ..Default::default()
                });
            }
        }
    };
    let mut report = PullReport::default();
    for m in config
        .machines
        .iter()
        .filter(|m| only.is_none_or(|n| n == m.name))
    {
        if deadline.passed() {
            break;
        }
        let mut pulled = pull_one(home, m, deadline, "ssh", QUIET);
        pulled.at = Some(now_rfc3339());
        report.machines.push(pulled);
    }
    Ok(report)
}

/// How long a pull waits for the next bytes before it gives up. A spoke sends without a pause
/// once ssh is in, so a capture still coming is let finish past the deadline, and an export
/// that went quiet is not waited on.
const QUIET: Duration = Duration::from_secs(60);

/// Counts the bytes read through it, for the watch on a pull, and stops waiting for more once
/// the watch gave up: a process ssh left behind can hold the pipe open after ssh is gone.
struct Counted<R> {
    inner: R,
    seen: Arc<AtomicU64>,
    gave_up: Arc<AtomicBool>,
}

impl<R: Read + std::os::fd::AsRawFd> Read for Counted<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut ready = libc::pollfd {
            fd: self.inner.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            if self.gave_up.load(Ordering::Relaxed) {
                return Err(std::io::ErrorKind::TimedOut.into());
            }
            // SAFETY: one valid pollfd, for a descriptor `inner` keeps open.
            match unsafe { libc::poll(&mut ready, 1, 100) } {
                0 => {}
                -1 => {
                    let e = std::io::Error::last_os_error();
                    if e.kind() != std::io::ErrorKind::Interrupted {
                        return Err(e);
                    }
                }
                _ => break,
            }
        }
        let n = self.inner.read(buf)?;
        self.seen.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

fn pull_one(
    home: &Home,
    m: &MachineConfig,
    deadline: Deadline,
    ssh: &str,
    quiet: Duration,
) -> Pulled {
    let mut pulled = Pulled {
        machine: m.name.clone(),
        at: None,
        took: Transfer::default(),
        error: None,
    };
    let since = match archive::copy_length(home, &m.name) {
        Ok(n) => n,
        Err(e) => {
            pulled.error = Some(format!("{e:#}"));
            return pulled;
        }
    };
    pulled.took.since = since;
    pulled.took.to = since;
    let command = m.command.as_deref().unwrap_or("distill");
    let spawned = Command::new(ssh)
        // No prompt, so the timer can run it; and a connection that goes quiet is dropped
        // rather than holding the tick.
        .args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=15"])
        .args([
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=4",
            "--",
        ])
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
    // What ssh says, gathered as it comes: a proxy it started can hold the pipe open after ssh
    // exits, so the pull waits for its end only briefly.
    let said = Arc::new(Mutex::new(Vec::new()));
    let drain = {
        let (mut stderr, said) = (child.stderr.take().expect("piped"), said.clone());
        std::thread::spawn(move || {
            let mut buf = [0; 4096];
            while let Ok(n @ 1..) = stderr.read(&mut buf) {
                said.lock().map(|mut s| s.extend_from_slice(&buf[..n])).ok();
            }
        })
    };
    let seen = Arc::new(AtomicU64::new(0));
    let (done, gave_up) = (
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
    );
    let mut stdout = BufReader::new(Counted {
        inner: child.stdout.take().expect("piped"),
        seen: seen.clone(),
        gave_up: gave_up.clone(),
    });
    // An export that sends nothing for `quiet` is stopped, so a spoke that hangs holds neither
    // the tick nor the archive lock.
    let child = Arc::new(Mutex::new(child));
    let watch = {
        let (child, done, gave_up) = (child.clone(), done.clone(), gave_up.clone());
        std::thread::spawn(move || {
            let (mut bytes, mut since_bytes) = (0, Instant::now());
            while !done.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                let now = seen.load(Ordering::Relaxed);
                if now != bytes {
                    (bytes, since_bytes) = (now, Instant::now());
                } else if since_bytes.elapsed() >= quiet {
                    gave_up.store(true, Ordering::Relaxed);
                    let _ = child.lock().map(|mut c| c.kill());
                    return;
                }
            }
        })
    };
    let took = archive::take_into(
        home,
        &m.name,
        &mut stdout,
        &|| deadline.passed(),
        &mut pulled.took,
    );
    if took.is_err() || pulled.took.stopped {
        let _ = child.lock().map(|mut c| c.kill());
    }
    drop(stdout);
    let status = loop {
        match child.lock().map(|mut c| c.try_wait()) {
            Ok(Ok(Some(status))) => break Ok(status),
            Ok(Ok(None)) => std::thread::sleep(Duration::from_millis(20)),
            Ok(Err(e)) => break Err(e),
            Err(_) => break Err(std::io::Error::other("the watch on ssh failed")),
        }
    };
    done.store(true, Ordering::Relaxed);
    let _ = watch.join();
    let waited = Instant::now();
    while !drain.is_finished() && waited.elapsed() < Duration::from_secs(1) {
        std::thread::sleep(Duration::from_millis(20));
    }
    let said =
        String::from_utf8_lossy(&said.lock().map(|s| s.clone()).unwrap_or_default()).into_owned();
    if took.is_err() && gave_up.load(Ordering::Relaxed) {
        pulled.error = Some(format!(
            "{} sent nothing for {}s, so the pull stopped",
            m.host,
            quiet.as_secs()
        ));
        return pulled;
    }
    let last = said
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(str::trim);
    match took {
        Ok(()) => {
            let failed = status.as_ref().map_or(true, |s| !s.success());
            if failed && !pulled.took.stopped {
                pulled.error = Some(match last {
                    Some(l) => format!("ssh {}: {l}", m.host),
                    None => format!("ssh {} failed", m.host),
                });
            }
        }
        // What the other side said says more than a stream that ended early. When the pull
        // refused what came, ssh only says how it was stopped.
        Err(e) => {
            let ended = format!("{e:#}").starts_with(archive::ENDED_EARLY);
            pulled.error = Some(match last {
                Some(l) if ended && status.as_ref().is_ok_and(|s| !s.success()) => {
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
    use crate::archive::Archive;
    use crate::config::Spoke;
    use crate::harness::Harness;
    use std::fs;

    fn hub_of(dir: &std::path::Path) -> (Home, Config) {
        let home = Home::new(dir.join("hub"));
        home.ensure().unwrap();
        let config = Config {
            machines: vec![MachineConfig {
                name: "nas".into(),
                host: "me@nas".into(),
                command: None,
            }],
            ..Config::default()
        };
        (home, config)
    }

    #[test]
    fn the_tick_doesnt_wait_on_a_sweep_or_backup_holding_the_archive() {
        let dir = tempfile::tempdir().unwrap();
        let (home, config) = hub_of(dir.path());
        let _held = home.lock_archive().unwrap();
        let r = pull(
            &home,
            &config,
            None,
            Deadline::after(Duration::from_secs(60)),
        )
        .unwrap();
        assert!(r.machines.is_empty(), "{r:?}");
        assert!(r.skipped.unwrap().contains("holds the archive"));
    }

    #[cfg(unix)]
    #[test]
    fn an_export_that_goes_quiet_is_stopped() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (home, config) = hub_of(dir.path());
        // A stand-in for ssh: a header for an empty copy, then nothing, and a process left
        // behind that holds its output open.
        let ssh = dir.path().join("ssh");
        use sha2::Digest;
        let empty = hex::encode(sha2::Sha256::digest(b""));
        fs::write(
            &ssh,
            format!(
                "#!/bin/sh\nprintf 'distill-export 1\\n{{\"machine\":\"nas\",\"since\":0,\"prefix\":\"{empty}\",\"end\":0}}\\n'\nsleep 20 &\nexec sleep 30\n"
            ),
        )
        .unwrap();
        fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
        let start = Instant::now();
        let p = pull_one(
            &home,
            &config.machines[0],
            Deadline::NONE,
            ssh.to_str().unwrap(),
            Duration::from_millis(500),
        );
        assert!(start.elapsed() < Duration::from_secs(10), "{p:?}");
        assert!(p.error.unwrap().contains("sent nothing"));
    }

    /// Takes `room` bytes, then fails as a pipe whose reader went away does.
    struct HungUp {
        room: usize,
    }

    impl Write for HungUp {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.room == 0 {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            let n = buf.len().min(self.room);
            self.room -= n;
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn an_export_the_hub_hung_up_on_logs_what_went_before() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        Config {
            spoke: Some(Spoke {
                machine: "nas".into(),
            }),
            ..Config::default()
        }
        .save(&home.config_path())
        .unwrap();
        let mut a = Archive::open(&home).unwrap();
        a.put(Harness::Pi, "s", "1", &b"one\n"[..]).unwrap();
        a.put(Harness::Pi, "t", "1", &b"two\n"[..]).unwrap();
        let mut whole = Vec::new();
        export(&home, 0, &mut whole).unwrap();
        // Room for everything but the last line and the end.
        let at = whole.windows(5).rposition(|w| w == b"line ").unwrap();
        assert!(export(&home, 0, &mut HungUp { room: at }).is_err());
        let log = fs::read_to_string(home.export_log_path()).unwrap();
        let last: ExportEntry = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!((last.sent.lines, last.sent.blobs), (1, 2), "{last:?}");
        assert!(last.error.is_some(), "{last:?}");

        // Cut inside a large blob: what of it went is counted.
        let big = "x".repeat(99_999) + "\n";
        a.put(Harness::Pi, "u", "1", big.as_bytes()).unwrap();
        let mut whole = Vec::new();
        export(&home, 0, &mut whole).unwrap();
        let at = whole.windows(5).rposition(|w| w == b"blob ").unwrap();
        let payload = at + whole[at..].iter().position(|b| *b == b'\n').unwrap() + 1;
        assert!(export(&home, 0, &mut HungUp { room: at + 80_000 }).is_err());
        let log = fs::read_to_string(home.export_log_path()).unwrap();
        let last: ExportEntry = serde_json::from_str(log.lines().last().unwrap()).unwrap();
        assert_eq!(last.sent.blobs, 2, "{last:?}");
        assert_eq!(
            last.sent.bytes,
            (8 + at + 80_000 - payload) as u64,
            "{last:?}"
        );
    }

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
