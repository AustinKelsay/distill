//! The eleven harnesses: where each keeps its sessions, which captured files are sessions, and
//! how the sweep finds and names what to capture.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::home::user_home;

/// A harness whose sessions Distill collects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Harness {
    ClaudeCode,
    Codex,
    #[serde(rename = "opencode")]
    OpenCode,
    Droid,
    Pi,
    Cursor,
    Grok,
    Antigravity,
    Hermes,
    /// T3 Code's own thread database: which harness, model and worktree each thread used.
    #[serde(rename = "t3code")]
    T3Code,
    Goose,
}

impl Harness {
    pub const ALL: [Harness; 11] = [
        Harness::ClaudeCode,
        Harness::Codex,
        Harness::OpenCode,
        Harness::Droid,
        Harness::Pi,
        Harness::Cursor,
        Harness::Grok,
        Harness::Antigravity,
        Harness::Hermes,
        Harness::T3Code,
        Harness::Goose,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude-code",
            Harness::Codex => "codex",
            Harness::OpenCode => "opencode",
            Harness::Droid => "droid",
            Harness::Pi => "pi",
            Harness::Cursor => "cursor",
            Harness::Grok => "grok",
            Harness::Antigravity => "antigravity",
            Harness::Hermes => "hermes",
            Harness::T3Code => "t3code",
            Harness::Goose => "goose",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "Claude Code",
            Harness::Codex => "Codex",
            Harness::OpenCode => "OpenCode",
            Harness::Droid => "Droid",
            Harness::Pi => "Pi",
            Harness::Cursor => "Cursor",
            Harness::Grok => "Grok",
            Harness::Antigravity => "Antigravity",
            Harness::Hermes => "Hermes",
            Harness::T3Code => "T3 Code",
            Harness::Goose => "Goose",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == s)
    }

    /// Whether Distill can turn this harness's captures into events. Harnesses without a
    /// parser are still archived, so nothing is lost while one is written.
    pub fn has_parser(self) -> bool {
        crate::normalize::parser_version(self).is_some()
    }

    pub fn default_roots(self) -> Vec<PathBuf> {
        let Ok(home) = user_home() else {
            return Vec::new();
        };
        match self {
            Harness::ClaudeCode => vec![home.join(".claude/projects")],
            Harness::Codex => {
                vec![
                    home.join(".codex/sessions"),
                    home.join(".codex/archived_sessions"),
                ]
            }
            Harness::OpenCode => vec![home.join(".local/share/opencode/opencode.db")],
            Harness::Droid => vec![home.join(".factory/sessions")],
            Harness::Pi => vec![home.join(".pi/agent/sessions")],
            // Chats from the agent CLI and IDE, sessions run over ACP, agents run through the SDK
            // (`projects/*/sdk-agent-store`), and the plain transcripts it keeps per project.
            Harness::Cursor => vec![
                home.join(".cursor/chats"),
                home.join(".cursor/acp-sessions"),
                home.join(".cursor/projects"),
                home.join(".cursor/chat-exports"),
            ],
            Harness::Grok => vec![home.join(".grok/sessions")],
            Harness::Antigravity => vec![
                home.join(".t3/userdata/providers/antigravity"),
                home.join(".gemini/antigravity"),
            ],
            Harness::Hermes => {
                vec![home.join(".hermes/sessions"), home.join(".hermes/state.db")]
            }
            Harness::T3Code => vec![home.join(".t3/userdata/statev2.sqlite")],
            Harness::Goose => vec![home.join(".local/share/goose/sessions/sessions.db")],
        }
    }

    /// Whether a captured file is a session this harness's parser reads. Everything else under
    /// a harness's roots (tool-result files, attachments, settings, databases) is archived as a
    /// sidecar so nothing is lost, but not parsed on its own.
    pub fn is_primary(self, origin: &str) -> bool {
        let name = origin.rsplit('/').next().unwrap_or(origin);
        match self {
            Harness::ClaudeCode | Harness::Droid | Harness::Pi => name.ends_with(".jsonl"),
            Harness::Codex => name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"),
            // Sessions are exported from the database; its spilled tool outputs are sidecars.
            Harness::OpenCode => origin.contains("#session="),
            // A thread's native references are archived beside it but aren't a session.
            Harness::T3Code => !origin.ends_with(crate::normalize::t3code::NATIVE_SUFFIX),
            Harness::Grok => name == "chat_history.jsonl",
            Harness::Antigravity => name == "transcript_full.jsonl",
            Harness::Hermes => {
                name.ends_with(".jsonl")
                    || (name.starts_with("session_") && name.ends_with(".json"))
                    || origin.contains("#session=")
            }
            Harness::Cursor => {
                name == "store.db" || cursor_transcript(origin) || cursor_export(origin)
            }
            Harness::Goose => true,
        }
    }
}

