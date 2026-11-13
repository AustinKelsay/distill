//! Turn raw captures into `meta.json` and `events.jsonl`. Everything written here can be deleted
//! and rebuilt from the archive.

pub mod blocks;
pub mod claude;
pub mod codex;
pub mod droid;
pub mod opencode;
pub mod pi;

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use anyhow::{Result, bail};
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
        SourceKind::Cursor => None,
    }
}

pub fn parse(kind: SourceKind, origin: &str, bytes: &[u8]) -> Result<Parsed> {
    let parsed = match kind {
        SourceKind::ClaudeCode => claude::parse(bytes)?,
        SourceKind::Codex => codex::parse(bytes)?,
        SourceKind::OpenCode => opencode::parse(bytes)?,
        SourceKind::Droid => droid::parse(bytes)?,
        SourceKind::Pi => pi::parse(bytes)?,
        SourceKind::Cursor => bail!("no parser for {kind}"),
    };
    let _ = origin;
    Ok(parsed)
}

/// Per-origin result of the last normalize, so unchanged captures are skipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizeState {
    pub source: SourceKind,
    pub sha256: String,
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The capture parsed but holds no conversation (a header-only file).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub empty: bool,
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
    pub sessions: u64,
}

pub fn normalize(home: &Home, force: bool) -> Result<NormalizeReport> {
    home.ensure()?;
    let archive = Archive::open(home)?;
    let mut state = load_state(home);
    let mut report = NormalizeReport::default();
    let mut git_cache = crate::git::Cache::default();

    for (origin, rec) in archive.latest() {
        let Some(version) = parser_version(rec.source) else {
            report.no_parser += 1;
            continue;
        };
        if !force
            && let Some(s) = state.get(origin)
            && s.sha256 == rec.sha256
            && s.version == version
        {
            report.skipped += 1;
            continue;
        }
        let result = normalize_one(home, &archive, rec, version, &mut git_cache);
        let mut entry = NormalizeState {
            source: rec.source,
            sha256: rec.sha256.clone(),
            version,
            session: None,
            error: None,
            empty: false,
        };
        match result {
            Ok(Some(meta)) => {
                report.normalized += 1;
                entry.session = Some(meta.session_id);
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
            "Normalized {} sessions ({} unchanged, {} empty, {} failed, {} without a parser)",
            report.normalized, report.skipped, report.empty, report.failed, report.no_parser
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
    let parsed = parse(rec.source, &rec.origin, &bytes)?;
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
        (Some(cwd), Some(start)) if !steward => {
            git_cache.outcome(cwd, start, ended_at.as_deref().unwrap_or(start))
        }
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

fn fallback_id(origin: &str) -> Option<String> {
    let name = origin.rsplit('/').next()?;
    Some(name.trim_end_matches(".jsonl").to_string())
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
