//! Goose keeps every session in one SQLite database (`sessions.db`: `sessions` and `messages`).
//! The sweep exports each session as JSONL rows, the session first and then its messages in
//! order, and that export is what the archive stores; a growing session appends.
//!
//! A message's `content_json` is a list of blocks: `text`, `thinking` (streamed, so often one
//! word per block), `toolRequest`, `toolResponse` and `image`.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::blocks::millis_to_rfc3339;
use crate::model::{Event, EventKind, Parsed};
use crate::source::Candidate;

fn open(db: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open {}", db.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

/// Every session, with a hash of everything its export holds as the change signal. Goose
/// edits messages in place (tool metadata, for one) without touching the session's
/// `updated_at`, so neither that nor the newest message id is enough. The database is small;
/// hashing it each sweep is cheap.
pub fn list_sessions(db: &Path) -> Result<Vec<Candidate>> {
    let conn = open(db)?;
    let mut hashes: std::collections::BTreeMap<String, Sha256> = Default::default();
    // Each value is hashed with whether it is NULL and its length, as the export tells them
    // apart.
    fn field(h: &mut Sha256, v: Option<String>) {
        match v {
            None => h.update([0]),
            Some(s) => {
                h.update([1]);
                h.update((s.len() as u64).to_le_bytes());
                h.update(s.as_bytes());
            }
        }
    }
    let mut stmt = conn.prepare(
        "SELECT id, working_dir, created_at, session_type, provider_name, model_config_json,
                parent_session_id
         FROM sessions",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let mut h = Sha256::new();
        for i in 1..7 {
            field(&mut h, r.get::<_, Option<String>>(i)?);
        }
        hashes.insert(r.get(0)?, h);
    }
    let mut stmt = conn.prepare(
        "SELECT session_id, id, message_id, role, content_json, created_timestamp
         FROM messages ORDER BY session_id, created_timestamp, id",
    )?;
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let Some(h) = hashes.get_mut(&r.get::<_, String>(0)?) else {
            continue;
        };
        h.update(r.get::<_, i64>(1)?.to_le_bytes());
        for i in 2..5 {
            field(h, r.get::<_, Option<String>>(i)?);
        }
        h.update(r.get::<_, i64>(5)?.to_le_bytes());
    }
    Ok(hashes
        .into_iter()
        .map(|(session_id, h)| Candidate::GooseSession {
            db: db.to_path_buf(),
            session_id,
            updated: hex::encode(h.finalize())[..16].to_string(),
        })
        .collect())
}

pub struct Export {
    pub bytes: Vec<u8>,
    pub working_dir: Option<String>,
}

pub fn export_session(db: &Path, session_id: &str) -> Result<Export> {
    let conn = open(db)?;
    // Only fields set when the session is made, so the export grows by appending.
    let session: Value = conn.query_row(
        "SELECT id, working_dir, created_at, session_type, provider_name, model_config_json,
                parent_session_id
         FROM sessions WHERE id = ?1",
        [session_id],
        |r| {
            let model = r
                .get::<_, Option<String>>(5)?
                .and_then(|m| serde_json::from_str::<Value>(&m).ok())
                .and_then(|m| m.get("model_name").cloned());
            Ok(json!({
                "row": "session",
                "id": r.get::<_, String>(0)?,
                "working_dir": r.get::<_, String>(1)?,
                "created_at": r.get::<_, Option<String>>(2)?,
                "session_type": r.get::<_, String>(3)?,
                "provider": r.get::<_, Option<String>>(4)?,
                "model": model,
                "parent_session_id": r.get::<_, Option<String>>(6)?,
            }))
        },
    )?;
    let working_dir = session
        .get("working_dir")
        .and_then(Value::as_str)
        .filter(|d| d.starts_with('/'))
        .map(str::to_string);
    let mut bytes = Vec::new();
    serde_json::to_writer(&mut bytes, &session)?;
    bytes.push(b'\n');
    let mut stmt = conn.prepare(
        "SELECT id, message_id, role, content_json, created_timestamp
         FROM messages WHERE session_id = ?1 ORDER BY created_timestamp, id",
    )?;
    let rows = stmt.query_map([session_id], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;
    for row in rows {
        let (id, message_id, role, content, created) = row?;
        let content: Value = serde_json::from_str(&content).unwrap_or(Value::String(content));
        serde_json::to_writer(
            &mut bytes,
            &json!({"row": "message", "id": id, "message_id": message_id, "role": role,
                    "created": created, "content": content}),
        )?;
        bytes.push(b'\n');
    }
    Ok(Export { bytes, working_dir })
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|x| !x.is_empty())
        .map(str::to_string)
}

