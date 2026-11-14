use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::home::user_home;

/// A harness whose sessions Distill collects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
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
}

impl SourceKind {
    pub const ALL: [SourceKind; 10] = [
        SourceKind::ClaudeCode,
        SourceKind::Codex,
        SourceKind::OpenCode,
        SourceKind::Droid,
        SourceKind::Pi,
        SourceKind::Cursor,
        SourceKind::Grok,
        SourceKind::Antigravity,
        SourceKind::Hermes,
        SourceKind::T3Code,
    ];

    pub fn id(self) -> &'static str {
        match self {
            SourceKind::ClaudeCode => "claude-code",
            SourceKind::Codex => "codex",
            SourceKind::OpenCode => "opencode",
            SourceKind::Droid => "droid",
            SourceKind::Pi => "pi",
            SourceKind::Cursor => "cursor",
            SourceKind::Grok => "grok",
            SourceKind::Antigravity => "antigravity",
            SourceKind::Hermes => "hermes",
            SourceKind::T3Code => "t3code",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SourceKind::ClaudeCode => "Claude Code",
            SourceKind::Codex => "Codex",
            SourceKind::OpenCode => "OpenCode",
            SourceKind::Droid => "Droid",
            SourceKind::Pi => "Pi",
            SourceKind::Cursor => "Cursor",
            SourceKind::Grok => "Grok",
            SourceKind::Antigravity => "Antigravity",
            SourceKind::Hermes => "Hermes",
            SourceKind::T3Code => "T3 Code",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.id() == s)
    }

    /// Whether Distill can turn this source's raw captures into trajectories. Sources without a
    /// parser are still archived, so nothing is lost while one is written.
    pub fn has_parser(self) -> bool {
        !matches!(self, SourceKind::Cursor)
    }

    pub fn default_roots(self) -> Vec<PathBuf> {
        let Ok(home) = user_home() else {
            return Vec::new();
        };
        match self {
            SourceKind::ClaudeCode => vec![home.join(".claude/projects")],
            SourceKind::Codex => {
                vec![
                    home.join(".codex/sessions"),
                    home.join(".codex/archived_sessions"),
                ]
            }
            SourceKind::OpenCode => vec![home.join(".local/share/opencode/opencode.db")],
            SourceKind::Droid => vec![home.join(".factory/sessions")],
            SourceKind::Pi => vec![home.join(".pi/agent/sessions")],
            SourceKind::Cursor => vec![home.join(".cursor/chats")],
            SourceKind::Grok => vec![home.join(".grok/sessions")],
            SourceKind::Antigravity => vec![
                home.join(".t3/userdata/providers/antigravity"),
                home.join(".gemini/antigravity"),
            ],
            SourceKind::Hermes => {
                vec![home.join(".hermes/sessions"), home.join(".hermes/state.db")]
            }
            SourceKind::T3Code => vec![home.join(".t3/userdata/statev2.sqlite")],
        }
    }

    /// Whether a captured file is a session this source's parser reads. Everything else under
    /// a source's roots (tool-result files, attachments, settings, databases) is archived as a
    /// sidecar so nothing is lost, but not parsed on its own.
    pub fn is_primary(self, origin: &str) -> bool {
        let name = origin.rsplit('/').next().unwrap_or(origin);
        match self {
            SourceKind::ClaudeCode | SourceKind::Droid | SourceKind::Pi => name.ends_with(".jsonl"),
            SourceKind::Codex => name.ends_with(".jsonl") || name.ends_with(".jsonl.zst"),
            SourceKind::OpenCode | SourceKind::T3Code => true,
            SourceKind::Grok => name == "chat_history.jsonl",
            SourceKind::Antigravity => name == "transcript_full.jsonl",
            SourceKind::Hermes => {
                name.ends_with(".jsonl")
                    || (name.starts_with("session_") && name.ends_with(".json"))
            }
            SourceKind::Cursor => false,
        }
    }
}

