//! The parse step (`distill parse`): turn captures into events and session facts, written to
//! `events.jsonl` and `meta.json` per session and to the session list. Everything written here
//! can be deleted and rebuilt from the raw archive. The module keeps its earlier name.

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

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::archive::{Archive, CaptureRecord};
use crate::harness::Harness;
use crate::home::Home;
use crate::model::{Counts, Event, Parsed, Producer, RawRef, SessionMeta, Skipped};
use crate::util::{read_jsonl, safe_component, write_atomic, write_atomic_with, write_json_pretty};

/// Bump a harness's version when its parser's output changes; its sessions are parsed again.
pub fn parser_version(kind: Harness) -> Option<u32> {
    match kind {
        Harness::ClaudeCode => Some(1),
        // 2: agent messages, tool searches and image generations are events.
        Harness::Codex => Some(2),
        // 2: a spilled tool output's tool result holds the full output.
        Harness::OpenCode => Some(2),
        // 2: compaction summaries and plans are system events.
        Harness::Droid => Some(2),
        // 2: compactions, summary recaps and background terminal results are events.
        Harness::Pi => Some(2),
        Harness::Grok => Some(1),
        Harness::Antigravity => Some(1),
        Harness::Hermes => Some(4),
        // 8: answers, requests and plans are events.
        Harness::T3Code => Some(8),
        Harness::Cursor => Some(6),
        Harness::Goose => Some(2),
    }
}

/// The record types a harness's parser skips on purpose (each parser's `IGNORED`): not format
/// drift.
pub fn ignored(kind: Harness) -> &'static [&'static str] {
    match kind {
        Harness::ClaudeCode => claude::IGNORED,
        Harness::Codex => codex::IGNORED,
        Harness::OpenCode => opencode::IGNORED,
        Harness::Droid => droid::IGNORED,
        Harness::Pi => pi::IGNORED,
        Harness::Grok => grok::IGNORED,
        Harness::Antigravity => antigravity::IGNORED,
        Harness::Hermes => hermes::IGNORED,
        Harness::T3Code => t3code::IGNORED,
        Harness::Goose => goose::IGNORED,
        Harness::Cursor => cursor::IGNORED,
    }
}

/// Parse one capture. What it skipped (`Parsed::skipped`) leaves out the types its parser
/// ignores on purpose.
pub fn parse(kind: Harness, origin: &str, bytes: &[u8]) -> Result<Parsed> {
    if streams(kind) {
        return parse_stream(kind, origin, bytes);
    }
    let mut parsed = match kind {
        Harness::OpenCode => opencode::parse(bytes)?,
        Harness::Droid => droid::parse(bytes)?,
        Harness::Pi => pi::parse(bytes)?,
        Harness::Grok => grok::parse(origin, bytes)?,
        Harness::Antigravity => antigravity::parse(origin, bytes)?,
        Harness::Hermes => hermes::parse(origin, bytes)?,
        Harness::T3Code => t3code::parse(bytes)?,
        Harness::Goose => goose::parse(bytes)?,
        Harness::Cursor => cursor::parse(origin, bytes)?,
        Harness::ClaudeCode | Harness::Codex => unreachable!("streamed above"),
    };
    parsed
        .skipped
        .retain(|ty, _| !ignored(kind).contains(&ty.as_str()));
    Ok(parsed)
}

/// Whether a harness's parser reads a capture as a stream (#105), so a capture is never held
/// whole: the line-based ones that meet giant files.
pub fn streams(kind: Harness) -> bool {
    matches!(kind, Harness::ClaudeCode | Harness::Codex)
}

/// [`parse`] for a harness that [`streams`], over a reader. Codex compresses older sessions,
/// and the archive keeps the compressed bytes as they were: they are decompressed as they are
/// read.
pub fn parse_stream(kind: Harness, origin: &str, reader: impl std::io::Read) -> Result<Parsed> {
    let mut parsed = match kind {
        Harness::ClaudeCode => {
            claude::parse_reader(std::io::BufReader::with_capacity(1 << 16, reader))?
        }
        Harness::Codex if origin.ends_with(".zst") => {
            let decoder = zstd::stream::read::Decoder::new(reader).context("decompress zstd")?;
            codex::parse_reader(std::io::BufReader::with_capacity(1 << 16, decoder))
                .context("decompress zstd")?
        }
        Harness::Codex => codex::parse_reader(std::io::BufReader::with_capacity(1 << 16, reader))?,
        other => bail!("{} is not parsed as a stream", other.id()),
    };
    parsed
        .skipped
        .retain(|ty, _| !ignored(kind).contains(&ty.as_str()));
    Ok(parsed)
}