/// Goose writes `created_timestamp` in seconds. Anything outside 1970–9999 is not a time.
fn when(v: &Value) -> Option<String> {
    let t = v.get("created").and_then(Value::as_i64)?;
    let ms = if t > 100_000_000_000 {
        t
    } else {
        t.checked_mul(1000)?
    };
    (0..=253_402_300_799_999)
        .contains(&ms)
        .then(|| millis_to_rfc3339(ms))
}

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let text = String::from_utf8_lossy(bytes);
    let mut model = None;
    // Streamed thinking arrives a word or two per block; it is joined into one event.
    let mut thinking: Option<(String, Option<String>)> = None;
    let flush = |out: &mut Parsed, thinking: &mut Option<(String, Option<String>)>| {
        if let Some((t, at)) = thinking.take()
            && !t.trim().is_empty()
        {
            out.push(Event::text(EventKind::Reasoning, t).at(at));
        }
    };
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("row").and_then(Value::as_str) {
            Some("session") => {
                out.session_id = s(&v, "id");
                out.cwd = s(&v, "working_dir").filter(|d| d.starts_with('/'));
                out.parent_session = s(&v, "parent_session_id");
                out.harness_version = s(&v, "provider");
                model = s(&v, "model");
                if let Some(m) = &model {
                    out.add_model(m);
                }
            }
            Some("message") => {
                let at = when(&v);
                let role = v.get("role").and_then(Value::as_str).unwrap_or("");
                let blocks = match v.get("content") {
                    Some(Value::Array(b)) => b.clone(),
                    Some(Value::String(t)) => vec![json!({"type": "text", "text": t})],
                    _ => Vec::new(),
                };
                for b in &blocks {
                    let ty = b.get("type").and_then(Value::as_str).unwrap_or("");
                    if ty == "thinking" {
                        let chunk = b.get("thinking").and_then(Value::as_str).unwrap_or("");
                        let (buf, _) = thinking.get_or_insert_with(|| (String::new(), at.clone()));
                        buf.push_str(chunk);
                        continue;
                    }
                    flush(&mut out, &mut thinking);
                    match ty {
                        "text" => {
                            let kind = if role == "assistant" {
                                EventKind::Assistant
                            } else {
                                EventKind::User
                            };
                            let t = b.get("text").and_then(Value::as_str).unwrap_or("");
                            if !t.is_empty() {
                                let mut e = Event::text(kind, t).at(at.clone());
                                if kind == EventKind::Assistant {
                                    e.model = model.clone();
                                }
                                out.push(e);
                            }
                        }
                        "image" => out.push(Event::text(EventKind::User, "[image]").at(at.clone())),
                        "toolRequest" => {
                            let call = b.pointer("/toolCall/value").unwrap_or(&Value::Null);
                            let mut e = Event::new(EventKind::ToolCall).at(at.clone());
                            e.tool = s(call, "name");
                            e.call_id = s(b, "id");
                            e.input = call.get("arguments").cloned();
                            e.model = model.clone();
                            out.push(e);
                        }
                        "toolResponse" => {
                            let result = b.get("toolResult").unwrap_or(&Value::Null);
                            let mut e = Event::new(EventKind::ToolResult).at(at.clone());
                            e.call_id = s(b, "id");
                            let content = result
                                .pointer("/value/content")
                                .or_else(|| result.get("value"))
                                .or_else(|| result.get("error"))
                                .unwrap_or(&Value::Null);
                            e.text = Some(super::blocks::result_text(content));
                            // A tool can fail inside a call that went through: MCP reports that
                            // as `isError` in the result.
                            e.is_error = Some(
                                result.get("status").and_then(Value::as_str) != Some("success")
                                    || result
                                        .pointer("/value/isError")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(false),
                            );
                            out.push(e);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    flush(&mut out, &mut thinking);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path) -> std::path::PathBuf {
        let db = dir.join("sessions.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE sessions (id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT '', session_type TEXT NOT NULL DEFAULT 'user',
                 working_dir TEXT NOT NULL, created_at TIMESTAMP, updated_at TIMESTAMP, provider_name TEXT, model_config_json TEXT,
                 parent_session_id TEXT);
               CREATE TABLE messages (id INTEGER PRIMARY KEY AUTOINCREMENT, message_id TEXT, session_id TEXT NOT NULL, role TEXT NOT NULL,
                 content_json TEXT NOT NULL, created_timestamp INTEGER NOT NULL);
               INSERT INTO sessions VALUES ('20261101_1', 'fix', 'user', '/work/app', '2026-11-01 10:00:00', '2026-11-01 10:05:00',
                 'local', '{"model_name":"m-7"}', NULL);
               INSERT INTO messages (message_id, session_id, role, content_json, created_timestamp) VALUES
                 ('a', '20261101_1', 'user', '[{"type":"text","text":"run the tests"}]', 1793527200),
                 ('b', '20261101_1', 'assistant', '[{"type":"thinking","thinking":"Run "},{"type":"thinking","thinking":"them."},
                   {"type":"toolRequest","id":"t1","toolCall":{"status":"success","value":{"name":"shell","arguments":{"command":"cargo test"}}}}]', 1793527201),
                 ('c', '20261101_1', 'user', '[{"type":"toolResponse","id":"t1","toolResult":{"status":"success","value":{"content":[{"type":"text","text":"1 failed"}]}}}]', 1793527202),
                 ('d', '20261101_1', 'assistant', '[{"type":"text","text":"One test fails."}]', 1793527203);"#,
        )
        .unwrap();
        db
    }

    #[test]
    fn exports_sessions_and_parses_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path());
        let sessions = list_sessions(&db).unwrap();
        assert_eq!(sessions.len(), 1);
        let export = export_session(&db, "20261101_1").unwrap();
        assert_eq!(export.working_dir.as_deref(), Some("/work/app"));
        let p = parse(&export.bytes).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("20261101_1"));
        assert_eq!(p.models, vec!["m-7"]);
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::User,
                EventKind::Reasoning,
                EventKind::ToolCall,
                EventKind::ToolResult,
                EventKind::Assistant
            ]
        );
        assert_eq!(p.events[1].text.as_deref(), Some("Run them."));
        assert_eq!(p.events[2].input.as_ref().unwrap()["command"], "cargo test");
        assert_eq!(p.events[3].text.as_deref(), Some("1 failed"));
        assert_eq!(p.events[3].is_error, Some(false));
        assert_eq!(p.events[0].ts.as_deref(), Some("2026-11-01T10:00:00.000Z"));

        // A tool that failed inside a successful call, and one whose call failed.
        let failed = parse(br#"{"row":"message","role":"user","created":-9223372036854775808,"content":[{"type":"toolResponse","id":"x","toolResult":{"status":"success","value":{"isError":true,"content":[{"type":"text","text":"boom"}]}}},{"type":"toolResponse","id":"y","toolResult":{"status":"error","error":"no such tool"}}]}"#)
        .unwrap();
        assert_eq!(failed.events[0].is_error, Some(true));
        assert_eq!(failed.events[1].is_error, Some(true));
        assert_eq!(
            failed.events[0].ts, None,
            "an impossible time is dropped, not a panic"
        );

        // A message edited in place changes the fingerprint.
        let before = list_sessions(&db).unwrap()[0].fingerprint();
        Connection::open(&db)
            .unwrap()
            .execute(
                r#"UPDATE messages SET content_json = '[{"type":"text","text":"run all tests"}]' WHERE message_id = 'a'"#,
                [],
            )
            .unwrap();
        assert_ne!(list_sessions(&db).unwrap()[0].fingerprint(), before);
        // NULL and an empty value are different exports, so different fingerprints.
        let before = list_sessions(&db).unwrap()[0].fingerprint();
        Connection::open(&db)
            .unwrap()
            .execute("UPDATE sessions SET parent_session_id = ''", [])
            .unwrap();
        assert_ne!(list_sessions(&db).unwrap()[0].fingerprint(), before);
        let sessions = list_sessions(&db).unwrap();
        let export = export_session(&db, "20261101_1").unwrap();

        // A new message appends to the export.
        Connection::open(&db)
            .unwrap()
            .execute(
                "INSERT INTO messages (message_id, session_id, role, content_json, created_timestamp)
                 VALUES ('e', '20261101_1', 'user', '[{\"type\":\"text\",\"text\":\"thanks\"}]', 1793527300)",
                [],
            )
            .unwrap();
        let grown = export_session(&db, "20261101_1").unwrap().bytes;
        assert!(grown.starts_with(&export.bytes));
        assert_ne!(
            list_sessions(&db).unwrap()[0].fingerprint(),
            sessions[0].fingerprint()
        );
    }
}
