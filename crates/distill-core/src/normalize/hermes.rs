//! Hermes keeps every session in `~/.hermes/state.db` (`sessions` and `messages`, OpenAI-style
//! chat rows) and writes some of them out as `~/.hermes/sessions/<id>.jsonl` (a `session_meta`
//! line, then one message per line) or older `session_<id>.json` files.
//!
//! The database holds more sessions than the files, so the sweep exports it one session at a
//! time (`<db>#session=<id>`: a `session_meta` row, then messages in order, so a growing
//! session appends). Those exports are the sessions; a file whose session the database also
//! holds is a copy of it.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::blocks::ts;
use super::chat::push_message;
use crate::model::{Event, EventKind, Parsed};
use crate::source::{Candidate, SourceKind};

fn open(db: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open {}", db.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    Ok(conn)
}

/// The session columns an export holds. Only ones set when a session starts, plus its title,
/// so an export grows by appending.
const SESSION_COLS: &str = "id, source, model, system_prompt, parent_session_id, started_at, \
                            cwd, git_branch, title";
/// The message columns an export holds, in order.
const MESSAGE_COLS: &str = "id, role, content, tool_call_id, tool_calls, tool_name, timestamp, \
                            reasoning, reasoning_content, finish_reason, active, compacted";

fn secs_to_rfc3339(t: f64) -> Option<String> {
    let ms = (t * 1000.0).round();
    (0.0..=253_402_300_799_999.0)
        .contains(&ms)
        .then(|| super::blocks::millis_to_rfc3339(ms as i64))
}

fn value(r: &rusqlite::Row, i: usize) -> rusqlite::Result<Value> {
    use rusqlite::types::ValueRef;
    Ok(match r.get_ref(i)? {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) => json!(n),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::String(hex::encode(b)),
    })
}

/// Every session in the database, with a hash of everything its export holds as the change
/// signal (compaction rewrites messages in place, so neither the newest id nor a session
/// timestamp is enough).
pub fn list_sessions(db: &Path) -> Result<Vec<Candidate>> {
    let conn = open(db)?;
    let mut hashes: std::collections::BTreeMap<String, Sha256> = Default::default();
    let mut stmt = conn.prepare(&format!("SELECT {SESSION_COLS} FROM sessions"))?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let mut h = Sha256::new();
        for i in 1..9 {
            h.update(value(r, i)?.to_string().as_bytes());
            h.update([0]);
        }
        hashes.insert(r.get(0)?, h);
    }
    let mut stmt = conn.prepare(&format!(
        "SELECT session_id, {MESSAGE_COLS} FROM messages ORDER BY session_id, timestamp, id"
    ))?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let Some(h) = hashes.get_mut(&r.get::<_, String>(0)?) else {
            continue;
        };
        for i in 1..13 {
            h.update(value(r, i)?.to_string().as_bytes());
            h.update([0]);
        }
    }
    Ok(hashes
        .into_iter()
        .map(|(session_id, h)| Candidate::DbSession {
            kind: SourceKind::Hermes,
            db: db.to_path_buf(),
            session_id,
            updated: hex::encode(h.finalize())[..16].to_string(),
        })
        .collect())
}

pub struct Export {
    pub bytes: Vec<u8>,
    pub cwd: Option<String>,
}

pub fn export_session(db: &Path, session_id: &str) -> Result<Export> {
    let conn = open(db)?;
    let header: Value = conn.query_row(
        &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
        [session_id],
        |r| {
            Ok(json!({
                "role": "session_meta",
                "id": value(r, 0)?,
                "platform": value(r, 1)?,
                "model": value(r, 2)?,
                "system_prompt": value(r, 3)?,
                "parent_session_id": value(r, 4)?,
                "timestamp": r.get::<_, Option<f64>>(5)?.and_then(secs_to_rfc3339),
                "cwd": value(r, 6)?,
                "git_branch": value(r, 7)?,
                "title": value(r, 8)?,
            }))
        },
    )?;
    let cwd = header
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|c| c.starts_with('/'))
        .map(str::to_string);
    let mut bytes = Vec::new();
    serde_json::to_writer(&mut bytes, &header)?;
    bytes.push(b'\n');
    let mut stmt = conn.prepare(&format!(
        "SELECT {MESSAGE_COLS} FROM messages WHERE session_id = ?1 ORDER BY timestamp, id"
    ))?;
    let mut rows = stmt.query([session_id])?;
    while let Some(r) = rows.next()? {
        // Tool calls are stored as JSON text; the export keeps them as JSON.
        let tool_calls = match value(r, 4)? {
            Value::String(s) => serde_json::from_str(&s).unwrap_or(Value::String(s)),
            other => other,
        };
        let reasoning = match value(r, 7)? {
            Value::Null => value(r, 8)?,
            v => v,
        };
        let row = json!({
            "id": value(r, 0)?,
            "role": value(r, 1)?,
            "content": value(r, 2)?,
            "tool_call_id": value(r, 3)?,
            "tool_calls": tool_calls,
            "name": value(r, 5)?,
            "timestamp": r.get::<_, Option<f64>>(6)?.and_then(secs_to_rfc3339),
            "reasoning": reasoning,
            "finish_reason": value(r, 9)?,
            "active": value(r, 10)?,
            "compacted": value(r, 11)?,
        });
        serde_json::to_writer(&mut bytes, &row)?;
        bytes.push(b'\n');
    }
    Ok(Export { bytes, cwd })
}