/// How a format-drift failure ends its error.
const DRIFT_MESSAGE: &str = "the format may have changed";

/// Captures larger than this that yield no events are treated as format drift.
const DRIFT_BYTES: u64 = 16 * 1024;

/// Whether a capture is too big to hold nothing. Only for files a harness writes itself: not
/// for Cursor (its chats are SQLite files whose size is mostly empty pages, and its parser fails
/// outright when the store's layout is not what it expects), and not for sessions Distill
/// exports from a database (`…#session=…`, `…#thread=…`), whose shape its own query fixes; a
/// schema change fails that query instead. A database session can legitimately hold nothing
/// but a long system prompt.
fn drift_suspect(rec: &CaptureRecord) -> bool {
    rec.size > DRIFT_BYTES && rec.harness != Harness::Cursor && !rec.origin.contains('#')
}

/// The session-facts version. Bump when what the parse step derives beyond the parsers changes;
/// every session is parsed again. 4: the title and the git outcome were removed. 5: the record
/// types each capture's parser skipped are kept (`NormalizeState::unknown`).
pub const META_VERSION: u32 = 5;

/// Per-origin result of the last normalize, so unchanged captures are skipped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizeState {
    #[serde(alias = "source")]
    pub harness: Harness,
    /// The capture hash it was parsed from.
    #[serde(alias = "sha256")]
    pub capture_hash: String,
    pub version: u32,
    #[serde(default = "first_meta_version")]
    pub meta_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The failure is format drift: a sizable capture that parsed to no events.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub drift: bool,
    /// The capture parsed but holds no conversation (a header-only file).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub empty: bool,
    /// For a T3 thread, the native references capture it was parsed with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refs: Option<String>,
    /// For an OpenCode session, the spilled tool outputs it names that the archive did not hold
    /// when it was parsed (file names only), so it is parsed again once one is captured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_spills: Vec<String>,
    /// Record types in the capture its parser doesn't know, by name only, with how many records
    /// and bytes each holds: format drift that drops some records while the rest still parse.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub unknown: BTreeMap<String, Skipped>,
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
    home.session_dir(meta.harness, &meta.session_id)
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

impl NormalizeReport {
    pub fn summary(&self) -> String {
        format!(
            "Parsed {} ({} unchanged, {} empty, {} failed, {} without a parser){}; {} total",
            crate::util::count(self.normalized, "session"),
            self.skipped,
            self.empty,
            self.failed,
            self.no_parser,
            if self.drift > 0 {
                format!("; {} look like format drift", self.drift)
            } else {
                String::new()
            },
            crate::util::count(self.sessions, "session"),
        )
    }
}