impl std::fmt::Display for Harness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// One thing the sweep can capture.
#[derive(Debug, Clone)]
pub enum Candidate {
    /// A session file copied byte for byte.
    File {
        kind: Harness,
        path: PathBuf,
        size: u64,
        mtime_ns: i128,
    },
    /// A SQLite file copied through SQLite's backup API, so a live writer can't tear it.
    Sqlite {
        kind: Harness,
        path: PathBuf,
        size: u64,
        mtime_ns: i128,
    },
    /// One OpenCode session exported from its database as JSONL rows.
    OpenCodeSession {
        db: PathBuf,
        session_id: String,
        updated: i64,
    },
    /// A tool output OpenCode spilled to `tool-output/` beside its database `db`, copied byte for
    /// byte before OpenCode deletes it.
    OpenCodeSpill {
        db: PathBuf,
        path: PathBuf,
        size: u64,
        mtime_ns: i128,
    },
    /// One T3 Code thread exported from its database as JSONL rows.
    T3Thread {
        db: PathBuf,
        thread_id: String,
        updated: String,
    },
    /// One session of a harness that keeps them all in one database (Goose, Hermes), exported
    /// as JSONL rows. `updated` is a hash of what the export holds.
    DbSession {
        kind: Harness,
        db: PathBuf,
        session_id: String,
        updated: String,
    },
}

impl Candidate {
    pub fn kind(&self) -> Harness {
        match self {
            Candidate::File { kind, .. } | Candidate::Sqlite { kind, .. } => *kind,
            Candidate::OpenCodeSession { .. } | Candidate::OpenCodeSpill { .. } => {
                Harness::OpenCode
            }
            Candidate::T3Thread { .. } => Harness::T3Code,
            Candidate::DbSession { kind, .. } => *kind,
        }
    }

    /// Stable key identifying where this capture came from.
    pub fn origin(&self) -> String {
        match self {
            Candidate::File { path, .. }
            | Candidate::Sqlite { path, .. }
            | Candidate::OpenCodeSpill { path, .. } => path.to_string_lossy().into_owned(),
            Candidate::OpenCodeSession { db, session_id, .. } => {
                format!("{}#session={}", db.to_string_lossy(), session_id)
            }
            Candidate::T3Thread { db, thread_id, .. } => {
                format!("{}#thread={}", db.to_string_lossy(), thread_id)
            }
            Candidate::DbSession { db, session_id, .. } => {
                format!("{}#session={}", db.to_string_lossy(), session_id)
            }
        }
    }

