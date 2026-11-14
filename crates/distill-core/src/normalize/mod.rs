//! Turn raw captures into `meta.json` and `events.jsonl`. Everything written here can be deleted
//! and rebuilt from the archive.

pub mod antigravity;
pub mod blocks;
pub mod chat;
pub mod claude;
pub mod codex;
pub mod cursor;
pub mod droid;
pub mod goose;
pub mod grok;
pub mod hermes;
pub mod opencode;
pub mod pi;
pub mod t3code;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::archive::{Archive, CaptureRecord};
use crate::home::Home;
use crate::model::{Counts, Event, Parsed, Producer, RawRef, SessionMeta};
use crate::runlog::{self, RunEntry};
use crate::source::SourceKind;
use crate::util::{read_jsonl, safe_component, write_atomic, write_json_pretty};

/// Bump a source's version when its parser's output changes; sessions are re-normalized.
pub fn parser_version(kind: SourceKind) -> Option<u32> {
    match kind {
        SourceKind::ClaudeCode => Some(1),
        SourceKind::Codex => Some(1),
        SourceKind::OpenCode => Some(1),
        SourceKind::Droid => Some(1),
        SourceKind::Pi => Some(1),
        SourceKind::Grok => Some(1),
        SourceKind::Antigravity => Some(1),
        SourceKind::Hermes => Some(1),
        SourceKind::T3Code => Some(7),
        SourceKind::Cursor => Some(4),
        SourceKind::Goose => Some(1),
    }
}

pub fn parse(kind: SourceKind, origin: &str, bytes: &[u8]) -> Result<Parsed> {
    let parsed = match kind {
        SourceKind::ClaudeCode => claude::parse(bytes)?,
        // Codex compresses older sessions; the archive keeps the compressed bytes as they were.
        SourceKind::Codex if origin.ends_with(".zst") => {
            codex::parse(&zstd::stream::decode_all(bytes).context("decompress zstd")?)?
        }
        SourceKind::Codex => codex::parse(bytes)?,
        SourceKind::OpenCode => opencode::parse(bytes)?,
        SourceKind::Droid => droid::parse(bytes)?,
        SourceKind::Pi => pi::parse(bytes)?,
        SourceKind::Grok => grok::parse(origin, bytes)?,
        SourceKind::Antigravity => antigravity::parse(origin, bytes)?,
        SourceKind::Hermes => hermes::parse(origin, bytes)?,
        SourceKind::T3Code => t3code::parse(bytes)?,
        SourceKind::Goose => goose::parse(bytes)?,
        SourceKind::Cursor => cursor::parse(origin, bytes)?,
    };
    Ok(parsed)
}

/// Captures larger than this that yield no events are treated as format drift.
const DRIFT_BYTES: u64 = 16 * 1024;

/// Whether a capture is too big to hold nothing. Not for Cursor: its chats are SQLite files
/// whose size is mostly empty pages, and its parser fails outright when the store's layout is
/// not what it expects.
fn drift_suspect(rec: &CaptureRecord) -> bool {
    rec.size > DRIFT_BYTES && rec.source != SourceKind::Cursor
}

/// Bump when what normalize derives beyond the parsers (git outcomes, flags) changes.
pub const META_VERSION: u32 = 3;

/// Per-origin result of the last normalize, so unchanged captures are skipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizeState {
    pub source: SourceKind,
    pub sha256: String,
    pub version: u32,
    #[serde(default = "first_meta_version")]
    pub meta_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The capture parsed but holds no conversation (a header-only file).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub empty: bool,
    /// For a T3 thread, the native references capture it was normalized with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refs: Option<String>,
}

fn first_meta_version() -> u32 {
    1
}

fn state_path(home: &Home) -> PathBuf {
    home.state_dir().join("normalize.json")
}