/// Parse every capture whose result is missing or out of date. The caller records the run.
pub fn normalize(home: &Home, force: bool) -> Result<NormalizeReport> {
    home.ensure()?;
    // Two runs at once (the timer and a manual run) would each read the state, rewrite
    // sessions from it and save their own copy, losing the other's work.
    let _lock = home.lock("normalize")?;
    let archive = Archive::open(home)?;
    let mut state = load_state(home);
    let mut report = NormalizeReport::default();

    // Codex compresses or moves a session file (`x.jsonl` → `archived_sessions/x.jsonl.zst`).
    // Every capture stays in the archive, but only the newest one of a file is the session, so
    // incremental runs and full rebuilds agree.
    // Ranked by when each origin's current bytes first entered the manifest: append order,
    // not timestamps (one-second resolution) and not change-signal refreshes (a touched file).
    let mut first_seen: std::collections::HashMap<(&str, &str), usize> = Default::default();
    for (idx, r) in archive.records().iter().enumerate() {
        first_seen
            .entry((r.origin.as_str(), r.capture_hash.as_str()))
            .or_insert(idx);
    }
    let mut groups: std::collections::HashMap<String, Vec<(usize, &str)>> = Default::default();
    for (origin, rec) in archive.latest() {
        if rec.harness == Harness::Codex && rec.harness.is_primary(origin) {
            // Old bytes coming back rank where they first arrived, not where they returned.
            let at = first_seen
                .get(&(origin.as_str(), rec.capture_hash.as_str()))
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
        if !rec.harness.is_primary(origin) {
            report.sidecars += 1;
            continue;
        }
        let alias = if rec.harness == Harness::Codex {
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
                let _ = std::fs::remove_dir_all(home.session_dir(rec.harness, &id));
            }
            report.superseded += 1;
            continue;
        }
        let Some(version) = parser_version(rec.harness) else {
            report.no_parser += 1;
            continue;
        };
        let refs = (rec.harness == Harness::T3Code)
            .then(|| {
                archive
                    .latest()
                    .get(&format!("{origin}{}", t3code::NATIVE_SUFFIX))
            })
            .flatten()
            .map(|r| r.capture_hash.clone());
        if !force
            && let Some(s) = state.get(origin)
            && s.capture_hash == rec.capture_hash
            && s.refs == refs
            && !s.missing_spills.iter().any(|name| {
                opencode::spill_origin(origin, name)
                    .is_some_and(|o| archive.latest().contains_key(&o))
            })
            && s.version == version
            && s.meta_version == META_VERSION
            // An empty result recorded before drift was checked gets the check now.
            && !(s.empty && drift_suspect(rec))
            // When several captures share a session, skip only if this one wrote it, under the
            // session's own id rather than a collision suffix from before they were grouped.
            && (!alias.is_some_and(|(_, shared)| shared)
                || s.session.as_deref().is_some_and(|id| {
                    !id.ends_with(&format!("-{}", &crate::util::sha256_hex(origin.as_bytes())[..8]))
                        && find(home, rec.harness, id)
                            .ok()
                            .flatten()
                            .is_some_and(|m| m.raw.origin == *origin)
                }))
        {
            report.skipped += 1;
            continue;
        }
        let previous = state.get(origin).and_then(|s| s.session.clone());
        let mut missing_spills = Vec::new();
        let result = normalize_one(home, &archive, rec, version, &mut missing_spills);
        let mut entry = NormalizeState {
            harness: rec.harness,
            capture_hash: rec.capture_hash.clone(),
            version,
            meta_version: META_VERSION,
            session: None,
            error: None,
            drift: false,
            empty: false,
            refs,
            missing_spills,
            unknown: BTreeMap::new(),
        };
        let result = result.map(|(meta, unknown)| {
            entry.unknown = unknown;
            meta
        });
        match result {
            Ok(Some(meta)) => {
                report.normalized += 1;
                // This capture now lives under another id: its old folder leaves the list.
                if let Some(old) = previous
                    && old != meta.session_id
                    && find(home, rec.harness, &old)
                        .ok()
                        .flatten()
                        .is_some_and(|m| m.raw.origin == *origin)
                {
                    let _ = std::fs::remove_dir_all(home.session_dir(rec.harness, &old));
                }
                entry.session = Some(meta.session_id);
            }
            // A sizable capture that parses to nothing almost always means the harness changed
            // its format and the parser now skips everything. Report it as a failure, so it
            // shows in status as format drift, rather than as an empty session.
            Ok(None) if drift_suspect(rec) => {
                report.failed += 1;
                report.drift += 1;
                entry.drift = true;
                entry.error = Some(format!(
                    "{} bytes parsed to no events; {DRIFT_MESSAGE}",
                    rec.size
                ));
            }
            Ok(None) => {
                report.empty += 1;
                entry.empty = true;
                // It held a session before (an earlier parser read more into it): that session
                // leaves the list now.
                if let Some(old) = previous
                    && find(home, rec.harness, &old)
                        .ok()
                        .flatten()
                        .is_some_and(|m| m.raw.origin == *origin)
                {
                    let _ = std::fs::remove_dir_all(home.session_dir(rec.harness, &old));
                }
            }
            Err(e) => {
                report.failed += 1;
                entry.error = Some(format!("{e:#}"));
            }
        }
        state.insert(origin.clone(), entry);
    }
    write_json_pretty(&state_path(home), &state)?;
    report.sessions = rebuild_session_list(home)? as u64;
    Ok(report)
}

