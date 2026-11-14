//! The sweep: copy new and changed session data into the archive. Deterministic, no model.

use std::fs;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::archive::{Archive, CaptureKind, copy_sqlite};
use crate::config::Config;
use crate::home::Home;
use crate::normalize::{opencode, t3code};
use crate::runlog::{self, RunEntry};
use crate::source::{self, Candidate, SourceKind};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SweepReport {
    pub seen: u64,
    pub unchanged: u64,
    pub new: u64,
    pub appended: u64,
    pub rewritten: u64,
    pub excluded: u64,
    pub errors: Vec<String>,
    pub bytes_written: u64,
}

impl SweepReport {
    pub fn captured(&self) -> u64 {
        self.new + self.appended + self.rewritten
    }
    pub fn summary(&self) -> String {
        format!(
            "Swept {} sources: {} new, {} grew, {} rewritten, {} unchanged{}{}",
            self.seen,
            self.new,
            self.appended,
            self.rewritten,
            self.unchanged,
            if self.excluded > 0 {
                format!(", {} excluded", self.excluded)
            } else {
                String::new()
            },
            if self.errors.is_empty() {
                String::new()
            } else {
                format!(", {} errors", self.errors.len())
            },
        )
    }
}

/// Whether privacy exclusions keep a file out of the archive. A session's cwd comes from its
/// records or from the folder it sits in; a sidecar whose session can't be placed is kept out
/// whenever any exclusion is set.
fn file_excluded(config: &Config, kind: SourceKind, path: &std::path::Path, origin: &str) -> bool {
    let exclude = &config.privacy.exclude_paths;
    if exclude.is_empty() {
        return false;
    }
    let primary = kind.is_primary(origin);
    if primary
        && let Some(cwd) = source::peek_cwd(kind, path)
        && config.is_excluded(&cwd)
    {
        return true;
    }
    match source::path_cwd(kind, path) {
        Some((cwd, lossy)) => source::path_cwd_excluded(&cwd, lossy, exclude),
        None => !primary,
    }
}

pub fn sweep(home: &Home, config: &Config, only: Option<SourceKind>) -> Result<SweepReport> {
    home.ensure()?;
    let _lock = home.lock_archive()?;
    let mut archive = Archive::open(home)?;
    let mut report = SweepReport::default();
    for sc in &config.sources {
        if !sc.enabled || only.is_some_and(|k| k != sc.kind) {
            continue;
        }
        let roots = config.roots_for(sc.kind);
        let candidates = match source::discover(sc.kind, &roots) {
            Ok(c) => c,
            Err(e) => {
                report.errors.push(format!("{}: {e:#}", sc.kind));
                continue;
            }
        };
        for c in candidates {
            report.seen += 1;
            if let Err(e) = capture_one(home, config, &mut archive, &c, &mut report) {
                report.errors.push(format!("{}: {e:#}", c.origin()));
            }
        }
    }
    let mut entry = RunEntry::new("sweep", report.errors.is_empty(), report.summary());
    entry.details = serde_json::to_value(&report)?;
    runlog::record(home, &entry)?;
    Ok(report)
}

