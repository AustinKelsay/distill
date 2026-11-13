//! OpenCode keeps every session in one SQLite database. The sweep exports each session as JSONL
//! rows (the session, then messages and parts in creation order), and that export is what the
//! archive stores. Rows are ordered by creation so a growing session appends.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

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

pub fn list_sessions(db: &Path) -> Result<Vec<Candidate>> {
    let conn = open(db)?;
    let mut stmt = conn.prepare("SELECT id, time_updated FROM session ORDER BY id")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (session_id, updated) = row?;
        out.push(Candidate::OpenCodeSession {
            db: db.to_path_buf(),
            session_id,
            updated,
        });
    }
    Ok(out)
}

pub struct Export {
    pub bytes: Vec<u8>,
    pub directory: Option<String>,
}

pub fn export_session(db: &Path, session_id: &str) -> Result<Export> {
    let conn = open(db)?;
    let session: Value = conn.query_row(
        "SELECT id, parent_id, directory, title, version, model, agent, time_created, time_updated FROM session WHERE id = ?1",
        [session_id],
        |r| {
            Ok(json!({
                "row": "session",
                "id": r.get::<_, String>(0)?,
                "parent_id": r.get::<_, Option<String>>(1)?,
                "directory": r.get::<_, String>(2)?,
                "title": r.get::<_, String>(3)?,
                "version": r.get::<_, String>(4)?,
                "model": r.get::<_, Option<String>>(5)?,
                "agent": r.get::<_, Option<String>>(6)?,
                "time_created": r.get::<_, i64>(7)?,
            }))
        },
    )?;
    let directory = session
        .get("directory")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut lines = vec![session];
    let mut stmt = conn.prepare(
        "SELECT 'message', id, id, time_created, data FROM message WHERE session_id = ?1
         UNION ALL
         SELECT 'part', id, message_id, time_created, data FROM part WHERE session_id = ?1
         ORDER BY 4, 1, 2",
    )?;
    let rows = stmt.query_map([session_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (kind, id, message_id, created, data) = row?;
        let data: Value = serde_json::from_str(&data).unwrap_or(Value::String(data));
        lines.push(json!({"row": kind, "id": id, "message_id": message_id, "time_created": created, "data": data}));
    }
    let mut bytes = Vec::new();
    for l in &lines {
        serde_json::to_writer(&mut bytes, l)?;
        bytes.push(b'\n');
    }
    Ok(Export { bytes, directory })
}

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let mut roles = std::collections::HashMap::<String, (EventKind, Option<String>)>::new();
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let at = v
            .get("time_created")
            .and_then(Value::as_i64)
            .map(millis_to_rfc3339);
        match v.get("row").and_then(Value::as_str).unwrap_or("") {
            "session" => {
                out.session_id = v.get("id").and_then(Value::as_str).map(str::to_string);
                out.cwd = v
                    .get("directory")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                out.title = v.get("title").and_then(Value::as_str).map(str::to_string);
                out.harness_version = v.get("version").and_then(Value::as_str).map(str::to_string);
                out.parent_session = v
                    .get("parent_id")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            "message" => {
                let d = &v["data"];
                let role = match d.get("role").and_then(Value::as_str) {
                    Some("assistant") => EventKind::Assistant,
                    _ => EventKind::User,
                };
                let model = d.get("modelID").and_then(Value::as_str).map(str::to_string);
                if let Some(m) = &model {
                    out.add_model(m);
                }
                let id = v
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                roles.insert(id, (role, model));
            }
            "part" => {
                let d = &v["data"];
                let mid = v.get("message_id").and_then(Value::as_str).unwrap_or("");
                let (role, model) = roles
                    .get(mid)
                    .cloned()
                    .unwrap_or((EventKind::Assistant, None));
                match d.get("type").and_then(Value::as_str).unwrap_or("") {
                    "text" => {
                        let mut e =
                            Event::text(role, d.get("text").and_then(Value::as_str).unwrap_or(""))
                                .at(at);
                        if role == EventKind::Assistant {
                            e.model = model;
                        }
                        out.push(e);
                    }
                    "reasoning" => out.push(
                        Event::text(
                            EventKind::Reasoning,
                            d.get("text").and_then(Value::as_str).unwrap_or(""),
                        )
                        .at(at),
                    ),
                    "tool" => {
                        let state = &d["state"];
                        let call_id = d.get("callID").and_then(Value::as_str).map(str::to_string);
                        let tool = d.get("tool").and_then(Value::as_str).map(str::to_string);
                        let mut call = Event::new(EventKind::ToolCall).at(at.clone());
                        call.tool = tool.clone();
                        call.call_id = call_id.clone();
                        call.input = state.get("input").cloned();
                        call.model = model;
                        out.push(call);
                        let status = state.get("status").and_then(Value::as_str).unwrap_or("");
                        if matches!(status, "completed" | "error") {
                            let mut res = Event::new(EventKind::ToolResult).at(at);
                            res.tool = tool;
                            res.call_id = call_id;
                            res.is_error = Some(status == "error");
                            res.text =
                                state.get("output").or_else(|| state.get("error")).map(|o| {
                                    o.as_str()
                                        .map(str::to_string)
                                        .unwrap_or_else(|| o.to_string())
                                });
                            out.push(res);
                        }
                    }
                    "file" => out.push(
                        Event::text(
                            role,
                            format!(
                                "[file {}]",
                                d.get("filename")
                                    .or_else(|| d.get("url"))
                                    .and_then(Value::as_str)
                                    .unwrap_or("")
                            ),
                        )
                        .at(at),
                    ),
                    _ => {}
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_and_parses_a_session_from_sqlite() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("opencode.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT, title TEXT, version TEXT, model TEXT, agent TEXT, time_created INTEGER, time_updated INTEGER);
               CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
               CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
               INSERT INTO session VALUES ('ses1', NULL, '/r', 'Title', '1.2.3', NULL, NULL, 1000, 5000);
               INSERT INTO message VALUES ('m1', 'ses1', 1000, 1000, '{"role":"user"}');
               INSERT INTO part VALUES ('p1', 'm1', 'ses1', 1001, 1001, '{"type":"text","text":"run ls"}');
               INSERT INTO message VALUES ('m2', 'ses1', 2000, 2000, '{"role":"assistant","modelID":"kimi-k3"}');
               INSERT INTO part VALUES ('p2', 'm2', 'ses1', 2001, 2001, '{"type":"tool","callID":"c1","tool":"bash","state":{"status":"completed","input":{"command":"ls"},"output":"a"}}');"#,
        )
        .unwrap();
        drop(conn);
        let sessions = list_sessions(&db).unwrap();
        assert_eq!(sessions.len(), 1);
        let export = export_session(&db, "ses1").unwrap();
        assert_eq!(export.directory.as_deref(), Some("/r"));
        let p = parse(&export.bytes).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("ses1"));
        assert_eq!(p.models, vec!["kimi-k3"]);
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![EventKind::User, EventKind::ToolCall, EventKind::ToolResult]
        );
        assert_eq!(p.events[2].text.as_deref(), Some("a"));
    }
}
