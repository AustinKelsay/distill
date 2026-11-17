//! OpenCode keeps every session in one SQLite database. The sweep exports each session as JSONL
//! rows (the session, then messages and parts in creation order), and that export is what the
//! archive stores. Rows are ordered by creation so a growing session appends.
//!
//! A tool output too long to keep inline is spilled: OpenCode writes the whole of it to a file in
//! `tool-output/` beside the database, keeps a preview in the tool part, and deletes the file
//! after a fixed 7 days. The sweep captures those files on their own ([`list_spills`]), and the
//! parse step puts a spilled output back in its tool result ([`parse_with_spills`]).
//!
//! OpenCode 2 keeps new sessions in other tables: `session_v2`, `session_message` (one row per
//! message, its `data` the message without `type` and `id`; an assistant message holds its text,
//! reasoning and tool calls in `data.content`, or in `session_part` rows when OpenCode's
//! `part_rows` flag is on). Its sessions that the v1 tables lack are exported at the same
//! `<db>#session=<id>` origin, as `message_v2` and `part_v2` rows; a session in both sets of
//! tables is exported from v1 only, so what is already archived never changes.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde_json::{Value, json};

use super::blocks::millis_to_rfc3339;
use crate::harness::Candidate;
use crate::model::{Event, EventKind, Parsed};

fn open(db: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open {}", db.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(conn)
}

fn has_table(conn: &Connection, name: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [name],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The sessions in the database: those in the v1 tables, then those only in OpenCode 2's. A
/// database without either set of tables (older or newer OpenCode) just has fewer.
pub fn list_sessions(db: &Path) -> Result<Vec<Candidate>> {
    let conn = open(db)?;
    let mut out = Vec::new();
    let mut push = |rows: Vec<(String, i64)>| {
        for (session_id, updated) in rows {
            out.push(Candidate::OpenCodeSession {
                db: db.to_path_buf(),
                session_id,
                updated,
            });
        }
    };
    let v1 = has_table(&conn, "session")?;
    if v1 {
        let mut stmt = conn.prepare("SELECT id, time_updated FROM session ORDER BY id")?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        push(rows);
    }
    if has_table(&conn, "session_v2")? && has_table(&conn, "session_message")? {
        // A session changes when its row does or when a message does, whichever is later.
        let sql = format!(
            "SELECT s.id, MAX(s.time_updated, COALESCE((SELECT MAX(m.time_updated) FROM session_message m WHERE m.session_id = s.id), 0))
             FROM session_v2 s {} ORDER BY s.id",
            if v1 { "WHERE s.id NOT IN (SELECT id FROM session)" } else { "" }
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        push(rows);
    }
    Ok(out)
}

/// The folder OpenCode spills long tool outputs to: `tool-output/` in its data folder, the folder
/// that holds `opencode.db` (`ToolOutput.DIRECTORY` beside `databasePath(data)` in OpenCode's
/// source; `TRUNCATION_DIR` in v1).
pub const SPILL_DIR: &str = "tool-output";

/// How many days OpenCode keeps a spilled output: the file goes once its mtime is older
/// (`ToolOutput.RETENTION`; `RETENTION` in v1's `Truncate`). Not configurable.
pub const SPILL_RETENTION_DAYS: u64 = 7;

/// The database an OpenCode root names: the file itself, or `opencode.db` in a folder.
pub fn database(root: &Path) -> PathBuf {
    if root.is_dir() {
        root.join("opencode.db")
    } else {
        root.to_path_buf()
    }
}

/// Where the database at `db` spills its tool outputs.
pub fn spill_dir(db: &Path) -> PathBuf {
    db.parent().unwrap_or(Path::new("")).join(SPILL_DIR)
}

/// The file name of a spilled output, from a path naming one. OpenCode names them `tool_<id>`
/// and deletes only names starting `tool_`.
pub fn spill_name(path: &str) -> Option<&str> {
    let name = path.rsplit(['/', '\\']).next()?;
    (name.len() > "tool_".len() && name.starts_with("tool_")).then_some(name)
}

/// The spilled outputs beside the database at `db`, each captured as a file of its own.
pub fn list_spills(db: &Path) -> Result<Vec<Candidate>> {
    let dir = spill_dir(db);
    // The folder is optional; any other failure to read it leaves outputs uncaptured, so it is
    // an error the sweep reports.
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e).with_context(|| format!("read {}", dir.display())),
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("read {}", dir.display()))?;
        if entry.file_name().to_str().and_then(spill_name).is_none() {
            continue;
        }
        let meta = entry
            .metadata()
            .with_context(|| format!("read {}", entry.path().display()))?;
        if !meta.is_file() {
            continue;
        }
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos() as i128)
            .unwrap_or(0);
        out.push(Candidate::OpenCodeSpill {
            db: db.to_path_buf(),
            path: entry.path(),
            size: meta.len(),
            mtime_ns,
        });
    }
    Ok(out)
}

