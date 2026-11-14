//! One-time import from the Distill this one replaced (`~/.distill/distill.db`).
//!
//! The old app kept no raw files for Claude Code; it stored each line of a session file as a
//! `capture_records` row. Joined back in order, those rows hash to the capture's recorded
//! `raw_sha256`, so the original file is recovered exactly. That matters because Claude Code
//! deletes old sessions: some exist nowhere else.
//!
//! Only captures whose rebuilt bytes match their recorded hash are imported, and only for
//! origins the archive has never seen; anything the new sweep already captured stays as it is.

use std::collections::HashSet;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::archive::Archive;
use crate::config::Config;
use crate::home::Home;
use crate::runlog::{self, RunEntry};
use crate::source::SourceKind;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LegacyReport {
    /// Legacy captures read.
    pub seen: u64,
    /// Captures imported into the archive.
    pub imported: u64,
    /// Bytes they hold.
    pub bytes: u64,
    /// The archive already has this origin or these exact bytes.
    pub already_archived: u64,
    /// Kept out by privacy exclusions, or the source is disabled.
    pub excluded: u64,
    /// Sources the old app stored in a form that can't be turned back into raw files.
    pub unsupported: u64,
    /// Rebuilt bytes that don't match the recorded hash; not imported.
    pub mismatched: Vec<String>,
    pub dry_run: bool,
}

impl LegacyReport {
    pub fn summary(&self) -> String {
        format!(
            "{} {} of {} legacy captures ({} already archived, {} excluded, {} unsupported, {} unverifiable)",
            if self.dry_run {
                "Would import"
            } else {
                "Imported"
            },
            self.imported,
            self.seen,
            self.already_archived,
            self.excluded,
            self.unsupported,
            self.mismatched.len()
        )
    }
}

/// Marks captures this import wrote, so an interrupted import can pick up where it stopped.
const FINGERPRINT_PREFIX: &str = "legacy:";

/// The old app's store under the user's home.
pub fn default_db() -> Result<std::path::PathBuf> {
    Ok(crate::home::user_home()?.join(".distill/distill.db"))
}

struct LegacyCapture {
    id: i64,
    kind: String,
    path: Option<String>,
    sha: String,
}