/// A Hermes session file's id: `<id>.jsonl` or `session_<id>.json`.
fn file_id(origin: &str) -> &str {
    let name = origin.rsplit('/').next().unwrap_or(origin);
    let name = name.strip_suffix(".jsonl").unwrap_or(name);
    let name = name.strip_suffix(".json").unwrap_or(name);
    name.strip_prefix("session_").unwrap_or(name)
}

pub fn parse(origin: &str, bytes: &[u8]) -> Result<Parsed> {
    if let Some((_, id)) = origin.split_once("#session=") {
        return Ok(parse_export(id, bytes));
    }
    let mut out = parse_file(origin, bytes)?;
    // A file is a copy of the database's session with the same id, when there is one.
    let id = out
        .session_id
        .clone()
        .unwrap_or_else(|| file_id(origin).to_string());
    out.session_id = Some(format!("file-{id}"));
    out.mirror_of = vec![SourceKind::Hermes];
    out.mirror_refs = vec![format!("hermes/{id}")];
    Ok(out)
}

fn parse_export(id: &str, bytes: &[u8]) -> Parsed {
    let mut out = Parsed {
        session_id: Some(id.to_string()),
        ..Default::default()
    };
    let s = |v: &Value, k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
    };
    for line in String::from_utf8_lossy(bytes).lines() {
        let Ok(m) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let role = m.get("role").and_then(Value::as_str).unwrap_or("");
        if role == "session_meta" && m.get("platform").is_some() {
            out.cwd = s(&m, "cwd").filter(|c| c.starts_with('/'));
            out.git_branch = s(&m, "git_branch");
            out.title = s(&m, "title");
            out.parent_session = s(&m, "parent_session_id");
            out.harness_version = s(&m, "platform");
            if let Some(md) = s(&m, "model") {
                out.add_model(&md);
            }
            if let Some(sp) = s(&m, "system_prompt") {
                out.push(Event::text(EventKind::System, sp).at(ts(&m)));
            }
            continue;
        }
        if role == "session_meta" {
            if let Some(md) = s(&m, "model") {
                out.add_model(&md);
            }
            continue;
        }
        push_message(&mut out, role, &m, ts(&m));
    }
    out
}