/// The origin the spilled output `name` is captured at, for a session captured from
/// `session_origin` (`<db>#session=<id>`): the file of that name in the database's spill folder.
pub fn spill_origin(session_origin: &str, name: &str) -> Option<String> {
    let (db, _) = session_origin.split_once("#session=")?;
    let name = spill_name(name)?;
    Some(
        spill_dir(Path::new(db))
            .join(name)
            .to_string_lossy()
            .into_owned(),
    )
}

/// Where a tool part's state says its full output was spilled: `metadata.outputPath`, which
/// OpenCode sets when it truncates a tool's output (in v1, `Tool.wrap` after `Truncate.output`,
/// stored in the part by `SessionProcessor.completeToolCall`; in v2, `ToolOutput.truncate`).
fn spill_path(state: &Value) -> Option<&str> {
    state
        .get("metadata")?
        .get("outputPath")?
        .as_str()
        .filter(|p| spill_name(p).is_some())
}

/// The projects of the sessions whose tool parts name the spilled output `name`, so privacy
/// exclusions can be checked for it. Empty when no session names it (yet).
pub fn spill_projects(db: &Path, name: &str) -> Result<Vec<String>> {
    let conn = open(db)?;
    let mut out = Vec::new();
    // `instr`, not `LIKE`, whose wildcards include the `_` in every spill's name. Each row it
    // finds is checked for the name in the right place.
    if has_table(&conn, "part")? && has_table(&conn, "session")? {
        let mut stmt = conn.prepare(
            "SELECT p.data, s.directory FROM part p JOIN session s ON s.id = p.session_id
             WHERE instr(p.data, ?1) > 0",
        )?;
        let rows = stmt.query_map([name], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (data, directory) = row?;
            let Ok(v) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            if spill_path(&v["state"]).and_then(spill_name) == Some(name)
                && !out.contains(&directory)
            {
                out.push(directory);
            }
        }
    }
    if has_table(&conn, "session_v2")? {
        let mut sources = Vec::new();
        if has_table(&conn, "session_message")? {
            sources.push("SELECT m.data, s.directory FROM session_message m JOIN session_v2 s ON s.id = m.session_id WHERE instr(m.data, ?1) > 0");
        }
        if has_table(&conn, "session_part")? {
            sources.push("SELECT p.data, s.directory FROM session_part p JOIN session_v2 s ON s.id = p.session_id WHERE instr(p.data, ?1) > 0");
        }
        for sql in sources {
            let mut stmt = conn.prepare(sql)?;
            let rows = stmt.query_map([name], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            for row in rows {
                let (data, directory) = row?;
                let Ok(v) = serde_json::from_str::<Value>(&data) else {
                    continue;
                };
                // A message holds its tools in `content`; a `session_part` row is one tool.
                let named = match v.get("content").and_then(Value::as_array) {
                    Some(items) => items.iter().any(|i| spill_in(i, name)),
                    None => spill_in(&v, name),
                };
                if named && !out.contains(&directory) {
                    out.push(directory);
                }
            }
        }
    }
    Ok(out)
}

/// Whether an OpenCode 2 tool item names the spilled output `name`.
fn spill_in(item: &Value, name: &str) -> bool {
    spill_path(&item["state"]).and_then(spill_name) == Some(name)
}

pub struct Export {
    pub bytes: Vec<u8>,
    pub directory: Option<String>,
}

pub fn export_session(db: &Path, session_id: &str) -> Result<Export> {
    let conn = open(db)?;
    // A session in both sets of tables is exported from v1, as it always was.
    let in_v1 = has_table(&conn, "session")?
        && conn
            .query_row("SELECT 1 FROM session WHERE id = ?1", [session_id], |_| {
                Ok(())
            })
            .optional()?
            .is_some();
    if !in_v1 {
        return export_session_v2(&conn, session_id);
    }
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

/// Export a session that only OpenCode 2's tables hold: the session, then its messages in `seq`
/// order, each followed by its `session_part` rows (when the table exists) in ordinal order.
fn export_session_v2(conn: &Connection, session_id: &str) -> Result<Export> {
    let session: Value = conn.query_row(
        "SELECT id, parent_id, directory, title, version, model, agent, time_created FROM session_v2 WHERE id = ?1",
        [session_id],
        |r| {
            // `model` is JSON ({id, providerID, variant}); keep the model's id.
            let model = r
                .get::<_, Option<String>>(5)?
                .and_then(|m| serde_json::from_str::<Value>(&m).ok())
                .and_then(|m| m.get("id").and_then(Value::as_str).map(str::to_string));
            Ok(json!({
                "row": "session",
                "id": r.get::<_, String>(0)?,
                "parent_id": r.get::<_, Option<String>>(1)?,
                "directory": r.get::<_, String>(2)?,
                "title": r.get::<_, Option<String>>(3)?,
                "version": r.get::<_, String>(4)?,
                "model": model,
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
    let mut parts: std::collections::HashMap<String, Vec<Value>> = Default::default();
    if has_table(conn, "session_part")? {
        let mut stmt = conn.prepare(
            "SELECT message_id, ordinal, type, call_id, time_created, data FROM session_part
             WHERE session_id = ?1 ORDER BY message_id, ordinal",
        )?;
        let rows = stmt.query_map([session_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, String>(5)?,
            ))
        })?;
        for row in rows {
            let (message_id, ordinal, kind, call_id, created, data) = row?;
            let data: Value = serde_json::from_str(&data).unwrap_or(Value::String(data));
            parts.entry(message_id.clone()).or_default().push(json!({
                "row": "part_v2", "message_id": message_id, "ordinal": ordinal, "type": kind,
                "call_id": call_id, "time_created": created, "data": data,
            }));
        }
    }
    let mut stmt = conn.prepare(
        "SELECT id, seq, type, time_created, data FROM session_message WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map([session_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
        ))
    })?;
    for row in rows {
        let (id, seq, kind, created, data) = row?;
        let data: Value = serde_json::from_str(&data).unwrap_or(Value::String(data));
        let parts = parts.remove(&id).unwrap_or_default();
        lines.push(json!({"row": "message_v2", "id": id, "seq": seq, "type": kind, "time_created": created, "data": data}));
        lines.extend(parts);
    }
    let mut bytes = Vec::new();
    for l in &lines {
        serde_json::to_writer(&mut bytes, l)?;
        bytes.push(b'\n');
    }
    Ok(Export { bytes, directory })
}

/// Parse an export, leaving each spilled tool output as the database has it: a preview and a
/// note saying where the rest went.
pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    Ok(parse_with_spills(bytes, |_| Ok(None))?.0)
}

/// Parse an export, giving each tool result whose output was spilled the full output, which
/// `spilled` returns for a spill's file name when the archive holds it. Also returns the names
/// of the spills the session names that `spilled` did not have.
pub fn parse_with_spills(
    bytes: &[u8],
    mut spilled: impl FnMut(&str) -> Result<Option<Vec<u8>>>,
) -> Result<(Parsed, Vec<String>)> {
    let mut missing: Vec<String> = Vec::new();
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
                            if let Some(name) = spill_path(state).and_then(spill_name) {
                                match spilled(name)? {
                                    Some(full) => {
                                        res.text = Some(String::from_utf8_lossy(&full).into_owned())
                                    }
                                    None if !missing.iter().any(|m| m == name) => {
                                        missing.push(name.to_string())
                                    }
                                    None => {}
                                }
                            }
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
            // OpenCode 2 (`session_message`). The message's own `type` says what it is; the
            // `data` holds the rest. An assistant message's content is inline, or in the
            // `part_v2` rows that follow it.
            "message_v2" => {
                let d = &v["data"];
                let id = v.get("id").and_then(Value::as_str).unwrap_or("");
                let text_of = |d: &Value| {
                    d.get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string()
                };
                match v.get("type").and_then(Value::as_str).unwrap_or("") {
                    "user" => {
                        out.push(Event::text(EventKind::User, text_of(d)).at(at.clone()));
                        for f in d
                            .get("files")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            out.push(Event::text(EventKind::User, file_note(f)).at(at.clone()));
                        }
                    }
                    "assistant" => {
                        let model = d
                            .pointer("/model/id")
                            .and_then(Value::as_str)
                            .map(str::to_string);
                        if let Some(m) = &model {
                            out.add_model(m);
                        }
                        roles.insert(id.to_string(), (EventKind::Assistant, model.clone()));
                        for item in d
                            .get("content")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            push_v2_item(&mut out, &mut missing, &mut spilled, item, &model, &at)?;
                        }
                    }
                    // Text OpenCode gave the model: the session's own, not the owner's.
                    "system" | "synthetic" | "skill" => {
                        out.push(Event::text(EventKind::System, text_of(d)).at(at))
                    }
                    // A compaction's summary replaces earlier turns in the model's context.
                    // Only a completed one has a summary worth keeping.
                    "compaction"
                        if d.get("status").and_then(Value::as_str) == Some("completed") =>
                    {
                        out.push(
                            Event::text(
                                EventKind::System,
                                d.get("summary").and_then(Value::as_str).unwrap_or(""),
                            )
                            .at(at),
                        )
                    }
                    // A shell command the owner ran: a call and, once it ends, its result.
                    "shell" => {
                        let mut call = Event::new(EventKind::ToolCall).at(at.clone());
                        call.tool = Some("shell".into());
                        call.call_id = d.get("shellID").and_then(Value::as_str).map(str::to_string);
                        call.input = Some(json!({"command": d.get("command")}));
                        let (call_id, status) = (
                            call.call_id.clone(),
                            d.get("status").and_then(Value::as_str),
                        );
                        out.push(call);
                        if status != Some("running")
                            && let Some(o) = d.get("output")
                        {
                            let mut res = Event::new(EventKind::ToolResult).at(at);
                            res.tool = Some("shell".into());
                            res.call_id = call_id;
                            res.text = Some(
                                o.as_str()
                                    .map(str::to_string)
                                    .unwrap_or_else(|| o.to_string()),
                            );
                            res.is_error = d.get("exit").and_then(Value::as_i64).map(|c| c != 0);
                            out.push(res);
                        }
                    }
                    // Ignored, with nothing to say in a transcript: `idle` (a turn's end marker),
                    // `model-switched`, `agent-switched` and `location-switched` (settings
                    // changes), a running or failed `compaction`, and any type OpenCode adds.
                    _ => {}
                }
            }
            "part_v2" => {
                let mid = v.get("message_id").and_then(Value::as_str).unwrap_or("");
                let model = roles.get(mid).and_then(|(_, m)| m.clone());
                // The row's `type` column is the part's type; `data` may not repeat it.
                let mut item = v["data"].clone();
                if let (Some(o), Some(t)) = (item.as_object_mut(), v.get("type")) {
                    o.entry("type").or_insert_with(|| t.clone());
                }
                push_v2_item(&mut out, &mut missing, &mut spilled, &item, &model, &at)?;
            }
            _ => {}
        }
    }
    Ok((out, missing))
}

