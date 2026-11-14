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
        Some((cwd, enc)) => source::path_cwd_excluded(&cwd, enc, exclude),
        // Antigravity and Hermes never say where they ran, so with any exclusion set nothing
        // from them is archived; elsewhere a session without a recorded cwd is kept.
        None => !primary || source::records_no_cwd(kind),
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
    fn exclusions_cover_dotted_droid_folders_and_sources_without_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let droid = root.join("factory/sessions/-work-private.repo");
        std::fs::create_dir_all(&droid).unwrap();
        std::fs::write(droid.join("d1.settings.json"), "{}").unwrap();
        let ag = root.join("ag/brain/c1/.system_generated/logs");
        std::fs::create_dir_all(&ag).unwrap();
        std::fs::write(
            ag.join("transcript_full.jsonl"),
            r#"{"type":"USER_INPUT","content":"hi"}"#,
        )
        .unwrap();

        let home = Home::new(root.join("home"));
        let mut config = Config {
            sources: vec![],
            ..Default::default()
        };
        let c = config.source_mut(SourceKind::Droid);
        c.enabled = true;
        c.roots = vec![root.join("factory/sessions")];
        let c = config.source_mut(SourceKind::Antigravity);
        c.enabled = true;
        c.roots = vec![root.join("ag")];
        config.privacy.exclude_paths = vec!["/work/private.repo".into()];
        let r = sweep(&home, &config, None).unwrap();
        assert_eq!((r.new, r.excluded), (0, 2), "{r:?}");

        config.privacy.exclude_paths.clear();
        let r = sweep(&home, &config, None).unwrap();
        assert_eq!(r.new, 2, "{r:?}");
    }

    #[test]
    fn a_codex_session_compressed_later_stays_one_session() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let codex = root.join("codex");
        std::fs::create_dir_all(&codex).unwrap();
        let rollout = r#"{"timestamp":"2026-11-01T00:00:00Z","type":"session_meta","payload":{"id":"c1","cwd":"/w"}}
{"timestamp":"2026-11-01T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}}
"#;
        std::fs::write(codex.join("rollout-c1.jsonl"), rollout).unwrap();
        let home = Home::new(root.join("home"));
        let mut config = Config {
            sources: vec![],
            ..Default::default()
        };
        let c = config.source_mut(SourceKind::Codex);
        c.enabled = true;
        c.roots = vec![codex.clone()];
        sweep(&home, &config, None).unwrap();
        crate::normalize::normalize(&home, false).unwrap();
        std::fs::remove_file(codex.join("rollout-c1.jsonl")).unwrap();
        std::fs::write(
            codex.join("rollout-c1.jsonl.zst"),
            zstd::stream::encode_all(rollout.as_bytes(), 3).unwrap(),
        )
        .unwrap();
        sweep(&home, &config, None).unwrap();
        let n = crate::normalize::normalize(&home, false).unwrap();
        assert_eq!(n.sessions, 1, "{n:?}");
    }

    #[test]
    fn the_newest_capture_of_a_moved_codex_file_wins_even_on_rebuild() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (live, archived) = (
            root.join("codex/sessions"),
            root.join("codex/archived_sessions"),
        );
        std::fs::create_dir_all(&live).unwrap();
        std::fs::create_dir_all(&archived).unwrap();
        let head = r#"{"timestamp":"2026-11-01T00:00:00Z","type":"session_meta","payload":{"id":"c1","cwd":"/w"}}
"#;
        let turn = |t: &str| {
            format!(
                r#"{{"timestamp":"2026-11-01T00:00:01Z","type":"response_item","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"{t}"}}]}}}}"#
            ) + "\n"
        };
        std::fs::write(
            live.join("rollout-c1.jsonl"),
            format!("{head}{}", turn("one")),
        )
        .unwrap();
        let home = Home::new(root.join("home"));
        let mut config = Config {
            sources: vec![],
            ..Default::default()
        };
        let c = config.source_mut(SourceKind::Codex);
        c.enabled = true;
        c.roots = vec![root.join("codex")];
        sweep(&home, &config, None).unwrap();
        crate::normalize::normalize(&home, false).unwrap();
        // No sleep: both captures land in the same second, so order must come from the manifest.
        let old_bytes = std::fs::read(live.join("rollout-c1.jsonl")).unwrap();
        std::fs::remove_file(live.join("rollout-c1.jsonl")).unwrap();
        let both = format!("{head}{}{}", turn("one"), turn("two"));
        std::fs::write(
            archived.join("rollout-c1.jsonl.zst"),
            zstd::stream::encode_all(both.as_bytes(), 3).unwrap(),
        )
        .unwrap();
        sweep(&home, &config, None).unwrap();
        let check = |label: &str| {
            for force in [false, true] {
                let n = crate::normalize::normalize(&home, force).unwrap();
                assert_eq!(n.sessions, 1, "{label}: {n:?}");
                let meta = crate::normalize::find(&home, SourceKind::Codex, "c1")
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    meta.counts.user, 2,
                    "{label}, force={force}: newer capture kept"
                );
            }
        };
        check("moved and compressed");
        // The old file reappears with its old bytes and a new mtime: a fingerprint refresh,
        // not newer content.
        std::fs::write(live.join("rollout-c1.jsonl"), &old_bytes).unwrap();
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(5);
        std::fs::File::options()
            .write(true)
            .open(live.join("rollout-c1.jsonl"))
            .unwrap()
            .set_modified(later)
            .unwrap();
        sweep(&home, &config, None).unwrap();
        check("old file touched");
    }

    #[test]
    fn distinct_files_with_the_same_name_stay_separate_outside_codex() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for (d, text) in [("a", "first"), ("b", "second")] {
            let p = root.join(format!("claude/projects/-{d}"));
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(
                p.join("shared.jsonl"),
                format!(r#"{{"type":"user","cwd":"/{d}","timestamp":"2026-11-01T00:00:00Z","message":{{"role":"user","content":"{text}"}}}}"#),
            )
            .unwrap();
        }
        let home = Home::new(root.join("home"));
        let mut config = Config {
            sources: vec![],
            ..Default::default()
        };
        let c = config.source_mut(SourceKind::ClaudeCode);
        c.enabled = true;
        c.roots = vec![root.join("claude/projects")];
        sweep(&home, &config, None).unwrap();
        let n = crate::normalize::normalize(&home, false).unwrap();
        assert_eq!(n.sessions, 2, "{n:?}");
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