fn load_state(home: &Home) -> BTreeMap<String, NormalizeState> {
    std::fs::read_to_string(state_path(home))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

pub fn session_dir(home: &Home, meta: &SessionMeta) -> PathBuf {
    session_dir_for(home, meta.source, &meta.session_id)
}

pub fn session_dir_for(home: &Home, source: SourceKind, session_id: &str) -> PathBuf {
    home.sessions_dir()
        .join(source.id())
        .join(safe_component(session_id))
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NormalizeReport {
    pub normalized: u64,
    pub skipped: u64,
    pub failed: u64,
    pub no_parser: u64,
    /// Captures with no conversation in them; not failures.
    pub empty: u64,
    /// Archived files that belong to a session but aren't one (attachments, tool output).
    #[serde(default)]
    pub sidecars: u64,
    /// Older captures of a Codex file that was since compressed or moved.
    #[serde(default)]
    pub superseded: u64,
    /// Of the failures, captures that parsed cleanly to nothing (see [`DRIFT_BYTES`]).
    #[serde(default)]
    pub drift: u64,
    pub sessions: u64,
}

pub fn normalize(home: &Home, force: bool) -> Result<NormalizeReport> {
    home.ensure()?;
    // Two runs at once (the timer and a manual run) would each read the state, rewrite
    // sessions from it and save their own copy, losing the other's work.
    let _lock = home.lock("normalize")?;
    let archive = Archive::open(home)?;
    let mut state = load_state(home);
    let mut report = NormalizeReport::default();
    let mut git_cache = crate::git::Cache::default();

    // Codex compresses or moves a session file (`x.jsonl` → `archived_sessions/x.jsonl.zst`).
    // Every capture stays in the archive, but only the newest one of a file is the session, so
    // incremental runs and full rebuilds agree.
    // Ranked by when each origin's current bytes first entered the manifest: append order,
    // not timestamps (one-second resolution) and not fingerprint refreshes (a touched file).
    let mut first_seen: std::collections::HashMap<(&str, &str), usize> = Default::default();
    for (idx, r) in archive.records().iter().enumerate() {
        first_seen
            .entry((r.origin.as_str(), r.sha256.as_str()))
            .or_insert(idx);
    }
    let mut groups: std::collections::HashMap<String, Vec<(usize, &str)>> = Default::default();
    for (origin, rec) in archive.latest() {
        if rec.source == SourceKind::Codex && rec.source.is_primary(origin) {
            // Old bytes coming back rank where they first arrived, not where they returned.
            let at = first_seen
                .get(&(origin.as_str(), rec.sha256.as_str()))
                .copied()
                .unwrap_or(0);
            groups
                .entry(codex_file(origin))
                .or_default()
                .push((at, origin.as_str()));
        }
    }
    let newest = |origin: &str| -> Option<(&str, bool)> {
        let g = groups.get(&codex_file(origin))?;
        let winner = g.iter().max_by_key(|(at, _)| *at)?.1;
        Some((winner, g.len() > 1))
    };

    for (origin, rec) in archive.latest() {
        // Sidecars (tool-result files, attachments, settings, databases) stay in the archive
        // but are not sessions on their own.
        if !rec.source.is_primary(origin) {
            report.sidecars += 1;
            continue;
        }
        let alias = if rec.source == SourceKind::Codex {
            newest(origin)
        } else {
            None
        };
        if let Some((winner, _)) = alias
            && winner != origin
        {
            // Forget it, so it is parsed again if it ever becomes the newest. If an earlier run
            // gave it a copy of its own (an `<id>-<hash>` session), that copy goes too.
            if let Some(old) = state.remove(origin)
                && let Some(id) = old.session
                && id.ends_with(&format!(
                    "-{}",
                    &crate::util::sha256_hex(origin.as_bytes())[..8]
                ))
            {
                let _ = std::fs::remove_dir_all(session_dir_for(home, rec.source, &id));
            }
            report.superseded += 1;
            continue;
        }
        let Some(version) = parser_version(rec.source) else {
            report.no_parser += 1;
            continue;
        };
        let refs = (rec.source == SourceKind::T3Code)
            .then(|| {
                archive
                    .latest()
                    .get(&format!("{origin}{}", t3code::NATIVE_SUFFIX))
            })
            .flatten()
            .map(|r| r.sha256.clone());
        if !force
            && let Some(s) = state.get(origin)
            && s.sha256 == rec.sha256
            && s.refs == refs
            && s.version == version
            && s.meta_version == META_VERSION
            // An empty result recorded before drift was checked gets the check now.
            && !(s.empty && drift_suspect(rec))
            // When several captures share a session, skip only if this one wrote it, under the
            // session's own id rather than a collision suffix from before they were grouped.
            && (!alias.is_some_and(|(_, shared)| shared)
                || s.session.as_deref().is_some_and(|id| {
                    !id.ends_with(&format!("-{}", &crate::util::sha256_hex(origin.as_bytes())[..8]))
                        && find(home, rec.source, id)
                            .ok()
                            .flatten()
                            .is_some_and(|m| m.raw.origin == *origin)
                }))
        {
            report.skipped += 1;
            continue;
        }
        let previous = state.get(origin).and_then(|s| s.session.clone());
        let result = normalize_one(home, &archive, rec, version, &mut git_cache);
        let mut entry = NormalizeState {
            source: rec.source,
            sha256: rec.sha256.clone(),
            version,
            meta_version: META_VERSION,
            session: None,
            error: None,
            empty: false,
            refs,
        };
        match result {
            Ok(Some(meta)) => {
                report.normalized += 1;
                // This capture now lives under another id: its old folder is gone from the index.
                if let Some(old) = previous
                    && old != meta.session_id
                    && find(home, rec.source, &old)
                        .ok()
                        .flatten()
                        .is_some_and(|m| m.raw.origin == *origin)
                {
                    let _ = std::fs::remove_dir_all(session_dir_for(home, rec.source, &old));
                }
                entry.session = Some(meta.session_id);
            }
            // A sizable capture that parses to nothing almost always means the harness changed
            // its format and the parser now skips everything. Report it as a failure, so it
            // shows on the dashboard and in repair briefs, rather than as an empty session.
            Ok(None) if drift_suspect(rec) => {
                report.failed += 1;
                report.drift += 1;
                entry.error = Some(format!(
                    "{} bytes parsed to no events; the format may have changed",
                    rec.size
                ));
            }
            Ok(None) => {
                report.empty += 1;
                entry.empty = true;
            }
            Err(e) => {
                report.failed += 1;
                entry.error = Some(format!("{e:#}"));
            }
        }
        state.insert(origin.clone(), entry);
    }
    write_json_pretty(&state_path(home), &state)?;
    report.sessions = rebuild_index(home)? as u64;

    let ok = report.failed == 0;
    let mut entry = RunEntry::new(
        "normalize",
        ok,
        format!(
            "Normalized {} sessions ({} unchanged, {} empty, {} failed, {} without a parser){}",
            report.normalized,
            report.skipped,
            report.empty,
            report.failed,
            report.no_parser,
            if report.drift > 0 {
                format!("; {} look like format drift", report.drift)
            } else {
                String::new()
            }
        ),
    );
    entry.details = serde_json::to_value(&report)?;
    runlog::record(home, &entry)?;
    Ok(report)
}

fn normalize_one(
    home: &Home,
    archive: &Archive,
    rec: &CaptureRecord,
    version: u32,
    git_cache: &mut crate::git::Cache,
) -> Result<Option<SessionMeta>> {
    let bytes = archive.read_blob(&rec.sha256)?;
    let mut parsed = parse(rec.source, &rec.origin, &bytes)?;
    // A T3 mirror's originals are named in a capture beside it. Runs that name none (and
    // threads captured before references were kept) fall back to their sources.
    if rec.source == SourceKind::T3Code && parsed.is_mirror() {
        let refs_origin = format!("{}{}", rec.origin, t3code::NATIVE_SUFFIX);
        let refs = match archive.latest().get(&refs_origin) {
            Some(r) => t3code::native_refs(&archive.read_blob(&r.sha256)?),
            None => Default::default(),
        };
        parsed.mirror_unresolved = if refs.keys.is_empty() && refs.unresolved.is_empty() {
            parsed.mirror_of.clone()
        } else {
            refs.unresolved
        };
        parsed.mirror_refs = refs.keys;
    }
    if parsed.events.is_empty() {
        return Ok(None);
    }
    let mut session_id = parsed
        .session_id
        .clone()
        .or_else(|| fallback_id(&rec.origin))
        .unwrap_or_else(|| rec.sha256[..16].to_string());
    // Two captures can claim one session id (a copied or forked file). Keep both.
    if let Ok(Some(existing)) = find(home, rec.source, &session_id)
        && existing.raw.origin != rec.origin
        && !(rec.source == SourceKind::Codex
            && codex_file(&existing.raw.origin) == codex_file(&rec.origin))
    {
        session_id = format!(
            "{session_id}-{}",
            &crate::util::sha256_hex(rec.origin.as_bytes())[..8]
        );
    }
    let events = parsed.events;
    let started_at = events.iter().find_map(|e| e.ts.clone());
    let ended_at = events.iter().rev().find_map(|e| e.ts.clone());
    let steward = parsed.cwd.as_deref().is_some_and(|c| home.is_inside(c));
    let git = match (&parsed.cwd, &started_at) {
        (Some(cwd), Some(start)) if !steward => git_cache.outcome(
            cwd,
            parsed.git_branch.as_deref(),
            start,
            ended_at.as_deref().unwrap_or(start),
        ),
        _ => None,
    };
    let meta = SessionMeta {
        source: rec.source,
        session_id,
        title: parsed.title.or_else(|| first_user_line(&events)),
        cwd: parsed.cwd,
        git_branch: parsed.git_branch,
        models: parsed.models,
        harness_version: parsed.harness_version,
        started_at,
        ended_at,
        parent_session: parsed.parent_session,
        counts: Counts::of(&events),
        raw: RawRef {
            origin: rec.origin.clone(),
            sha256: rec.sha256.clone(),
            size: rec.size,
        },
        normalizer: Producer {
            name: rec.source.id().to_string(),
            version,
        },
        steward,
        mirror: !parsed.mirror_of.is_empty(),
        mirror_of: parsed.mirror_of,
        mirror_refs: parsed.mirror_refs,
        mirror_unresolved: parsed.mirror_unresolved,
        git,
    };
    let dir = session_dir(home, &meta);
    let mut lines = Vec::new();
    for e in &events {
        serde_json::to_writer(&mut lines, e)?;
        lines.push(b'\n');
    }
    write_atomic(&dir.join("events.jsonl"), &lines)?;
    write_json_pretty(&dir.join("meta.json"), &meta)?;
    Ok(Some(meta))
}

/// A Codex session file's name without the compression suffix: the same file whether it is
/// live under `sessions/` or compressed under `archived_sessions/`.
fn codex_file(origin: &str) -> String {
    origin
        .rsplit('/')
        .next()
        .unwrap_or(origin)
        .trim_end_matches(".zst")
        .to_string()
}

fn fallback_id(origin: &str) -> Option<String> {
    let name = origin.rsplit('/').next()?;
    Some(
        name.trim_end_matches(".zst")
            .trim_end_matches(".jsonl")
            .to_string(),
    )
}

fn first_user_line(events: &[Event]) -> Option<String> {
    let text = events
        .iter()
        .find(|e| e.kind == crate::model::EventKind::User)?
        .text
        .as_deref()?;
    let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
    Some(line.chars().take(100).collect())
}

/// Rewrite `sessions/index.jsonl` from every `meta.json` on disk. Returns the session count.
pub fn rebuild_index(home: &Home) -> Result<usize> {
    let mut metas = Vec::new();
    for kind in SourceKind::ALL {
        let dir = home.sessions_dir().join(kind.id());
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in rd.filter_map(Result::ok) {
            let p = entry.path().join("meta.json");
            if let Ok(text) = std::fs::read_to_string(&p)
                && let Ok(m) = serde_json::from_str::<SessionMeta>(&text)
            {
                // A folder left under an older naming of its session is stale; the session
                // lives in its current folder. Derived data, so it is removed.
                if entry.file_name().to_str() != Some(safe_component(&m.session_id).as_str()) {
                    // Keep a steward card the session already paid for: the old folder goes only
                    // once it has no card or its card is in the current folder. Until then it
                    // stays on disk, out of the index.
                    let current = session_dir(home, &m);
                    let card = entry.path().join("card.json");
                    let card_safe = !card.exists()
                        || current.join("card.json").exists()
                        || (current.join("meta.json").exists()
                            && std::fs::rename(&card, current.join("card.json")).is_ok());
                    if card_safe {
                        let _ = std::fs::remove_dir_all(entry.path());
                    }
                    continue;
                }
                metas.push(m);
            }
        }
    }
    metas.sort_by(|a, b| b.started_at.cmp(&a.started_at).then(a.key().cmp(&b.key())));
    let mut out = Vec::new();
    for m in &metas {
        serde_json::to_writer(&mut out, m)?;
        out.push(b'\n');
    }
    write_atomic(&home.index_path(), &out)?;
    Ok(metas.len())
}

/// Every normalized session, newest first.
pub fn read_index(home: &Home) -> Result<Vec<SessionMeta>> {
    read_jsonl(&home.index_path())
}

pub fn read_events(home: &Home, meta: &SessionMeta) -> Result<Vec<Event>> {
    read_jsonl(&session_dir(home, meta).join("events.jsonl"))
}

/// Failed captures per source, from the last normalize.
pub fn failures(home: &Home) -> Result<HashMap<SourceKind, u64>> {
    let mut out = HashMap::new();
    for s in load_state(home).values() {
        if s.error.is_some() {
            *out.entry(s.source).or_default() += 1;
        }
    }
    Ok(out)
}

/// Failed captures with their errors, for parser repair.
pub fn failure_list(home: &Home, source: SourceKind) -> Vec<(String, String)> {
    load_state(home)
        .into_iter()
        .filter(|(_, s)| s.source == source)
        .filter_map(|(origin, s)| s.error.map(|e| (origin, e)))
        .collect()
}

/// Read one session by source and id.
pub fn find(home: &Home, source: SourceKind, session_id: &str) -> Result<Option<SessionMeta>> {
    let p = session_dir_for(home, source, session_id).join("meta.json");
    match std::fs::read_to_string(p) {
        Ok(t) => Ok(Some(serde_json::from_str(&t)?)),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_large_capture_that_parses_to_nothing_is_reported_as_drift() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        // Records in a shape the parser doesn't know: what a format change looks like.
        let line = r#"{"type":"turn.v2","payload":{"role":"user","text":"hello there"}}"#;
        let big: String = std::iter::repeat_n(format!("{line}\n"), 400).collect();
        let small = format!("{line}\n");
        {
            let _lock = home.lock_archive().unwrap();
            let mut archive = Archive::open(&home).unwrap();
            for (name, body) in [("big", &big), ("small", &small)] {
                let origin = format!("/u/.claude/projects/-w/{name}.jsonl");
                archive
                    .put(SourceKind::ClaudeCode, &origin, "f", body.as_bytes())
                    .unwrap();
            }
        }
        // A run from before drift was checked recorded the big capture as empty.
        let archive = Archive::open(&home).unwrap();
        let mut state = BTreeMap::new();
        for (origin, rec) in archive.latest() {
            state.insert(
                origin.clone(),
                NormalizeState {
                    source: rec.source,
                    sha256: rec.sha256.clone(),
                    version: parser_version(rec.source).unwrap(),
                    meta_version: META_VERSION,
                    session: None,
                    error: None,
                    empty: true,
                    refs: None,
                },
            );
        }
        write_json_pretty(&state_path(&home), &state).unwrap();

        let r = normalize(&home, false).unwrap();
        // The small one stays cached as empty; the big one is checked again and flagged.
        assert_eq!((r.failed, r.drift, r.skipped), (1, 1, 1));
        let fails = failure_list(&home, SourceKind::ClaudeCode);
        assert_eq!(fails.len(), 1);
        assert!(fails[0].0.ends_with("big.jsonl"));
        assert!(fails[0].1.contains("format may have changed"));
    }
}