    /// The change signal: if this matches the last sweep's, the bytes are not re-read. The
    /// manifest stores it as `fingerprint`; it is not a hash.
    pub fn change_signal(&self) -> String {
        match self {
            Candidate::File { size, mtime_ns, .. }
            | Candidate::Sqlite { size, mtime_ns, .. }
            | Candidate::OpenCodeSpill { size, mtime_ns, .. } => format!("{size}:{mtime_ns}"),
            Candidate::OpenCodeSession { updated, .. } => format!("updated:{updated}"),
            Candidate::T3Thread { updated, .. } | Candidate::DbSession { updated, .. } => {
                format!("updated:{updated}")
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Detection {
    pub kind: Harness,
    pub roots: Vec<PathBuf>,
    pub found: bool,
    pub bytes: u64,
    pub files: u64,
    pub has_parser: bool,
}

/// What exists on disk for a harness, for `distill harnesses` and setup.
pub fn detect(kind: Harness, roots: &[PathBuf]) -> Detection {
    let mut bytes = 0;
    let mut files = 0;
    let mut found = false;
    for root in roots {
        if !root.exists() {
            continue;
        }
        found = true;
        for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
            if entry.file_type().is_file() {
                files += 1;
                bytes += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    Detection {
        kind,
        roots: roots.to_vec(),
        found,
        bytes,
        files,
        has_parser: kind.has_parser(),
    }
}

fn mtime_ns(meta: &fs::Metadata) -> i128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i128)
        .unwrap_or(0)
}

/// List capture candidates under a harness's roots.
pub fn discover(kind: Harness, roots: &[PathBuf]) -> anyhow::Result<Vec<Candidate>> {
    let mut out = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        match kind {
            Harness::OpenCode => {
                let db = crate::normalize::opencode::database(root);
                if db.exists() {
                    out.extend(crate::normalize::opencode::list_sessions(&db)?);
                    out.extend(crate::normalize::opencode::list_spills(&db)?);
                }
            }
            Harness::T3Code => {
                if root.is_file() {
                    out.extend(crate::normalize::t3code::list_threads(root)?);
                }
            }
            Harness::Goose => {
                if root.is_file() {
                    out.extend(crate::normalize::goose::list_sessions(root)?);
                }
            }
            // Hermes keeps every session in `state.db` too (more of them than it writes out as
            // files); it is exported one session at a time, not copied whole.
            Harness::Hermes
                if root.is_file()
                    && [".db", ".sqlite", ".sqlite3"]
                        .iter()
                        .any(|e| root.to_string_lossy().ends_with(e)) =>
            {
                out.extend(crate::normalize::hermes::list_sessions(root)?);
            }
            _ => {
                let exports = kind == Harness::Cursor && root.ends_with(".cursor/chat-exports");
                let walk = WalkDir::new(root).follow_root_links(!exports);
                let walk = if exports { walk.max_depth(1) } else { walk };
                for entry in walk.into_iter().filter_map(Result::ok) {
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    let path = entry.path();
                    if exports && !cursor_export(&path.to_string_lossy()) {
                        continue;
                    }
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    let Ok(meta) = entry.metadata() else { continue };
                    let (size, mtime_ns) = (meta.len(), mtime_ns(&meta));
                    // SQLite's journal files are read through the database itself; lock files
                    // are noise.
                    if name.ends_with("-wal")
                        || name.ends_with("-shm")
                        || name.ends_with("-journal")
                        || name.ends_with(".lock")
                        || name == ".DS_Store"
                    {
                        continue;
                    }
                    // Cursor makes an ACP session folder for every connection; only the ones
                    // that held a conversation have a store beside their `meta.json`.
                    if kind == Harness::Cursor
                        && name == "meta.json"
                        && path.components().any(|c| c.as_os_str() == "acp-sessions")
                        && !path.with_file_name("store.db").exists()
                    {
                        continue;
                    }
                    let is_sqlite = [".db", ".sqlite", ".sqlite3"]
                        .iter()
                        .any(|e| name.ends_with(e));
                    if is_sqlite {
                        // Committed writes can land only in the WAL, so it is part of the
                        // change signal.
                        let wal = fs::metadata(format!("{}-wal", path.display())).ok();
                        out.push(Candidate::Sqlite {
                            kind,
                            path: path.to_path_buf(),
                            size: size + wal.as_ref().map_or(0, |m| m.len()),
                            mtime_ns: mtime_ns.max(wal.as_ref().map_or(0, self::mtime_ns)),
                        });
                    } else {
                        out.push(Candidate::File {
                            kind,
                            path: path.to_path_buf(),
                            size,
                            mtime_ns,
                        });
                    }
                }
            }
        }
    }
    out.sort_by_key(|c| c.origin());
    Ok(out)
}

/// How a harness turns a working directory into a folder name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathEncoding {
    /// Claude Code: `/` and `.` both become `-` (lossy).
    DashSlashDot,
    /// Droid: `/` becomes `-`; dots are kept.
    DashSlash,
    /// Grok: percent-encoded, decoded exactly.
    Exact,
}

/// Working directory encoded in a file's path, for harnesses that name folders after it: the
/// first folder below the harness root it was found under.
pub fn path_cwd(kind: Harness, roots: &[PathBuf], path: &Path) -> Option<(String, PathEncoding)> {
    let rel = roots.iter().find_map(|r| path.strip_prefix(r).ok())?;
    let mut parts = rel.components();
    let first = parts.next()?.as_os_str().to_str()?.to_string();
    // A file directly in the root has no folder to read.
    parts.next()?;
    match kind {
        Harness::ClaudeCode => Some((first, PathEncoding::DashSlashDot)),
        Harness::Droid => first
            .starts_with('-')
            .then_some((first, PathEncoding::DashSlash)),
        Harness::Grok => Some((percent_decode(&first), PathEncoding::Exact)),
        _ => None,
    }
}

/// Harnesses whose sessions record no working directory at all, so exclusions can't be checked.
/// (Cursor's transcripts don't; its chat stores are databases, which fail closed anyway.)
pub fn records_no_cwd(kind: Harness) -> bool {
    matches!(
        kind,
        Harness::Antigravity | Harness::Hermes | Harness::Cursor
    )
}

/// A Cursor agent transcript: `projects/<project>/agent-transcripts/[…/]<agent id>.jsonl`.
pub fn cursor_transcript(origin: &str) -> bool {
    origin.contains("/agent-transcripts/") && origin.ends_with(".jsonl")
}

/// One Markdown transcript explicitly saved in the Cursor export inbox.
pub fn cursor_export(origin: &str) -> bool {
    let path = Path::new(origin);
    path.extension().is_some_and(|ext| ext == "md")
        && path
            .parent()
            .is_some_and(|dir| dir.ends_with(".cursor/chat-exports"))
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        // Hex digits are ASCII, so checking bytes never splits a multibyte character.
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && bytes[i + 1].is_ascii_hexdigit()
            && bytes[i + 2].is_ascii_hexdigit()
        {
            let hex = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(bytes[i + 1]) * 16 + hex(bytes[i + 2]));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Whether an excluded path matches a cwd taken from `path_cwd`.
pub fn path_cwd_excluded(encoded: &str, enc: PathEncoding, exclude: &[String]) -> bool {
    exclude.iter().any(|p| {
        let p = p.trim_end_matches('/');
        if p.is_empty() {
            return false;
        }
        let want: String = match enc {
            PathEncoding::DashSlashDot => p
                .chars()
                .map(|c| if c == '/' || c == '.' { '-' } else { c })
                .collect(),
            PathEncoding::DashSlash => p.replace('/', "-"),
            PathEncoding::Exact => p.to_string(),
        };
        let sep = if enc == PathEncoding::Exact { "/" } else { "-" };
        encoded == want || encoded.starts_with(&format!("{want}{sep}"))
    })
}

/// Best-effort working directory of a session file, read from its first records, so excluded
/// repos can be skipped before anything is copied.
pub fn peek_cwd(kind: Harness, path: &Path) -> Option<String> {
    let mut buf = Vec::new();
    let file = fs::File::open(path).ok()?;
    // Compressed Codex sessions: read the head of the decompressed stream.
    if path.to_string_lossy().ends_with(".zst") {
        let _ = zstd::stream::read::Decoder::new(file)
            .ok()?
            .take(256 * 1024)
            .read_to_end(&mut buf);
    } else {
        file.take(256 * 1024).read_to_end(&mut buf).ok()?;
    }
    cwd_in(kind, &buf)
}

/// The working directory a session's first records name, read from its bytes.
pub fn cwd_in(kind: Harness, bytes: &[u8]) -> Option<String> {
    let head = &bytes[..bytes.len().min(256 * 1024)];
    let text = String::from_utf8_lossy(head);
    for line in text.lines().take(50) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let cwd = match kind {
            Harness::Codex => v.pointer("/payload/cwd"),
            _ => v.get("cwd"),
        };
        if let Some(s) = cwd.and_then(|c| c.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_names_match_ids() {
        for k in Harness::ALL {
            assert_eq!(serde_json::to_value(k).unwrap(), k.id(), "{k:?}");
            assert_eq!(Harness::parse(k.id()), Some(k));
        }
    }
}