impl std::fmt::Display for SourceKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// One thing the sweep can capture.
#[derive(Debug, Clone)]
pub enum Candidate {
    /// A session file copied byte for byte.
    File {
        kind: SourceKind,
        path: PathBuf,
        size: u64,
        mtime_ns: i128,
    },
    /// A SQLite file copied through SQLite's backup API, so a live writer can't tear it.
    Sqlite {
        kind: SourceKind,
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
    /// One T3 Code thread exported from its database as JSONL rows.
    T3Thread {
        db: PathBuf,
        thread_id: String,
        updated: String,
    },
}

impl Candidate {
    pub fn kind(&self) -> SourceKind {
        match self {
            Candidate::File { kind, .. } | Candidate::Sqlite { kind, .. } => *kind,
            Candidate::OpenCodeSession { .. } => SourceKind::OpenCode,
            Candidate::T3Thread { .. } => SourceKind::T3Code,
        }
    }

    /// Stable key identifying where this capture came from.
    pub fn origin(&self) -> String {
        match self {
            Candidate::File { path, .. } | Candidate::Sqlite { path, .. } => {
                path.to_string_lossy().into_owned()
            }
            Candidate::OpenCodeSession { db, session_id, .. } => {
                format!("{}#session={}", db.to_string_lossy(), session_id)
            }
            Candidate::T3Thread { db, thread_id, .. } => {
                format!("{}#thread={}", db.to_string_lossy(), thread_id)
            }
        }
    }

    /// A cheap change signal: if this matches the last sweep, the bytes are not re-read.
    pub fn fingerprint(&self) -> String {
        match self {
            Candidate::File { size, mtime_ns, .. } | Candidate::Sqlite { size, mtime_ns, .. } => {
                format!("{size}:{mtime_ns}")
            }
            Candidate::OpenCodeSession { updated, .. } => format!("updated:{updated}"),
            Candidate::T3Thread { updated, .. } => format!("updated:{updated}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Detection {
    pub kind: SourceKind,
    pub roots: Vec<PathBuf>,
    pub found: bool,
    pub bytes: u64,
    pub files: u64,
    pub has_parser: bool,
}

/// What exists on disk for a source, for setup and the dashboard.
pub fn detect(kind: SourceKind, roots: &[PathBuf]) -> Detection {
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

/// List capture candidates under a source's roots.
pub fn discover(kind: SourceKind, roots: &[PathBuf]) -> anyhow::Result<Vec<Candidate>> {
    let mut out = Vec::new();
    for root in roots {
        if !root.exists() {
            continue;
        }
        match kind {
            SourceKind::OpenCode => {
                let db = if root.is_dir() {
                    root.join("opencode.db")
                } else {
                    root.clone()
                };
                if db.exists() {
                    out.extend(crate::normalize::opencode::list_sessions(&db)?);
                }
            }
            SourceKind::T3Code => {
                if root.is_file() {
                    out.extend(crate::normalize::t3code::list_threads(root)?);
                }
            }
            _ => {
                for entry in WalkDir::new(root).into_iter().filter_map(Result::ok) {
                    if !entry.file_type().is_file() {
                        continue;
                    }
                    let path = entry.path();
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

/// Working directory encoded in a file's path, for harnesses that name folders after it.
pub fn path_cwd(kind: SourceKind, path: &Path) -> Option<(String, PathEncoding)> {
    let s = path.to_string_lossy();
    let after = |marker: &str| {
        s.split(marker)
            .nth(1)
            .and_then(|r| r.split('/').next())
            .map(str::to_string)
    };
    match kind {
        SourceKind::ClaudeCode => after("/projects/").map(|d| (d, PathEncoding::DashSlashDot)),
        SourceKind::Droid => after("/sessions/")
            .filter(|d| d.starts_with('-'))
            .map(|d| (d, PathEncoding::DashSlash)),
        SourceKind::Grok => after("/sessions/").map(|d| (percent_decode(&d), PathEncoding::Exact)),
        _ => None,
    }
}

/// Sources whose sessions record no working directory at all, so exclusions can't be checked.
pub fn records_no_cwd(kind: SourceKind) -> bool {
    matches!(kind, SourceKind::Antigravity | SourceKind::Hermes)
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16)
        {
            out.push(b);
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
pub fn peek_cwd(kind: SourceKind, path: &Path) -> Option<String> {
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
    let text = String::from_utf8_lossy(&buf);
    for line in text.lines().take(50) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let cwd = match kind {
            SourceKind::Codex => v.pointer("/payload/cwd"),
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
        for k in SourceKind::ALL {
            assert_eq!(serde_json::to_value(k).unwrap(), k.id(), "{k:?}");
            assert_eq!(SourceKind::parse(k.id()), Some(k));
        }
    }
}