/// Parse one capture into its session. For an OpenCode session, `missing_spills` gets the spilled
/// tool outputs it names that the archive doesn't hold.
fn normalize_one(
    home: &Home,
    archive: &Archive,
    rec: &CaptureRecord,
    version: u32,
    missing_spills: &mut Vec<String>,
) -> Result<(Option<SessionMeta>, BTreeMap<String, Skipped>)> {
    let mut attached = Vec::new();
    let mut parsed = if rec.harness == Harness::OpenCode {
        let bytes = archive.read_blob(&rec.capture_hash)?;
        // Spilled tool outputs are captures of their own, at their own origins. One the archive
        // can't read now counts as missing, so the session is parsed again until it can.
        let (parsed, missing) = opencode::parse_with_spills(&bytes, |name| {
            let Some(r) =
                opencode::spill_origin(&rec.origin, name).and_then(|o| archive.latest().get(&o))
            else {
                return Ok(None);
            };
            match archive.read_blob(&r.capture_hash) {
                Ok(b) => {
                    attached.push(r.capture_hash.clone());
                    Ok(Some(b))
                }
                Err(_) => Ok(None),
            }
        })?;
        *missing_spills = missing;
        parsed
    } else if streams(rec.harness) {
        // Read through the blob without holding it, hashing as it goes; a blob that does not
        // hash to its name fails the capture, as it would when read whole.
        let mut blob = archive.open_blob(&rec.capture_hash)?;
        let parsed = parse_stream(rec.harness, &rec.origin, &mut blob)?;
        blob.finish()?;
        parsed
    } else {
        parse(
            rec.harness,
            &rec.origin,
            &archive.read_blob(&rec.capture_hash)?,
        )?
    };
    attached.sort();
    attached.dedup();
    let unknown = std::mem::take(&mut parsed.skipped);
    // A T3 mirror's originals are named in a capture beside it. Runs that name none (and
    // threads captured before references were kept) fall back to their harnesses.
    if rec.harness == Harness::T3Code && parsed.is_mirror() {
        let refs_origin = format!("{}{}", rec.origin, t3code::NATIVE_SUFFIX);
        let refs = match archive.latest().get(&refs_origin) {
            Some(r) => t3code::native_refs(&archive.read_blob(&r.capture_hash)?),
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
        return Ok((None, unknown));
    }
    let mut session_id = parsed
        .session_id
        .clone()
        .or_else(|| fallback_id(&rec.origin))
        .unwrap_or_else(|| rec.capture_hash[..16].to_string());
    // Two captures can claim one session id (a copied or forked file). Keep both.
    if let Ok(Some(existing)) = find(home, rec.harness, &session_id)
        && existing.raw.origin != rec.origin
        && !(rec.harness == Harness::Codex
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
    let meta = SessionMeta {
        harness: rec.harness,
        session_id,
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
            sha256: rec.capture_hash.clone(),
            size: rec.size,
            attached,
        },
        normalizer: Producer {
            name: rec.harness.id().to_string(),
            version,
        },
        steward,
        mirror: !parsed.mirror_of.is_empty(),
        mirror_of: parsed.mirror_of,
        mirror_refs: parsed.mirror_refs,
        mirror_unresolved: parsed.mirror_unresolved,
    };
    write_atomic_with(&home.events_path(meta.harness, &meta.session_id), |w| {
        for e in &events {
            serde_json::to_writer(&mut *w, e)?;
            w.write_all(b"\n")?;
        }
        Ok(())
    })?;
    write_json_pretty(&home.meta_path(meta.harness, &meta.session_id), &meta)?;
    Ok((Some(meta), unknown))
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

/// Rewrite the session list (`sessions/index.jsonl`) from every `meta.json` on disk. Returns
/// the session count.
pub fn rebuild_session_list(home: &Home) -> Result<usize> {
    let mut metas = Vec::new();
    for kind in Harness::ALL {
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
                // lives in its current folder. Rebuildable, so it is removed.
                if entry.file_name().to_str() != Some(safe_component(&m.session_id).as_str()) {
                    let _ = std::fs::remove_dir_all(entry.path());
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
    write_atomic(&home.session_list_path(), &out)?;
    Ok(metas.len())
}

/// Every parsed session's facts, newest first.
pub fn read_session_list(home: &Home) -> Result<Vec<SessionMeta>> {
    read_jsonl(&home.session_list_path())
}

/// A session's events. Only the parse step, the detector and the read path read stored text
/// directly; everything else goes through the read path (`crate::read`).
pub(crate) fn read_events(home: &Home, meta: &SessionMeta) -> Result<Vec<Event>> {
    read_jsonl(&home.events_path(meta.harness, &meta.session_id))
}

/// Captures that failed to parse, per harness, from the last parse, and record types its
/// parser doesn't know.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Failures {
    pub failed: u64,
    /// Of those, the ones that look like format drift.
    pub drift: u64,
    /// Captures that parsed but hold record types the parser doesn't know.
    #[serde(default)]
    pub partial_drift: u64,
    /// Those types, in every capture, failed or not.
    #[serde(default)]
    pub unknown: BTreeMap<String, UnknownType>,
}

/// One record type a harness's parser doesn't know, across that harness's captures.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnknownType {
    /// Captures holding it.
    pub captures: u64,
    pub records: u64,
    pub bytes: u64,
}

pub fn failures(home: &Home) -> HashMap<Harness, Failures> {
    let mut out: HashMap<Harness, Failures> = HashMap::new();
    for s in load_state(home).values() {
        // A type a parser has since learned to ignore isn't drift, though its captures are
        // unchanged and not parsed again.
        let ignored = ignored(s.harness);
        let unknown: Vec<_> = s
            .unknown
            .iter()
            .filter(|(ty, _)| !ignored.contains(&ty.as_str()))
            .collect();
        if !unknown.is_empty() {
            let f = out.entry(s.harness).or_default();
            if s.error.is_none() {
                f.partial_drift += 1;
            }
            for (ty, n) in unknown {
                let t = f.unknown.entry(ty.clone()).or_default();
                t.captures += 1;
                t.records += n.records;
                t.bytes += n.bytes;
            }
        }
        if s.error.is_some() {
            let f = out.entry(s.harness).or_default();
            f.failed += 1;
            // State written before drift was flagged names it in the error.
            if s.drift
                || s.error
                    .as_deref()
                    .is_some_and(|e| e.contains(DRIFT_MESSAGE))
            {
                f.drift += 1;
            }
        }
    }
    out
}

/// Failed captures with their errors.
pub fn failure_list(home: &Home, harness: Harness) -> Vec<(String, String)> {
    load_state(home)
        .into_iter()
        .filter(|(_, s)| s.harness == harness)
        .filter_map(|(origin, s)| s.error.map(|e| (origin, e)))
        .collect()
}

/// Read one session's facts by harness and id.
pub fn find(home: &Home, harness: Harness, session_id: &str) -> Result<Option<SessionMeta>> {
    let p = home.meta_path(harness, session_id);
    match std::fs::read_to_string(p) {
        Ok(t) => Ok(Some(serde_json::from_str(&t)?)),
        Err(_) => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_facts_hold_no_session_text_and_replace_the_previous_builds() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        let session = r#"{"type":"custom-title","customTitle":"Rotate the widget key","sessionId":"s1"}
{"type":"user","cwd":"/w/app","gitBranch":"main","version":"2.1.0","sessionId":"s1","timestamp":"2026-11-01T00:00:00Z","message":{"role":"user","content":"please rotate the widget key"}}
{"type":"assistant","timestamp":"2026-11-01T00:00:05Z","message":{"role":"assistant","model":"m-1","content":[{"type":"text","text":"Rotated."}]}}
"#;
        let origin = "/u/.claude/projects/-w-app/s1.jsonl";
        {
            let _lock = home.lock_archive().unwrap();
            let mut archive = Archive::open(&home).unwrap();
            archive
                .put(Harness::ClaudeCode, origin, "f", session.as_bytes())
                .unwrap();
        }
        // What the previous build left: facts with a title and a git outcome, under an older
        // session-facts version.
        let meta_path = home.meta_path(Harness::ClaudeCode, "s1");
        std::fs::create_dir_all(meta_path.parent().unwrap()).unwrap();
        let rec = Archive::open(&home).unwrap().latest()[origin].clone();
        std::fs::write(
            &meta_path,
            serde_json::json!({"source": "claude-code", "session_id": "s1",
                "title": "please rotate the widget key", "cwd": "/w/app", "models": [],
                "counts": {"events": 2, "user": 1, "assistant": 1, "tool_calls": 0, "tool_errors": 0},
                "raw": {"origin": origin, "sha256": rec.capture_hash, "size": rec.size},
                "normalizer": {"name": "claude-code", "version": 1}, "steward": false,
                "git": {"repo": "/w/app", "commits": ["abc"], "reverted": 0}})
            .to_string(),
        )
        .unwrap();
        let mut state = BTreeMap::new();
        state.insert(
            origin.to_string(),
            serde_json::json!({"source": "claude-code", "sha256": rec.capture_hash,
                "version": 1, "meta_version": 3, "session": "s1"}),
        );
        write_json_pretty(&state_path(&home), &state).unwrap();

        let r = normalize(&home, false).unwrap();
        assert_eq!((r.normalized, r.sessions), (1, 1), "{r:?}");
        let text = std::fs::read_to_string(&meta_path).unwrap();
        let facts: serde_json::Value = serde_json::from_str(&text).unwrap();
        let keys: Vec<&str> = facts
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        for gone in ["title", "git", "source"] {
            assert!(!keys.contains(&gone), "{gone} in {text}");
        }
        assert_eq!(facts["harness"], "claude-code");
        assert_eq!(facts["cwd"], "/w/app");
        assert_eq!(facts["git_branch"], "main");
        assert_eq!(facts["models"], serde_json::json!(["m-1"]));
        for words in ["widget", "Rotated"] {
            assert!(!text.contains(words), "session text in the facts: {text}");
        }
        let list = std::fs::read_to_string(home.session_list_path()).unwrap();
        assert!(!list.contains("widget"), "{list}");
        // The events still hold the text.
        let events = read_events(
            &home,
            &find(&home, Harness::ClaudeCode, "s1").unwrap().unwrap(),
        );
        assert!(
            events
                .unwrap()
                .iter()
                .any(|e| e.text.as_deref() == Some("Rotated."))
        );
    }

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
                    .put(Harness::ClaudeCode, &origin, "f", body.as_bytes())
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
                    harness: rec.harness,
                    capture_hash: rec.capture_hash.clone(),
                    version: parser_version(rec.harness).unwrap(),
                    meta_version: META_VERSION,
                    session: None,
                    error: None,
                    drift: false,
                    empty: true,
                    refs: None,
                    missing_spills: Vec::new(),
                    unknown: BTreeMap::new(),
                },
            );
        }
        write_json_pretty(&state_path(&home), &state).unwrap();

        let r = normalize(&home, false).unwrap();
        // The small one stays cached as empty; the big one is checked again and flagged.
        assert_eq!((r.failed, r.drift, r.skipped), (1, 1, 1));
        let fails = failure_list(&home, Harness::ClaudeCode);
        assert_eq!(fails.len(), 1);
        assert!(fails[0].0.ends_with("big.jsonl"));
        assert!(fails[0].1.contains("format may have changed"));
        let by_harness = failures(&home);
        let f = &by_harness[&Harness::ClaudeCode];
        assert_eq!((f.failed, f.drift, f.partial_drift), (1, 1, 0));
        // The failed capture's records are of a type the parser doesn't know, named too.
        assert_eq!(f.unknown["turn.v2"].records, 400);
    }

    #[test]
    fn record_types_a_parser_does_not_know_are_counted_by_name_while_the_rest_parse() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        // One record of a type the parser doesn't know among ones it reads, one content block it
        // doesn't know, and a title record, which it ignores on purpose.
        let unknown = r#"{"type":"tool-output.v2","timestamp":"2026-11-01T00:00:03Z","text":"the widget key is rotated"}"#;
        let session = format!(
            r#"{{"type":"custom-title","customTitle":"Rotate the widget key","sessionId":"s1"}}
{{"type":"user","cwd":"/w","sessionId":"s1","timestamp":"2026-11-01T00:00:00Z","message":{{"role":"user","content":"please rotate the widget key"}}}}
{unknown}
{{"type":"assistant","timestamp":"2026-11-01T00:00:05Z","message":{{"role":"assistant","model":"m-1","content":[{{"type":"text","text":"Rotated."}},{{"type":"widget_card","body":"widget art"}}]}}}}
"#
        );
        let origin = "/u/.claude/projects/-w/s1.jsonl";
        {
            let _lock = home.lock_archive().unwrap();
            let mut archive = Archive::open(&home).unwrap();
            archive
                .put(Harness::ClaudeCode, origin, "f", session.as_bytes())
                .unwrap();
        }
        let r = normalize(&home, false).unwrap();
        assert_eq!((r.normalized, r.failed, r.drift), (1, 0, 0), "{r:?}");
        let meta = find(&home, Harness::ClaudeCode, "s1").unwrap().unwrap();
        let events = read_events(&home, &meta).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1].text.as_deref(), Some("Rotated."));

        let state = load_state(&home);
        let names: Vec<&str> = state[origin].unknown.keys().map(String::as_str).collect();
        assert_eq!(names, ["block:widget_card", "tool-output.v2"]);
        assert_eq!(
            state[origin].unknown["tool-output.v2"],
            Skipped {
                records: 1,
                bytes: unknown.len() as u64
            }
        );
        let f = &failures(&home)[&Harness::ClaudeCode];
        assert_eq!((f.failed, f.drift, f.partial_drift), (0, 0, 1));
        assert_eq!(f.unknown["tool-output.v2"].captures, 1);
        // The state holds the type names, never the records' text.
        let text = std::fs::read_to_string(state_path(&home)).unwrap();
        for words in [
            "widget key",
            "rotated",
            "widget art",
            "Rotate",
            "custom-title",
        ] {
            assert!(!text.contains(words), "{words} in {text}");
        }

        // Unchanged, it is not parsed again and keeps its tallies.
        let r = normalize(&home, false).unwrap();
        assert_eq!((r.normalized, r.skipped), (0, 1));
        assert_eq!(failures(&home)[&Harness::ClaudeCode].partial_drift, 1);
    }

    #[test]
    fn a_capture_parsed_before_types_were_counted_is_parsed_again() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        let session = r#"{"type":"user","cwd":"/w","sessionId":"s1","timestamp":"2026-11-01T00:00:00Z","message":{"role":"user","content":"hi"}}
{"type":"turn.v2","text":"hello"}
"#;
        let origin = "/u/.claude/projects/-w/s1.jsonl";
        {
            let _lock = home.lock_archive().unwrap();
            let mut archive = Archive::open(&home).unwrap();
            archive
                .put(Harness::ClaudeCode, origin, "f", session.as_bytes())
                .unwrap();
        }
        let rec = Archive::open(&home).unwrap().latest()[origin].clone();
        // What the previous build left: the session parsed, no tallies, an older facts version.
        let mut state = BTreeMap::new();
        state.insert(
            origin.to_string(),
            serde_json::json!({"harness": "claude-code", "capture_hash": rec.capture_hash,
                "version": 1, "meta_version": 4, "session": "s1"}),
        );
        write_json_pretty(&state_path(&home), &state).unwrap();
        assert!(failures(&home).is_empty());

        let r = normalize(&home, false).unwrap();
        assert_eq!(r.normalized, 1, "{r:?}");
        assert_eq!(failures(&home)[&Harness::ClaudeCode].partial_drift, 1);
    }

    #[test]
    fn nested_types_are_named_within_their_record_and_ignored_ones_are_left_out() {
        let src = r#"{"timestamp":"2026-11-03T08:30:53Z","type":"session_meta","payload":{"id":"abc","cwd":"/r"}}
{"timestamp":"2026-11-03T08:30:54Z","type":"event_msg","payload":{"type":"user_message","message":"list files"}}
{"timestamp":"2026-11-03T08:30:55Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"list files"}]}}
{"timestamp":"2026-11-03T08:30:56Z","type":"response_item","payload":{"type":"tool_output_v2","call_id":"c1","output":"a.txt"}}
{"timestamp":"2026-11-03T08:30:57Z","type":"turn_summary","payload":{"text":"listed"}}"#;
        let p = parse(Harness::Codex, "/u/.codex/sessions/r.jsonl", src.as_bytes()).unwrap();
        assert_eq!(p.events.len(), 1);
        let names: Vec<&str> = p.skipped.keys().map(String::as_str).collect();
        assert_eq!(names, ["response_item:tool_output_v2", "turn_summary"]);
    }

    #[test]
    fn type_names_keep_only_identifier_characters_and_are_capped() {
        use crate::model::type_name;
        assert_eq!(type_name("tool_result.v2:x-y"), "tool_result.v2:x-y");
        assert_eq!(type_name("a b/c\n\"d\"\u{e9}"), "abcd");
        assert_eq!(type_name(&"x".repeat(200)).len(), 64);
        let mut p = Parsed::default();
        p.skip(" / ", 10);
        assert!(p.skipped.is_empty());
    }

    const CODEX: &str = "{\"timestamp\":\"2026-11-03T08:30:53Z\",\"type\":\"session_meta\",\"payload\":{\"id\":\"abc\",\"cwd\":\"/r\"}}\r\n\
{\"type\":\"compacted\",\"payload\":{\"replacement_history\":[]}}\n\
{\"type\":\"event_msg\"}\n\
{\"type\":\"mystery\",\"payload\":{}}\n\
{\"timestamp\":\"2026-11-03T08:30:55Z\",\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"user\",\"content\":[{\"type\":\"input_text\",\"text\":\"caf\u{e9}\"}]}}\n\
not json\n\
{\"type\":\"response_item\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"content\":[{\"type\":\"output_text\",\"text\":\"done\"}]}}";

    /// One byte per read: nothing but the stream's own framing can hold a line together.
    struct Trickle<'a>(&'a [u8]);
    impl std::io::Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.0.len().min(1).min(buf.len());
            buf[..n].copy_from_slice(&self.0[..n]);
            self.0 = &self.0[n..];
            Ok(n)
        }
    }

    #[test]
    fn a_streamed_parse_equals_the_parse_of_the_whole_capture() {
        let whole = parse(Harness::Codex, "r.jsonl", CODEX.as_bytes()).unwrap();
        let streamed = parse_stream(Harness::Codex, "r.jsonl", Trickle(CODEX.as_bytes())).unwrap();
        let zst = zstd::stream::encode_all(CODEX.as_bytes(), 1).unwrap();
        let from_zst = parse_stream(Harness::Codex, "r.jsonl.zst", Trickle(&zst)).unwrap();
        let json =
            |p: &Parsed| serde_json::to_string(&(&p.events, &p.skipped, &p.session_id)).unwrap();
        assert_eq!(whole.events.len(), 2);
        // What the fixture must parse to, written out, so the comparisons below don't only
        // check the streaming path against itself.
        let said: Vec<_> = whole
            .events
            .iter()
            .map(|e| (e.kind, e.text.as_deref(), e.ts.as_deref()))
            .collect();
        assert_eq!(
            said,
            [
                (
                    crate::model::EventKind::User,
                    Some("caf\u{e9}"),
                    Some("2026-11-03T08:30:55Z")
                ),
                (crate::model::EventKind::Assistant, Some("done"), None),
            ]
        );
        assert_eq!(
            (whole.session_id.as_deref(), whole.cwd.as_deref()),
            (Some("abc"), Some("/r"))
        );
        let mystery = &whole.skipped["mystery"];
        assert_eq!(mystery.records, 1);
        assert_eq!(json(&whole), json(&streamed));
        assert_eq!(json(&whole), json(&from_zst));
        // Only the unknown type is reported; the ignored ones are not.
        assert_eq!(whole.skipped.keys().collect::<Vec<_>>(), ["mystery"]);

        let claude = "{\"type\":\"user\",\"cwd\":\"/r\",\"message\":{\"role\":\"user\",\"content\":\"hi\"}}\n{\"type\":\"system\",\"version\":\"2.1.0\"}\n";
        let a = parse(Harness::ClaudeCode, "c.jsonl", claude.as_bytes()).unwrap();
        let b = parse_stream(Harness::ClaudeCode, "c.jsonl", Trickle(claude.as_bytes())).unwrap();
        assert_eq!(json(&a), json(&b));
        assert_eq!(a.harness_version.as_deref(), Some("2.1.0"));
        assert_eq!(
            (a.cwd.as_deref(), b.cwd.as_deref()),
            (Some("/r"), Some("/r"))
        );
        let said: Vec<_> = b
            .events
            .iter()
            .map(|e| (e.kind, e.text.as_deref()))
            .collect();
        assert_eq!(said, [(crate::model::EventKind::User, Some("hi"))]);
    }

    #[test]
    fn a_streamed_capture_that_does_not_hash_to_its_name_fails() {
        let dir = tempfile::tempdir().unwrap();
        let home = Home::new(dir.path());
        home.ensure().unwrap();
        let origin = "/u/.codex/sessions/2026/11/03/rollout-1.jsonl";
        let sha = {
            let _lock = home.lock_archive().unwrap();
            let mut archive = Archive::open(&home).unwrap();
            archive
                .put(Harness::Codex, origin, "f", CODEX.as_bytes())
                .unwrap()
                .unwrap()
                .capture_hash
        };
        let blob = Archive::open(&home).unwrap().blob_path(&sha);
        let mut bytes = std::fs::read(&blob).unwrap();
        *bytes.last_mut().unwrap() ^= 1;
        std::fs::write(&blob, bytes).unwrap();
        let r = normalize(&home, true).unwrap();
        assert_eq!((r.normalized, r.failed), (0, 1), "{r:?}");
        assert!(
            !home.events_path(Harness::Codex, "abc").exists(),
            "a corrupt capture left a session"
        );
    }
}