pub fn import(home: &Home, config: &Config, db: &Path, dry_run: bool) -> Result<LegacyReport> {
    home.ensure()?;
    let conn = Connection::open_with_flags(db, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("open {}", db.display()))?;
    let captures: Vec<LegacyCapture> = conn
        .prepare(
            "SELECT c.id, s.kind, c.source_path, c.raw_sha256 FROM captures c \
             JOIN sources s ON s.id = c.source_id ORDER BY c.captured_at, c.id",
        )?
        .query_map([], |r| {
            Ok(LegacyCapture {
                id: r.get(0)?,
                kind: r.get(1)?,
                path: r.get(2)?,
                sha: r.get(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;

    let _lock = home.lock_archive()?;
    let mut archive = Archive::open(home)?;
    let known: HashSet<String> = archive.records().iter().map(|r| r.sha256.clone()).collect();
    let claude_on = config
        .sources
        .iter()
        .any(|s| s.kind == SourceKind::ClaudeCode && s.enabled);
    let mut report = LegacyReport {
        dry_run,
        ..Default::default()
    };
    for c in captures {
        report.seen += 1;
        // Codex and OpenCode captures were kept as summaries of files the sweep reads
        // directly; only Claude Code's line records rebuild the original bytes.
        let (Some(path), "claude_code") = (c.path.as_deref(), c.kind.as_str()) else {
            report.unsupported += 1;
            continue;
        };
        if !claude_on {
            report.excluded += 1;
            continue;
        }
        // An origin whose latest capture came from an import (this run or an interrupted
        // earlier one) takes the next legacy capture on top, so its chain lands as it did in
        // the old app. One the sweep has captured is left alone.
        let swept = archive
            .latest()
            .get(path)
            .is_some_and(|r| !r.fingerprint.starts_with(FINGERPRINT_PREFIX));
        if known.contains(&c.sha) || swept {
            report.already_archived += 1;
            continue;
        }
        let bytes = rebuild(&conn, c.id)?;
        if hex::encode(Sha256::digest(&bytes)) != c.sha {
            report.mismatched.push(path.to_string());
            continue;
        }
        if crate::sweep::bytes_excluded(config, SourceKind::ClaudeCode, Path::new(path), &bytes) {
            report.excluded += 1;
            continue;
        }
        report.imported += 1;
        report.bytes += bytes.len() as u64;
        if !dry_run {
            let fingerprint = format!("{FINGERPRINT_PREFIX}{}", c.sha);
            archive.put(SourceKind::ClaudeCode, path, &fingerprint, &bytes[..])?;
        }
    }
    if !dry_run {
        let mut entry = RunEntry::new("import", report.mismatched.is_empty(), report.summary());
        entry.details = serde_json::to_value(&report)?;
        runlog::record(home, &entry)?;
    }
    Ok(report)
}

/// A capture's file, from its line records in order.
fn rebuild(conn: &Connection, capture: i64) -> Result<Vec<u8>> {
    let mut stmt = conn.prepare(
        "SELECT content_json FROM capture_records WHERE capture_id = ? ORDER BY line_no",
    )?;
    let mut out = Vec::new();
    for line in stmt.query_map([capture], |r| r.get::<_, String>(0))? {
        out.extend_from_slice(line?.as_bytes());
        out.push(b'\n');
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn legacy_db(path: &Path, lines: &[&str], recorded_sha: Option<&str>) -> String {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sources (id INTEGER PRIMARY KEY, kind TEXT NOT NULL);
             CREATE TABLE captures (id INTEGER PRIMARY KEY, source_id INTEGER, source_path TEXT,
               raw_sha256 TEXT NOT NULL, captured_at TEXT NOT NULL);
             CREATE TABLE capture_records (id INTEGER PRIMARY KEY, capture_id INTEGER,
               line_no INTEGER, content_json TEXT NOT NULL);
             INSERT INTO sources VALUES (1, 'claude_code'), (2, 'opencode');",
        )
        .unwrap();
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        let sha = recorded_sha
            .map(str::to_string)
            .unwrap_or_else(|| hex::encode(Sha256::digest(body.as_bytes())));
        conn.execute(
            "INSERT INTO captures VALUES (1, 1, '/u/.claude/projects/-w-app/s1.jsonl', ?, '2026-03-01')",
            [&sha],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO captures VALUES (2, 2, '/u/oc/s2', 'x', '2026-03-01')",
            [],
        )
        .unwrap();
        for (i, l) in lines.iter().enumerate() {
            conn.execute(
                "INSERT INTO capture_records (capture_id, line_no, content_json) VALUES (1, ?, ?)",
                rusqlite::params![i as i64 + 1, l],
            )
            .unwrap();
        }
        sha
    }

    fn config(exclude: &[&str]) -> Config {
        let mut c = Config::default();
        c.privacy.exclude_paths = exclude.iter().map(|s| s.to_string()).collect();
        c
    }

    const LINES: &[&str] = &[
        r#"{"type":"user","cwd":"/w/app","sessionId":"s1","message":{"role":"user","content":"hi"}}"#,
        r#"{"type":"assistant","message":{"role":"assistant","content":"hello"}}"#,
    ];

    #[test]
    fn rebuilds_exact_bytes_once_and_skips_what_it_cannot_verify() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        let db = dir.path().join("legacy.db");
        let sha = legacy_db(&db, LINES, None);

        let dry = import(&home, &config(&[]), &db, true).unwrap();
        assert_eq!((dry.imported, dry.unsupported), (1, 1));
        assert!(Archive::open(&home).unwrap().records().is_empty());

        let r = import(&home, &config(&[]), &db, false).unwrap();
        assert_eq!(r.imported, 1);
        let archive = Archive::open(&home).unwrap();
        let rec = &archive.latest()["/u/.claude/projects/-w-app/s1.jsonl"];
        assert_eq!(rec.sha256, sha);
        assert_eq!(rec.source, SourceKind::ClaudeCode);

        // Running it again changes nothing.
        let again = import(&home, &config(&[]), &db, false).unwrap();
        assert_eq!((again.imported, again.already_archived), (0, 1));
    }

    #[test]
    fn an_interrupted_import_resumes_the_rest_of_a_chain() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        let db = dir.path().join("legacy.db");
        legacy_db(&db, &LINES[..1], None);
        // The old app captured the session again after it grew.
        let grown: String = LINES.iter().map(|l| format!("{l}\n")).collect();
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "INSERT INTO captures VALUES (3, 1, '/u/.claude/projects/-w-app/s1.jsonl', ?, '2026-03-02')",
            [hex::encode(Sha256::digest(grown.as_bytes()))],
        )
        .unwrap();
        for (i, l) in LINES.iter().enumerate() {
            conn.execute(
                "INSERT INTO capture_records (capture_id, line_no, content_json) VALUES (3, ?, ?)",
                rusqlite::params![i as i64 + 1, l],
            )
            .unwrap();
        }
        // Simulate a run that stopped after the first snapshot.
        conn.execute("UPDATE captures SET source_path = NULL WHERE id = 3", [])
            .unwrap();
        assert_eq!(import(&home, &config(&[]), &db, false).unwrap().imported, 1);
        conn.execute(
            "UPDATE captures SET source_path = '/u/.claude/projects/-w-app/s1.jsonl' WHERE id = 3",
            [],
        )
        .unwrap();

        let r = import(&home, &config(&[]), &db, false).unwrap();
        assert_eq!((r.imported, r.already_archived), (1, 1));
        let archive = Archive::open(&home).unwrap();
        let rec = &archive.latest()["/u/.claude/projects/-w-app/s1.jsonl"];
        assert_eq!(rec.size, grown.len() as u64);
        assert_eq!(rec.kind, crate::archive::CaptureKind::Append);
    }

    #[test]
    fn leaves_origins_the_sweep_captured_alone() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        let db = dir.path().join("legacy.db");
        legacy_db(&db, LINES, None);
        {
            home.ensure().unwrap();
            let _lock = home.lock_archive().unwrap();
            let mut archive = Archive::open(&home).unwrap();
            archive
                .put(
                    SourceKind::ClaudeCode,
                    "/u/.claude/projects/-w-app/s1.jsonl",
                    "size:mtime",
                    &b"newer bytes from disk\n"[..],
                )
                .unwrap();
        }
        let r = import(&home, &config(&[]), &db, false).unwrap();
        assert_eq!((r.imported, r.already_archived), (0, 1));
    }

    #[test]
    fn honors_exclusions_and_recorded_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path().join("home"));
        let db = dir.path().join("legacy.db");
        legacy_db(&db, LINES, None);
        let r = import(&home, &config(&["/w/app"]), &db, false).unwrap();
        assert_eq!((r.imported, r.excluded), (0, 1));
        // A session that says nothing about where it ran, from outside the source's roots,
        // is kept out while any exclusion is set.
        let db_nocwd = dir.path().join("nocwd.db");
        legacy_db(
            &db_nocwd,
            &[r#"{"type":"user","message":{"role":"user","content":"hi"}}"#],
            None,
        );
        let r = import(&home, &config(&["/elsewhere"]), &db_nocwd, false).unwrap();
        assert_eq!((r.imported, r.excluded), (0, 1));
        // With Claude Code turned off nothing is imported.
        let mut off = config(&[]);
        for s in &mut off.sources {
            s.enabled = s.kind != SourceKind::ClaudeCode;
        }
        let r = import(&home, &off, &db, false).unwrap();
        assert_eq!((r.imported, r.excluded), (0, 1));

        let db2 = dir.path().join("bad.db");
        legacy_db(&db2, LINES, Some(&"0".repeat(64)));
        let r = import(&home, &config(&[]), &db2, false).unwrap();
        assert_eq!(r.imported, 0);
        assert_eq!(r.mismatched.len(), 1);
    }
}