fn parse_file(origin: &str, bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let text = String::from_utf8_lossy(bytes);
    if origin.ends_with(".json") {
        let v: Value = serde_json::from_str(&text)?;
        out.session_id = v
            .get("session_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(m) = v.get("model").and_then(Value::as_str) {
            out.add_model(m);
        }
        let start = v
            .get("session_start")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(sp) = v.get("system_prompt").and_then(Value::as_str) {
            out.push(Event::text(EventKind::System, sp).at(start.clone()));
        }
        for (i, m) in v
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let role = m.get("role").and_then(Value::as_str).unwrap_or("");
            // These files carry no per-message times; the session start stands in for the first.
            let at = ts(m).or_else(|| (i == 0).then(|| start.clone()).flatten());
            push_message(&mut out, role, m, at);
        }
    } else {
        out.session_id = Some(
            origin
                .rsplit('/')
                .next()
                .unwrap_or(origin)
                .trim_end_matches(".jsonl")
                .to_string(),
        );
        for line in text.lines() {
            let Ok(m) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let role = m.get("role").and_then(Value::as_str).unwrap_or("");
            if role == "session_meta" {
                if let Some(md) = m.get("model").and_then(Value::as_str) {
                    out.add_model(md);
                }
                continue;
            }
            push_message(&mut out, role, &m, ts(&m));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_jsonl_and_json_sessions() {
        let jsonl = r#"{"role":"session_meta","model":"glm-5","platform":"telegram","timestamp":"2026-10-01T00:00:00Z"}
{"role":"user","content":"weather?","timestamp":"2026-10-01T00:00:01Z"}
{"role":"assistant","content":"","reasoning":"look it up","tool_calls":[{"id":"c1","function":{"name":"web","arguments":"{\"q\":\"weather\"}"}}],"timestamp":"2026-10-01T00:00:02Z"}
{"role":"tool","tool_call_id":"c1","content":"sunny","timestamp":"2026-10-01T00:00:03Z"}"#;
        let p = parse("/h/sessions/20261001_000000_ab12.jsonl", jsonl.as_bytes()).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("file-20261001_000000_ab12"));
        assert_eq!(p.mirror_refs, vec!["hermes/20261001_000000_ab12"]);
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::User,
                EventKind::Reasoning,
                EventKind::ToolCall,
                EventKind::ToolResult
            ]
        );
        assert_eq!(p.events[2].input.as_ref().unwrap()["q"], "weather");
        assert_eq!(p.models, vec!["glm-5"]);

        let json = r#"{"session_id":"s9","model":"m","session_start":"2026-09-01T00:00:00Z","system_prompt":"be brief","messages":[{"role":"user","content":"hi"},{"role":"assistant","content":"hello"}]}"#;
        let p = parse("/h/sessions/session_s9.json", json.as_bytes()).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("file-s9"));
        assert_eq!(p.events.len(), 3);
        assert_eq!(p.events[1].ts.as_deref(), Some("2026-09-01T00:00:00Z"));
    }

    #[test]
    fn exports_database_sessions_and_files_become_copies() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("state.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE sessions (id TEXT PRIMARY KEY, source TEXT NOT NULL, model TEXT, system_prompt TEXT,
                 parent_session_id TEXT, started_at REAL NOT NULL, cwd TEXT, git_branch TEXT, title TEXT);
               CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, session_id TEXT NOT NULL, role TEXT NOT NULL,
                 content TEXT, tool_call_id TEXT, tool_calls TEXT, tool_name TEXT, timestamp REAL NOT NULL,
                 reasoning TEXT, reasoning_content TEXT, finish_reason TEXT, active INTEGER NOT NULL DEFAULT 1,
                 compacted INTEGER NOT NULL DEFAULT 0);
               INSERT INTO sessions VALUES ('20261101_100000_aa', 'cli', 'glm-5', 'be brief', NULL, 1793527200.0,
                 '/work/app', 'main', 'Weather');
               INSERT INTO messages (session_id, role, content, tool_calls, timestamp) VALUES
                 ('20261101_100000_aa', 'user', 'weather?', NULL, 1793527201.0),
                 ('20261101_100000_aa', 'assistant', '', '[{"id":"c1","function":{"name":"web","arguments":"{\"q\":\"weather\"}"}}]', 1793527202.0);
               INSERT INTO messages (session_id, role, content, tool_call_id, tool_name, timestamp) VALUES
                 ('20261101_100000_aa', 'tool', 'sunny', 'c1', 'web', 1793527203.0);"#,
        )
        .unwrap();
        let listed = list_sessions(&db).unwrap();
        assert_eq!(listed.len(), 1);
        let origin = listed[0].origin();
        let export = export_session(&db, "20261101_100000_aa").unwrap();
        assert_eq!(export.cwd.as_deref(), Some("/work/app"));
        let p = parse(&origin, &export.bytes).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("20261101_100000_aa"));
        assert_eq!(p.title.as_deref(), Some("Weather"));
        assert_eq!(p.git_branch.as_deref(), Some("main"));
        assert!(!p.is_mirror());
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::System,
                EventKind::User,
                EventKind::ToolCall,
                EventKind::ToolResult
            ]
        );
        assert_eq!(p.events[2].input.as_ref().unwrap()["q"], "weather");
        assert_eq!(p.events[1].ts.as_deref(), Some("2026-11-01T10:00:01.000Z"));

        // A new message appends; compaction (an in-place update) changes the fingerprint.
        conn.execute(
            "INSERT INTO messages (session_id, role, content, timestamp) VALUES ('20261101_100000_aa', 'user', 'thanks', 1793527300.0)",
            [],
        )
        .unwrap();
        assert!(
            export_session(&db, "20261101_100000_aa")
                .unwrap()
                .bytes
                .starts_with(&export.bytes)
        );
        let before = list_sessions(&db).unwrap()[0].fingerprint();
        conn.execute("UPDATE messages SET compacted = 1 WHERE id = 1", [])
            .unwrap();
        assert_ne!(list_sessions(&db).unwrap()[0].fingerprint(), before);
    }
}