fn capture_one(
    home: &Home,
    config: &Config,
    archive: &mut Archive,
    c: &Candidate,
    report: &mut SweepReport,
) -> Result<()> {
    let origin = c.origin();
    let fingerprint = c.fingerprint();
    if archive
        .latest()
        .get(&origin)
        .is_some_and(|r| r.fingerprint == fingerprint)
    {
        report.unchanged += 1;
        return Ok(());
    }
    let rec = match c {
        Candidate::File { kind, path, .. } => {
            if file_excluded(config, *kind, path, &origin) {
                report.excluded += 1;
                return Ok(());
            }
            let _ = home;
            archive.put(*kind, &origin, &fingerprint, fs::File::open(path)?)?
        }
        Candidate::Sqlite { kind, path, .. } => {
            // Databases (Cursor chats, conversation stores, T3 Code's threads) don't say which
            // repository each row belongs to, so exclusions can't be checked. Fail closed: with
            // any exclusion set, they are not archived.
            if !config.privacy.exclude_paths.is_empty() {
                report.excluded += 1;
                return Ok(());
            }
            let bytes = copy_sqlite(path)?;
            archive.put(*kind, &origin, &fingerprint, &bytes[..])?
        }
        Candidate::T3Thread { db, thread_id, .. } => {
            let export = t3code::export_thread(db, thread_id)?;
            if let Some(cwd) = &export.cwd
                && config.is_excluded(cwd)
            {
                report.excluded += 1;
                return Ok(());
            }
            archive.put(SourceKind::T3Code, &origin, &fingerprint, &export.bytes[..])?
        }
        Candidate::OpenCodeSession { db, session_id, .. } => {
            let export = opencode::export_session(db, session_id)?;
            if let Some(cwd) = &export.directory
                && config.is_excluded(cwd)
            {
                report.excluded += 1;
                return Ok(());
            }
            archive.put(
                SourceKind::OpenCode,
                &origin,
                &fingerprint,
                &export.bytes[..],
            )?
        }
    };
    match rec {
        None => report.unchanged += 1,
        Some(r) => {
            report.bytes_written += r.size;
            match r.kind {
                CaptureKind::New => report.new += 1,
                CaptureKind::Append => report.appended += 1,
                CaptureKind::Rewrite => report.rewritten += 1,
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_sidecars_and_compressed_sessions_and_honors_exclusions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let claude = root.join("claude/projects");
        let codex = root.join("codex");
        std::fs::create_dir_all(claude.join("-work-app/s1/tool-results")).unwrap();
        std::fs::create_dir_all(claude.join("-work-secret-repo")).unwrap();
        std::fs::create_dir_all(&codex).unwrap();
        let line = |cwd: &str| {
            format!(
                r#"{{"type":"user","cwd":"{cwd}","timestamp":"2026-11-01T00:00:00Z","message":{{"role":"user","content":"hi"}}}}"#
            )
        };
        std::fs::write(claude.join("-work-app/s1.jsonl"), line("/work/app")).unwrap();
        std::fs::write(
            claude.join("-work-app/s1/tool-results/out.txt"),
            "big tool output",
        )
        .unwrap();
        std::fs::write(claude.join("-work-app/s1.jsonl.lock"), "").unwrap();
        // Excluded repo: neither the session nor its sidecar is archived.
        std::fs::write(
            claude.join("-work-secret-repo/s2.jsonl"),
            line("/work/secret/repo"),
        )
        .unwrap();
        std::fs::write(claude.join("-work-secret-repo/notes.md"), "private").unwrap();
        let rollout = r#"{"timestamp":"2026-11-01T00:00:00Z","type":"session_meta","payload":{"id":"z1","cwd":"/work/app"}}
{"timestamp":"2026-11-01T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"compressed hello"}]}}
"#;
        std::fs::write(
            codex.join("rollout-z1.jsonl.zst"),
            zstd::stream::encode_all(rollout.as_bytes(), 3).unwrap(),
        )
        .unwrap();

        let home = Home::new(root.join("home"));
        let mut config = Config {
            sources: vec![],
            ..Default::default()
        };
        let c = config.source_mut(SourceKind::ClaudeCode);
        c.enabled = true;
        c.roots = vec![claude.clone()];
        let c = config.source_mut(SourceKind::Codex);
        c.enabled = true;
        c.roots = vec![codex.clone()];
        config.privacy.exclude_paths = vec!["/work/secret".into()];

        let r = sweep(&home, &config, None).unwrap();
        assert_eq!(r.new, 3, "{r:?}"); // s1.jsonl, out.txt, the .zst; not the lock file
        assert_eq!(r.excluded, 2, "{r:?}");
        let n = crate::normalize::normalize(&home, false).unwrap();
        assert_eq!(n.sidecars, 1, "{n:?}");
        assert_eq!(n.normalized, 2, "{n:?}");
        let meta = crate::normalize::find(&home, SourceKind::Codex, "z1")
            .unwrap()
            .unwrap();
        assert_eq!(meta.counts.user, 1);
    }

    #[test]
    fn a_sweep_waits_for_the_archive_lock() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        let held = home.lock_archive().unwrap();
        let h2 = home.clone();
        let t = std::thread::spawn(move || {
            sweep(
                &h2,
                &Config {
                    sources: vec![],
                    ..Default::default()
                },
                None,
            )
        });
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            !t.is_finished(),
            "sweep ran while another writer held the lock"
        );
        drop(held);
        t.join().unwrap().unwrap();
    }
}
