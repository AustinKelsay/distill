//! The sweep: copy new and changed harness files into the raw archive. Deterministic, no model.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::archive::{Archive, CaptureKind, copy_sqlite};
use crate::config::Config;
use crate::harness::{self, Candidate, Harness};
use crate::home::Home;
use crate::normalize::{goose, hermes, opencode, t3code};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SweepReport {
    pub seen: u64,
    pub unchanged: u64,
    pub new: u64,
    pub appended: u64,
    pub rewritten: u64,
    pub excluded: u64,
    pub errors: Vec<String>,
    /// How many of the errors each harness had: its capture health for status.
    #[serde(default)]
    pub errors_by_harness: BTreeMap<Harness, u64>,
    pub bytes_written: u64,
}

impl SweepReport {
    pub fn captured(&self) -> u64 {
        self.new + self.appended + self.rewritten
    }
    pub fn summary(&self) -> String {
        format!(
            "Swept {}: {} new, {} grew, {} rewritten, {} unchanged{}{}",
            crate::util::count(self.seen, "file"),
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
fn file_excluded(config: &Config, kind: Harness, path: &std::path::Path, origin: &str) -> bool {
    if config.privacy.exclude_paths.is_empty() {
        return false;
    }
    let cwd = || harness::peek_cwd(kind, path);
    excluded(config, kind, path, origin, cwd)
}

fn excluded(
    config: &Config,
    kind: Harness,
    path: &std::path::Path,
    origin: &str,
    recorded_cwd: impl FnOnce() -> Option<String>,
) -> bool {
    let exclude = &config.privacy.exclude_paths;
    if exclude.is_empty() {
        return false;
    }
    let primary = kind.is_primary(origin);
    // A recorded cwd is exact; the folder name is lossy (`-work-private-tools` also looks like
    // a child of `-work-private`), so it is only the fallback.
    if primary && let Some(cwd) = recorded_cwd() {
        return config.is_excluded(&cwd);
    }
    match harness::path_cwd(kind, &config.roots_for(kind), path) {
        Some((cwd, enc)) => harness::path_cwd_excluded(&cwd, enc, exclude),
        // Antigravity and Hermes never say where they ran, so with any exclusion set nothing
        // from them is archived; elsewhere a session without a recorded cwd is kept.
        None => !primary || harness::records_no_cwd(kind),
    }
}

/// Sweep every enabled harness (or only `only`). The caller records the run.
pub fn sweep(home: &Home, config: &Config, only: Option<Harness>) -> Result<SweepReport> {
    home.ensure()?;
    let _lock = home.lock_archive()?;
    // What a sweep or pull that died left half-written; it is never a capture.
    crate::archive::clear_staging(home)?;
    let mut archive = Archive::open(home)?;
    let mut report = SweepReport::default();
    for sc in &config.harnesses {
        if !sc.enabled || only.is_some_and(|k| k != sc.kind) {
            continue;
        }
        let roots = config.roots_for(sc.kind);
        let failed = |report: &mut SweepReport, message: String| {
            report.errors.push(message);
            *report.errors_by_harness.entry(sc.kind).or_default() += 1;
        };
        let candidates = match harness::discover(sc.kind, &roots) {
            Ok(c) => c,
            Err(e) => {
                failed(&mut report, format!("{}: {e:#}", sc.kind));
                continue;
            }
        };
        // T3 Code's native references, read once per database.
        let mut refs: HashMap<PathBuf, HashMap<String, Vec<u8>>> = HashMap::new();
        for c in candidates {
            report.seen += 1;
            if let Err(e) = capture_one(home, config, &mut archive, &c, &mut report) {
                failed(&mut report, format!("{}: {e:#}", c.origin()));
            }
            if let Err(e) = capture_native_refs(config, &mut archive, &c, &mut refs, &mut report) {
                failed(&mut report, format!("{}: native refs: {e:#}", c.origin()));
            }
        }
    }
    Ok(report)
}

/// Whether exclusions keep a session exported from a database (a T3 Code thread, a Goose
/// session) out. One that doesn't say where it ran is kept out
/// while any exclusion is set, like the other databases.
fn db_session_excluded(config: &Config, cwd: Option<&str>) -> bool {
    if config.privacy.exclude_paths.is_empty() {
        return false;
    }
    cwd.is_none_or(|c| config.is_excluded(c))
}

/// Whether exclusions keep a spilled OpenCode tool output out. The file doesn't say where it ran,
/// so it is placed by the sessions whose tool parts name it: kept out when any of them is
/// excluded, or while none names it (yet; a later sweep tries again).
fn spill_excluded(config: &Config, db: &std::path::Path, path: &std::path::Path) -> Result<bool> {
    if config.privacy.exclude_paths.is_empty() {
        return Ok(false);
    }
    let Some(name) = path
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(opencode::spill_name)
    else {
        return Ok(true);
    };
    let projects = opencode::spill_projects(db, name)?;
    Ok(projects.is_empty() || projects.iter().any(|p| config.is_excluded(p)))
}

/// For an archived T3 Code thread, the native sessions behind its runs, kept beside it so
/// the parse step can tell which originals a mirror copies. Written whenever they change, which can
/// happen without the thread changing (a native id filled in later).
fn capture_native_refs(
    config: &Config,
    archive: &mut Archive,
    c: &Candidate,
    refs: &mut HashMap<PathBuf, HashMap<String, Vec<u8>>>,
    report: &mut SweepReport,
) -> Result<()> {
    let Candidate::T3Thread { db, thread_id, .. } = c else {
        return Ok(());
    };
    let origin = c.origin();
    // Only for threads in the archive, and only while they still pass the exclusions: a
    // thread excluded after it was first captured gets nothing new.
    if !archive.latest().contains_key(&origin) {
        return Ok(());
    }
    if !config.privacy.exclude_paths.is_empty()
        && db_session_excluded(config, t3code::thread_cwd(db, thread_id)?.as_deref())
    {
        return Ok(());
    }
    if !refs.contains_key(db) {
        refs.insert(db.clone(), t3code::export_native_refs(db)?);
    }
    let bytes = refs[db]
        .get(thread_id.as_str())
        .cloned()
        .unwrap_or_default();
    let refs_origin = format!("{origin}{}", t3code::NATIVE_SUFFIX);
    let sha = hex::encode(Sha256::digest(&bytes));
    if archive
        .latest()
        .get(&refs_origin)
        .is_some_and(|r| r.capture_hash == sha)
    {
        return Ok(());
    }
    if let Some(r) = archive.put(Harness::T3Code, &refs_origin, "refs", &bytes[..])? {
        report.bytes_written += r.size;
    }
    Ok(())
}

fn capture_one(
    home: &Home,
    config: &Config,
    archive: &mut Archive,
    c: &Candidate,
    report: &mut SweepReport,
) -> Result<()> {
    let origin = c.origin();
    let change_signal = c.change_signal();
    if archive
        .latest()
        .get(&origin)
        .is_some_and(|r| r.change_signal == change_signal)
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
            archive.put(*kind, &origin, &change_signal, fs::File::open(path)?)?
        }
        Candidate::OpenCodeSpill { db, path, .. } => {
            if spill_excluded(config, db, path)? {
                report.excluded += 1;
                return Ok(());
            }
            archive.put(
                Harness::OpenCode,
                &origin,
                &change_signal,
                fs::File::open(path)?,
            )?
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
            archive.put(*kind, &origin, &change_signal, &bytes[..])?
        }
        Candidate::T3Thread { db, thread_id, .. } => {
            let export = t3code::export_thread(db, thread_id)?;
            if db_session_excluded(config, export.cwd.as_deref()) {
                report.excluded += 1;
                return Ok(());
            }
            archive.put(Harness::T3Code, &origin, &change_signal, &export.bytes[..])?
        }
        Candidate::DbSession {
            kind,
            db,
            session_id,
            ..
        } => {
            let (bytes, cwd) = match kind {
                Harness::Hermes => {
                    let e = hermes::export_session(db, session_id)?;
                    (e.bytes, e.cwd)
                }
                _ => {
                    let e = goose::export_session(db, session_id)?;
                    (e.bytes, e.working_dir)
                }
            };
            if db_session_excluded(config, cwd.as_deref()) {
                report.excluded += 1;
                return Ok(());
            }
            archive.put(*kind, &origin, &change_signal, &bytes[..])?
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
                Harness::OpenCode,
                &origin,
                &change_signal,
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
            harnesses: vec![],
            ..Default::default()
        };
        let c = config.harness_mut(Harness::ClaudeCode);
        c.enabled = true;
        c.roots = vec![claude.clone()];
        let c = config.harness_mut(Harness::Codex);
        c.enabled = true;
        c.roots = vec![codex.clone()];
        config.privacy.exclude_paths = vec!["/work/secret".into()];

        let r = sweep(&home, &config, None).unwrap();
        assert_eq!(r.new, 3, "{r:?}"); // s1.jsonl, out.txt, the .zst; not the lock file
        assert_eq!(r.excluded, 2, "{r:?}");
        let n = crate::normalize::normalize(&home, false).unwrap();
        assert_eq!(n.sidecars, 1, "{n:?}");
        assert_eq!(n.normalized, 2, "{n:?}");
        let meta = crate::normalize::find(&home, None, Harness::Codex, "z1")
            .unwrap()
            .unwrap();
        assert_eq!(meta.counts.user, 1);
    }

    #[test]
    fn exclusions_cover_dotted_droid_folders_and_harnesses_without_cwd() {
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
            harnesses: vec![],
            ..Default::default()
        };
        let c = config.harness_mut(Harness::Droid);
        c.enabled = true;
        c.roots = vec![root.join("factory/sessions")];
        let c = config.harness_mut(Harness::Antigravity);
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
            harnesses: vec![],
            ..Default::default()
        };
        let c = config.harness_mut(Harness::Codex);
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
            harnesses: vec![],
            ..Default::default()
        };
        let c = config.harness_mut(Harness::Codex);
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
                let meta = crate::normalize::find(&home, None, Harness::Codex, "c1")
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    meta.counts.user, 2,
                    "{label}, force={force}: newer capture kept"
                );
            }
        };
        check("moved and compressed");
        // The old file reappears with its old bytes and a new mtime: a change-signal refresh,
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
    fn old_bytes_returning_to_an_old_codex_file_do_not_outrank_newer_content() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        let session = |turns: usize| {
            let mut s = String::from(
                r#"{"timestamp":"2026-11-01T00:00:00Z","type":"session_meta","payload":{"id":"c1","cwd":"/w"}}"#,
            );
            for i in 0..turns {
                s.push_str(&format!("\n{{\"timestamp\":\"2026-11-01T00:00:0{i}Z\",\"type\":\"response_item\",\"payload\":{{\"type\":\"message\",\"role\":\"user\",\"content\":[{{\"type\":\"input_text\",\"text\":\"t{i}\"}}]}}}}"));
            }
            s
        };
        let (a, b) = ("/s/rollout-c1.jsonl", "/arch/rollout-c1.jsonl.zst");
        let zst = |t: usize| zstd::stream::encode_all(session(t).as_bytes(), 3).unwrap();
        let mut ar = crate::archive::Archive::open(&home).unwrap();
        ar.put(Harness::Codex, a, "1", session(1).as_bytes())
            .unwrap(); // A: X
        ar.put(Harness::Codex, a, "2", session(2).as_bytes())
            .unwrap(); // A: Y
        ar.put(Harness::Codex, b, "3", &zst(3)[..]).unwrap(); //           B: Z (newest content)
        ar.put(Harness::Codex, a, "4", session(1).as_bytes())
            .unwrap(); // A: X again
        for force in [false, true] {
            crate::normalize::normalize(&home, force).unwrap();
            let meta = crate::normalize::find(&home, None, Harness::Codex, "c1")
                .unwrap()
                .unwrap();
            assert_eq!(meta.counts.user, 3, "force={force}");
        }
    }

    #[test]
    fn a_codex_copy_suffixed_by_an_earlier_run_is_merged_into_one_session() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        let body = r#"{"timestamp":"2026-11-01T00:00:00Z","type":"session_meta","payload":{"id":"c1","cwd":"/w"}}
{"timestamp":"2026-11-01T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"hi"}]}}"#;
        let (a, b) = ("/s/rollout-c1.jsonl", "/arch/rollout-c1.jsonl");
        let mut ar = crate::archive::Archive::open(&home).unwrap();
        ar.put(Harness::Codex, a, "1", body.as_bytes()).unwrap();
        ar.put(Harness::Codex, b, "2", body.as_bytes()).unwrap();
        // What an earlier version left: `a` as c1 and `b` as a suffixed copy.
        crate::normalize::normalize(&home, false).unwrap();
        let suffixed = format!("c1-{}", &crate::util::sha256_hex(b.as_bytes())[..8]);
        let src = home.session_dir(None, Harness::Codex, "c1");
        let dst = home.session_dir(None, Harness::Codex, &suffixed);
        std::fs::create_dir_all(&dst).unwrap();
        let mut meta: crate::model::SessionMeta =
            serde_json::from_str(&std::fs::read_to_string(src.join("meta.json")).unwrap()).unwrap();
        meta.session_id = suffixed.clone();
        meta.raw.origin = b.to_string();
        crate::util::write_json_pretty(&dst.join("meta.json"), &meta).unwrap();
        std::fs::copy(src.join("events.jsonl"), dst.join("events.jsonl")).unwrap();
        let state_path = home.state_dir().join("normalize.json");
        let mut state: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&state_path).unwrap()).unwrap();
        state[b]["session"] = serde_json::Value::String(suffixed.clone());
        crate::util::write_json_pretty(&state_path, &state).unwrap();
        crate::normalize::rebuild_session_list(&home).unwrap();
        assert_eq!(crate::normalize::read_session_list(&home).unwrap().len(), 2);

        let n = crate::normalize::normalize(&home, false).unwrap();
        assert_eq!(n.sessions, 1, "{n:?}");
        assert!(!dst.exists(), "the suffixed copy is gone");
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
            harnesses: vec![],
            ..Default::default()
        };
        let c = config.harness_mut(Harness::ClaudeCode);
        c.enabled = true;
        c.roots = vec![root.join("claude/projects")];
        sweep(&home, &config, None).unwrap();
        let n = crate::normalize::normalize(&home, false).unwrap();
        assert_eq!(n.sessions, 2, "{n:?}");
    }

    #[test]
    fn folder_cwd_is_read_below_the_harness_root_and_recorded_cwd_wins() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("backup/projects/snap/.claude/projects");
        std::fs::create_dir_all(root.join("-work-secret/s1")).unwrap();
        std::fs::create_dir_all(root.join("-work-private-tools")).unwrap();
        std::fs::write(root.join("-work-secret/s1/out.txt"), "tool output").unwrap();
        std::fs::write(
            root.join("-work-private-tools/s2.jsonl"),
            r#"{"type":"user","cwd":"/work/private-tools","timestamp":"2026-11-01T00:00:00Z","message":{"role":"user","content":"hi"}}"#,
        )
        .unwrap();
        let home = Home::new(dir.path().join("home"));
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        let c = config.harness_mut(Harness::ClaudeCode);
        c.enabled = true;
        c.roots = vec![root.clone()];
        config.privacy.exclude_paths = vec!["/work/secret".into(), "/work/private".into()];
        let r = sweep(&home, &config, None).unwrap();
        // The sidecar under -work-secret is excluded even though an ancestor folder is also
        // called "projects"; the sibling repo /work/private-tools is kept.
        assert_eq!((r.new, r.excluded), (1, 1), "{r:?}");
        assert!(
            harness::percent_decode("%€%2Fa").ends_with("/a"),
            "no panic on non-ASCII"
        );
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
                    harnesses: vec![],
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