/// A note naming an attached file, never its bytes: a file's `name`, else its `uri` unless that
/// holds the data itself.
fn file_note(f: &Value) -> String {
    let name = f
        .get("name")
        .or_else(|| f.get("filename"))
        .and_then(Value::as_str)
        .or_else(|| {
            f.get("uri")
                .and_then(Value::as_str)
                .filter(|u| !u.starts_with("data:"))
        })
        .unwrap_or("");
    format!("[file {name}]")
}

/// One entry of an OpenCode 2 assistant message's content: text, reasoning, or a tool (a call, and
/// its result once the tool completed or failed).
fn push_v2_item(
    out: &mut Parsed,
    missing: &mut Vec<String>,
    spilled: &mut impl FnMut(&str) -> Result<Option<Vec<u8>>>,
    item: &Value,
    model: &Option<String>,
    at: &Option<String>,
) -> Result<()> {
    let text = item.get("text").and_then(Value::as_str).unwrap_or("");
    match item.get("type").and_then(Value::as_str).unwrap_or("") {
        "text" => {
            let mut e = Event::text(EventKind::Assistant, text).at(at.clone());
            e.model = model.clone();
            out.push(e);
        }
        "reasoning" => out.push(Event::text(EventKind::Reasoning, text).at(at.clone())),
        "tool" => {
            let state = &item["state"];
            let tool = item.get("name").and_then(Value::as_str).map(str::to_string);
            let call_id = item.get("id").and_then(Value::as_str).map(str::to_string);
            let mut call = Event::new(EventKind::ToolCall).at(at.clone());
            call.tool = tool.clone();
            call.call_id = call_id.clone();
            call.input = state.get("input").cloned();
            call.model = model.clone();
            out.push(call);
            let status = state.get("status").and_then(Value::as_str).unwrap_or("");
            // A streaming or running tool has no result yet.
            if matches!(status, "completed" | "error") {
                let mut res = Event::new(EventKind::ToolResult).at(at.clone());
                res.tool = tool;
                res.call_id = call_id;
                res.is_error = Some(status == "error");
                let mut pieces: Vec<String> = state
                    .get("content")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .map(|c| match c.get("text").and_then(Value::as_str) {
                        Some(t) => t.to_string(),
                        None => file_note(c),
                    })
                    .collect();
                if let Some(e) = state.get("error") {
                    pieces.push(
                        e.get("message")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| e.to_string()),
                    );
                }
                res.text = Some(pieces.join("\n"));
                if let Some(name) = spill_path(state).and_then(spill_name) {
                    match spilled(name)? {
                        Some(full) => res.text = Some(String::from_utf8_lossy(&full).into_owned()),
                        None if !missing.iter().any(|m| m == name) => {
                            missing.push(name.to_string())
                        }
                        None => {}
                    }
                }
                out.push(res);
            }
        }
        // Ignored: any content type OpenCode adds.
        _ => {}
    }
    Ok(())
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

    use crate::archive::Archive;
    use crate::config::Config;
    use crate::harness::Harness;
    use crate::home::Home;

    /// A synthetic OpenCode data folder at `data`: a database holding, for each
    /// `(session, project, spill)`, a session whose one bash call's output was spilled to
    /// `tool-output/<spill>`, as OpenCode records it. Returns the database.
    fn opencode_data(data: &Path, sessions: &[(&str, &str, &str)]) -> PathBuf {
        std::fs::create_dir_all(data.join(SPILL_DIR)).unwrap();
        let db = data.join("opencode.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE session (id TEXT PRIMARY KEY, parent_id TEXT, directory TEXT, title TEXT, version TEXT, model TEXT, agent TEXT, time_created INTEGER, time_updated INTEGER);
               CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
               CREATE TABLE part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);"#,
        )
        .unwrap();
        for (id, project, spill) in sessions {
            let path = data.join(SPILL_DIR).join(spill);
            let output = format!(
                "line 1\n\n...2 lines truncated...\n\nThe tool call succeeded but the output was truncated. Full output saved to: {}",
                path.display()
            );
            let tool = json!({"type": "tool", "callID": format!("call-{id}"), "tool": "bash",
                "state": {"status": "completed", "input": {"command": "seq 3"}, "output": output,
                    "title": "seq 3", "metadata": {"truncated": true, "outputPath": path.display().to_string()}}});
            conn.execute(
                "INSERT INTO session VALUES (?1, NULL, ?2, 'Title', '1.19.6', NULL, NULL, 1000, 5000)",
                [id, project],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO message VALUES (?1 || '-m1', ?1, 1000, 1000, '{\"role\":\"user\"}')",
                [id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO part VALUES (?1 || '-p1', ?1 || '-m1', ?1, 1001, 1001, '{\"type\":\"text\",\"text\":\"count to three\"}')",
                [id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO message VALUES (?1 || '-m2', ?1, 2000, 2000, '{\"role\":\"assistant\",\"modelID\":\"m-1\"}')",
                [id],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO part VALUES (?1 || '-p2', ?1 || '-m2', ?1, 2001, 2001, ?2)",
                [id.to_string(), tool.to_string()],
            )
            .unwrap();
        }
        db
    }

    fn full_output(id: &str) -> String {
        format!("line 1\nline 2 of {id}\nline 3\n")
    }

    fn opencode_config(db: &Path) -> Config {
        let mut config = Config {
            harnesses: vec![],
            ..Default::default()
        };
        let c = config.harness_mut(Harness::OpenCode);
        c.enabled = true;
        c.roots = vec![db.to_path_buf()];
        config
    }

    fn tool_result(home: &Home, id: &str) -> String {
        let meta = crate::normalize::find(home, Harness::OpenCode, id)
            .unwrap()
            .unwrap();
        crate::normalize::read_events(home, &meta)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == EventKind::ToolResult)
            .and_then(|e| e.text)
            .unwrap()
    }

    #[test]
    fn a_spilled_output_is_captured_and_outlives_the_file_opencode_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("opencode");
        let spill = "tool_0a1b2c3d4e5fSyntheticAaa1";
        let db = opencode_data(&data, &[("ses1", "/w/app", spill)]);
        let spill_path = data.join(SPILL_DIR).join(spill);
        std::fs::write(&spill_path, full_output("ses1")).unwrap();
        // Not a name OpenCode spills to, so not captured.
        std::fs::write(data.join(SPILL_DIR).join("notes.txt"), "other").unwrap();
        let home = Home::new(dir.path().join("home"));
        let config = opencode_config(&db);

        let r = crate::sweep::sweep(&home, &config, None).unwrap();
        assert_eq!((r.new, r.errors.len()), (2, 0), "{r:?}"); // the session and its spill
        let origin = spill_path.to_string_lossy().into_owned();
        let rec = Archive::open(&home).unwrap().latest()[&origin].clone();
        assert_eq!(rec.harness, Harness::OpenCode);
        assert!(!Harness::OpenCode.is_primary(&origin));
        let n = crate::normalize::normalize(&home, false).unwrap();
        assert_eq!((n.sessions, n.sidecars), (1, 1), "{n:?}");
        assert_eq!(tool_result(&home, "ses1"), full_output("ses1"));

        // OpenCode deletes the file after a week; the capture stays, and so does the output,
        // even when every session is parsed again from the archive.
        std::fs::remove_file(&spill_path).unwrap();
        let r = crate::sweep::sweep(&home, &config, None).unwrap();
        assert_eq!((r.new, r.errors.len()), (0, 0), "{r:?}");
        let archive = Archive::open(&home).unwrap();
        assert_eq!(archive.latest()[&origin].capture_hash, rec.capture_hash);
        assert_eq!(
            archive.read_blob(&rec.capture_hash).unwrap(),
            full_output("ses1").as_bytes()
        );
        assert!(archive.verify().unwrap().ok());
        crate::normalize::normalize(&home, true).unwrap();
        assert_eq!(tool_result(&home, "ses1"), full_output("ses1"));
    }

    #[test]
    fn a_session_parsed_before_its_spill_was_captured_gets_the_full_output_later() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("opencode");
        let spill = "tool_0a1b2c3d4e5fSyntheticBbb2";
        let db = opencode_data(&data, &[("ses1", "/w/app", spill)]);
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        let session_origin = format!("{}#session=ses1", db.to_string_lossy());
        let export = export_session(&db, "ses1").unwrap();
        Archive::open(&home)
            .unwrap()
            .put(
                Harness::OpenCode,
                &session_origin,
                "updated:5000",
                &export.bytes[..],
            )
            .unwrap();
        crate::normalize::normalize(&home, false).unwrap();
        // Without the spill, the result is the preview OpenCode kept.
        assert!(
            tool_result(&home, "ses1").contains("output was truncated"),
            "preview kept"
        );
        assert_eq!(
            crate::normalize::normalize(&home, false).unwrap().skipped,
            1
        );

        let spill_origin = spill_origin(&session_origin, spill).unwrap();
        assert_eq!(
            spill_origin,
            data.join(SPILL_DIR).join(spill).to_string_lossy()
        );
        Archive::open(&home)
            .unwrap()
            .put(
                Harness::OpenCode,
                &spill_origin,
                "1:1",
                full_output("ses1").as_bytes(),
            )
            .unwrap();
        let n = crate::normalize::normalize(&home, false).unwrap();
        assert_eq!(
            n.normalized, 1,
            "parsed again once its spill arrived: {n:?}"
        );
        assert_eq!(tool_result(&home, "ses1"), full_output("ses1"));
        assert_eq!(
            crate::normalize::normalize(&home, false).unwrap().skipped,
            1
        );
    }

    #[test]
    fn a_secret_in_a_spill_that_arrives_later_is_found_before_any_model_reads_it() {
        use crate::read::{Range, Read};
        use crate::runtime::{FakeRuntime, Stored};
        use crate::trust::Reader;
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("opencode");
        let spill = "tool_0a1b2c3d4e5fSyntheticCcc3";
        let db = opencode_data(&data, &[("ses1", "/w/app", spill)]);
        let home = Home::new(dir.path().join("home"));
        home.ensure().unwrap();
        let config = Config::default();
        config.save(&home.config_path()).unwrap();
        let session_origin = format!("{}#session=ses1", db.to_string_lossy());
        let export = export_session(&db, "ses1").unwrap();
        let mut archive = Archive::open(&home).unwrap();
        archive
            .put(
                Harness::OpenCode,
                &session_origin,
                "updated:5000",
                &export.bytes[..],
            )
            .unwrap();
        drop(archive);
        crate::normalize::normalize(&home, false).unwrap();
        crate::findings::scan_all(&home, &config).unwrap();
        let facts = crate::normalize::find(&home, Harness::OpenCode, "ses1")
            .unwrap()
            .unwrap();
        let key = crate::findings::FingerprintKey::open(&home).unwrap();
        assert!(
            crate::findings::load(&home, &facts)
                .unwrap()
                .is_current(&facts, &key, &config)
        );

        // The spill holds a secret the preview didn't; its capture changes neither the session's
        // capture hash nor its parser version.
        let secret = ["sk-", "ant-", "api03-", &"Vq3".repeat(9)].concat();
        Archive::open(&home)
            .unwrap()
            .put(
                Harness::OpenCode,
                &spill_origin(&session_origin, spill).unwrap(),
                "1:1",
                format!("deploy key {secret}\n").as_bytes(),
            )
            .unwrap();
        crate::normalize::normalize(&home, false).unwrap();
        let facts = crate::normalize::find(&home, Harness::OpenCode, "ses1")
            .unwrap()
            .unwrap();
        assert_eq!(facts.raw.attached.len(), 1);
        assert_ne!(facts.raw.source(), facts.raw.sha256);
        assert!(
            !crate::findings::load(&home, &facts)
                .unwrap()
                .is_current(&facts, &key, &config),
            "the old findings were made from other events"
        );

        let runtime = FakeRuntime::running()
            .with_model("m", Stored::Here)
            .runtime();
        let Read::Text(text) = crate::read::read(
            &home,
            &runtime,
            &Reader::model(Some("ollama"), Some("m")),
            "opencode/ses1",
            Range::default(),
        )
        .unwrap() else {
            panic!("a model is served text")
        };
        assert!(!text.contains(&secret), "the secret is masked");
        assert!(text.contains("[secret:"), "{text}");
    }

    #[test]
    fn a_spill_is_excluded_with_the_sessions_that_name_it() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("opencode");
        let (kept, secret, unnamed) = (
            "tool_0a1b2c3d4e5fSyntheticKept",
            "tool_0a1b2c3d4e5fSyntheticSecr",
            "tool_0a1b2c3d4e5fSyntheticNone",
        );
        let db = opencode_data(
            &data,
            &[
                ("ses1", "/w/app", kept),
                ("ses2", "/w/private/repo", secret),
            ],
        );
        for name in [kept, secret, unnamed] {
            std::fs::write(data.join(SPILL_DIR).join(name), full_output(name)).unwrap();
        }
        assert_eq!(spill_projects(&db, kept).unwrap(), vec!["/w/app"]);
        assert!(spill_projects(&db, unnamed).unwrap().is_empty());
        let home = Home::new(dir.path().join("home"));
        let mut config = opencode_config(&db);
        config.privacy.exclude_paths = vec!["/w/private".into()];

        let r = crate::sweep::sweep(&home, &config, None).unwrap();
        // ses1 and its spill are kept; ses2, its spill and the spill no session names are not.
        assert_eq!((r.new, r.excluded), (2, 3), "{r:?}");
        let archive = Archive::open(&home).unwrap();
        assert!(
            archive
                .latest()
                .contains_key(&*data.join(SPILL_DIR).join(kept).to_string_lossy())
        );

        // With no exclusion set, every spill is captured.
        config.privacy.exclude_paths.clear();
        let r = crate::sweep::sweep(&home, &config, None).unwrap();
        assert_eq!(r.new, 3, "{r:?}");
    }

    /// Add OpenCode 2's tables to the database at `db`, with one session `ses_v2only` that only
    /// they hold, and `ses1`, which they hold too (its messages here are never exported).
    fn add_v2(db: &Path, spill: &Path) {
        let conn = Connection::open(db).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE session_v2 (id TEXT PRIMARY KEY, directory TEXT, parent_id TEXT, title TEXT, version TEXT, model TEXT, agent TEXT, time_created INTEGER, time_updated INTEGER);
               CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER, time_created INTEGER, time_updated INTEGER, data TEXT);
               INSERT INTO session_v2 VALUES ('ses1', '/w/app', NULL, 'T', '2.0.0', NULL, NULL, 1000, 5000);
               INSERT INTO session_message VALUES ('v2m-dup', 'ses1', 'user', 1, 1000, 1000, '{"text":"never exported"}');
               INSERT INTO session_v2 VALUES ('ses_v2only', '/w/new', NULL, 'T', '2.0.0', '{"id":"kimi-k3","providerID":"p"}', NULL, 1000, 6000);"#,
        )
        .unwrap();
        let tool = json!({"type": "tool", "id": "call-9", "name": "bash",
            "state": {"status": "completed", "input": {"command": "seq 3"},
                "content": [{"type": "text", "text": "line 1\n...truncated"}],
                "metadata": {"truncated": true, "outputPath": spill.display().to_string()}},
            "time": {"created": 2001}});
        let rows = [
            ("m1", "user", 1, 1000, json!({"text": "count to three"})),
            ("m2", "system", 2, 1500, json!({"text": "env changed"})),
            (
                "m3",
                "assistant",
                3,
                2000,
                json!({"agent": "build", "model": {"id": "kimi-k3", "providerID": "p"},
                    "content": [{"type": "reasoning", "text": "think"}, {"type": "text", "text": "running"}, tool]}),
            ),
            ("m4", "idle", 4, 3000, json!({"outcome": "succeeded"})),
            (
                "m5",
                "compaction",
                5,
                4000,
                json!({"status": "completed", "reason": "auto", "summary": "so far"}),
            ),
            (
                "m6",
                "model-switched",
                6,
                5000,
                json!({"model": {"id": "x"}}),
            ),
            ("m7", "future-type", 7, 5500, json!({})),
        ];
        for (id, kind, seq, at, data) in rows {
            conn.execute(
                "INSERT INTO session_message VALUES (?1, 'ses_v2only', ?2, ?3, ?4, ?4, ?5)",
                rusqlite::params![id, kind, seq, at, data.to_string()],
            )
            .unwrap();
        }
    }

    #[test]
    fn a_session_only_opencode_2_holds_is_archived_once_beside_those_in_v1() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("opencode");
        let db = opencode_data(
            &data,
            &[("ses1", "/w/app", "tool_0a1b2c3d4e5fSyntheticDdd4")],
        );
        let spill_path = data.join(SPILL_DIR).join("tool_0a1b2c3d4e5fSyntheticCcc3");
        add_v2(&db, &spill_path);
        std::fs::write(&spill_path, "line 1\nline 2\nline 3\n").unwrap();
        let home = Home::new(dir.path().join("home"));
        let config = opencode_config(&db);

        // A session in both sets of tables is one candidate.
        let origins: Vec<_> = list_sessions(&db)
            .unwrap()
            .into_iter()
            .map(|c| c.origin())
            .collect();
        assert_eq!(origins.len(), 2, "{origins:?}");
        let r = crate::sweep::sweep(&home, &config, None).unwrap();
        assert_eq!((r.new, r.errors.len()), (3, 0), "{r:?}"); // two sessions and the spill
        let archive = Archive::open(&home).unwrap();
        for origin in &origins {
            assert!(Harness::OpenCode.is_primary(origin));
            assert!(archive.latest().contains_key(origin));
        }
        let dup = export_session(&db, "ses1").unwrap();
        assert!(!String::from_utf8_lossy(&dup.bytes).contains("never exported"));
        let n = crate::normalize::normalize(&home, false).unwrap();
        assert_eq!((n.sessions, n.sidecars), (2, 1), "{n:?}");
        let meta = crate::normalize::find(&home, Harness::OpenCode, "ses_v2only")
            .unwrap()
            .unwrap();
        assert_eq!(tool_result(&home, "ses_v2only"), "line 1\nline 2\nline 3\n");
        assert!(!meta.raw.attached.is_empty());

        // Nothing changed, nothing is written again.
        let r = crate::sweep::sweep(&home, &config, None).unwrap();
        assert_eq!(
            (r.new, r.appended, r.rewritten, r.errors.len()),
            (0, 0, 0, 0),
            "{r:?}"
        );
    }

    #[test]
    fn a_v2_session_parses_into_events() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("opencode.db");
        Connection::open(&db).unwrap();
        add_v2(
            &db,
            &dir.path()
                .join("tool-output/tool_0a1b2c3d4e5fSyntheticEee5"),
        );
        let p = parse(&export_session(&db, "ses_v2only").unwrap().bytes).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("ses_v2only"));
        assert_eq!(p.cwd.as_deref(), Some("/w/new"));
        assert_eq!(p.models, vec!["kimi-k3"]);
        let got: Vec<_> = p
            .events
            .iter()
            .map(|e| (e.kind, e.text.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![
                (EventKind::User, Some("count to three")),
                (EventKind::System, Some("env changed")),
                (EventKind::Reasoning, Some("think")),
                (EventKind::Assistant, Some("running")),
                (EventKind::ToolCall, None),
                // The spilled output is not in the archive here: the preview stays.
                (EventKind::ToolResult, Some("line 1\n...truncated")),
                (EventKind::System, Some("so far")),
            ]
        );
        assert_eq!(p.events[4].tool.as_deref(), Some("bash"));
        assert_eq!(p.events[4].call_id.as_deref(), Some("call-9"));
        assert_eq!(p.events[5].is_error, Some(false));
    }

    #[test]
    fn a_v2_assistant_message_may_keep_its_content_in_part_rows() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("opencode.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"CREATE TABLE session_v2 (id TEXT PRIMARY KEY, directory TEXT, parent_id TEXT, title TEXT, version TEXT, model TEXT, agent TEXT, time_created INTEGER, time_updated INTEGER);
               CREATE TABLE session_message (id TEXT PRIMARY KEY, session_id TEXT, type TEXT, seq INTEGER, time_created INTEGER, time_updated INTEGER, data TEXT);
               CREATE TABLE session_part (message_id TEXT, session_id TEXT, ordinal INTEGER, type TEXT, call_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT);
               INSERT INTO session_v2 VALUES ('s', '/w', NULL, NULL, '2', NULL, NULL, 1, 2);
               INSERT INTO session_message VALUES ('m', 's', 'assistant', 1, 1, 1, '{"model":{"id":"k"},"content":[]}');
               INSERT INTO session_part VALUES ('m', 's', 1, 'text', NULL, 1, 1, '{"text":"second"}');
               INSERT INTO session_part VALUES ('m', 's', 0, 'tool', 'c', 1, 1, '{"id":"c","name":"read","state":{"status":"error","input":{},"error":{"type":"x","message":"nope"}}}');"#,
        )
        .unwrap();
        drop(conn);
        let p = parse(&export_session(&db, "s").unwrap().bytes).unwrap();
        let got: Vec<_> = p
            .events
            .iter()
            .map(|e| (e.kind, e.text.as_deref()))
            .collect();
        assert_eq!(
            got,
            vec![
                (EventKind::ToolCall, None),
                (EventKind::ToolResult, Some("nope")),
                (EventKind::Assistant, Some("second")),
            ]
        );
        assert_eq!(p.events[1].is_error, Some(true));
    }

    #[test]
    fn a_database_without_v2_tables_still_sweeps() {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("opencode");
        let db = opencode_data(
            &data,
            &[("ses1", "/w/app", "tool_0a1b2c3d4e5fSyntheticFff6")],
        );
        assert_eq!(list_sessions(&db).unwrap().len(), 1);
        let home = Home::new(dir.path().join("home"));
        let r = crate::sweep::sweep(&home, &opencode_config(&db), None).unwrap();
        assert_eq!((r.new, r.errors.len()), (1, 0), "{r:?}");
    }
}
