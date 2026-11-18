//! Cursor's agent: `~/.cursor/chats/<workspace hash>/<agent id>/store.db`, a content-addressed
//! store. `meta` row `0` is hex-encoded JSON naming the chat (`agentId`, `name`, `createdAt`)
//! and its latest root blob. The root is a protobuf message whose field 1 lists the blob ids of
//! the conversation's messages in order, field 9 the workspace URI and field 21 the workspace
//! path and git branch. Each message blob is JSON in the AI SDK shape: `role` plus a string or
//! an array of parts (`text`, `reasoning`, `tool-call`, `tool-result`, `image`).
//!
//! Cursor wraps what the user typed in `<user_query>` and sends context (`<user_info>`,
//! `<system_reminder>`, attached files) as user messages too; those become system events. A
//! `<timestamp>` before a query is the only clock in the store, so it dates that turn.

use std::io::Write;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OpenFlags};
use serde_json::Value;

use super::blocks::{millis_to_rfc3339, result_text};
use crate::harness::percent_decode;
use crate::model::{Event, EventKind, Parsed};

/// Message parts (`<role>:<type>`) the parser skips on purpose. Any other part it skips is
/// reported as format drift.
///
/// Reasoning Cursor keeps encrypted, so it has no text to read (#104).
pub const IGNORED: &[&str] = &["assistant:redacted-reasoning"];

pub fn parse(origin: &str, bytes: &[u8]) -> Result<Parsed> {
    if crate::harness::cursor_export(origin) {
        return parse_export(bytes);
    }
    if crate::harness::cursor_transcript(origin) {
        return Ok(parse_transcript(origin, bytes));
    }
    // SQLite reads from a file; the copy lives only as long as the parse.
    let mut tmp = tempfile::NamedTempFile::new()?;
    tmp.write_all(bytes)?;
    tmp.flush()?;
    let conn = Connection::open_with_flags(
        tmp.path(),
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("open Cursor chat store")?;
    parse_store(origin, &conn)
}

/// Cursor's plain transcript of an agent (`projects/<project>/agent-transcripts/<id>.jsonl`):
/// `{"role", "message": {"content": [text | tool_use]}}` lines, plus run status lines. It holds
/// less than the agent's store (no tool results, no models), so it is a copy of that store
/// when the store is archived, and the only record of the agent when it isn't.
pub fn parse_transcript(origin: &str, bytes: &[u8]) -> Parsed {
    let stem = origin
        .rsplit('/')
        .next()
        .and_then(|f| f.strip_suffix(".jsonl"))
        .unwrap_or(origin);
    let mut out = Parsed {
        // Its own id, so it never takes the store's place.
        session_id: Some(format!("transcript-{stem}")),
        mirror_of: vec![crate::harness::Harness::Cursor],
        mirror_refs: vec![format!("cursor/{stem}")],
        ..Default::default()
    };
    let mut at = None;
    for line in String::from_utf8_lossy(bytes).lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let role = v.get("role").and_then(Value::as_str).unwrap_or("");
        let content = v.pointer("/message/content");
        let blocks: Vec<Value> = match content {
            Some(Value::Array(b)) => b.clone(),
            Some(Value::String(t)) => vec![serde_json::json!({"type": "text", "text": t})],
            _ => Vec::new(),
        };
        if blocks.is_empty() {
            // A run that ended in an error says so; other status lines are bookkeeping.
            if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
                let mut e = Event::new(EventKind::ToolResult).at(at.clone());
                e.tool = Some("error".into());
                e.text = Some(match err {
                    Value::String(s) => s.clone(),
                    other => other.to_string(),
                });
                e.is_error = Some(true);
                out.push(e);
            }
            continue;
        }
        for b in &blocks {
            match (role, b.get("type").and_then(Value::as_str).unwrap_or("")) {
                ("user", "text") => push_user_text(
                    &mut out,
                    b.get("text").and_then(Value::as_str).unwrap_or(""),
                    &mut at,
                ),
                ("assistant", "text") => {
                    let t = b.get("text").and_then(Value::as_str).unwrap_or("");
                    if !t.is_empty() {
                        out.push(Event::text(EventKind::Assistant, t).at(at.clone()));
                    }
                }
                ("assistant", "tool_use") => {
                    let mut e = Event::new(EventKind::ToolCall).at(at.clone());
                    e.tool = b.get("name").and_then(Value::as_str).map(str::to_string);
                    e.call_id = b.get("id").and_then(Value::as_str).map(str::to_string);
                    e.input = b.get("input").cloned();
                    out.push(e);
                }
                (role, ty) => out.skip_in(role, ty, b),
            }
        }
    }
    out
}

fn parse_store(origin: &str, conn: &Connection) -> Result<Parsed> {
    let meta_hex: String = conn
        .query_row("SELECT value FROM meta WHERE key = '0'", [], |r| r.get(0))
        .context("chat store has no meta row")?;
    let meta: Value = serde_json::from_slice(&hex::decode(meta_hex.trim())?)?;
    let root_id = meta
        .get("latestRootBlobId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let blob = |id: &str| -> Result<Option<Vec<u8>>> {
        let mut stmt = conn.prepare_cached("SELECT data FROM blobs WHERE id = ?1")?;
        let mut rows = stmt.query([id])?;
        Ok(match rows.next()? {
            Some(r) => Some(r.get::<_, Vec<u8>>(0)?),
            None => None,
        })
    };
    // A chat opened but never sent has no root yet: an empty session, not a broken one.
    let root = if root_id.is_empty() {
        Vec::new()
    } else {
        blob(root_id)?.with_context(|| format!("root blob {root_id} missing"))?
    };

    let mut out = Parsed {
        session_id: meta
            .get("agentId")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| origin.rsplit('/').nth(1).map(str::to_string)),
        ..Default::default()
    };
    let mut message_ids = Vec::new();
    for (field, value) in fields(&root)? {
        match (field, value) {
            (1, Field::Bytes(id)) => message_ids.push(hex::encode(id)),
            (9, Field::Bytes(uri)) if out.cwd.is_none() => {
                let uri = String::from_utf8_lossy(uri);
                if let Some(path) = uri.strip_prefix("file://") {
                    out.cwd = Some(percent_decode(path));
                }
            }
            (21, Field::Bytes(ws)) => {
                for (f, v) in fields(ws)? {
                    match (f, v) {
                        (1, Field::Bytes(p)) => {
                            out.cwd = Some(String::from_utf8_lossy(p).into_owned())
                        }
                        (2, Field::Bytes(b)) if !b.is_empty() => {
                            out.git_branch = Some(String::from_utf8_lossy(b).into_owned())
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    // Until a turn says when it happened, events carry the chat's creation time.
    let mut at = meta
        .get("createdAt")
        .and_then(Value::as_i64)
        .map(millis_to_rfc3339);
    for id in message_ids {
        // A message the root lists but the store lacks would leave a hole in the conversation
        // (an answer with no prompt); better to fail than to keep it.
        let bytes = blob(&id)?.with_context(|| format!("message blob {id} missing"))?;
        // Field 1 also lists non-message blobs in some versions; only JSON messages count.
        let Ok(m) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        push_message(&mut out, &m, &mut at);
    }
    Ok(out)
}

fn push_message(out: &mut Parsed, m: &Value, at: &mut Option<String>) {
    let role = m.get("role").and_then(Value::as_str).unwrap_or("");
    let parts: Vec<Value> = match m.get("content") {
        Some(Value::String(s)) => vec![serde_json::json!({"type": "text", "text": s})],
        Some(Value::Array(a)) => a.clone(),
        _ => return,
    };
    for p in &parts {
        let ty = p.get("type").and_then(Value::as_str).unwrap_or("");
        let text = || p.get("text").and_then(Value::as_str).unwrap_or("");
        let model = p
            .pointer("/providerOptions/cursor/modelName")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(md) = &model {
            out.add_model(md);
        }
        match (role, ty) {
            ("system", "text") => out.push(Event::text(EventKind::System, text()).at(at.clone())),
            ("user", "text") => push_user_text(out, text(), at),
            ("user", "image") => out.push(Event::text(EventKind::User, "[image]").at(at.clone())),
            ("assistant", "text") => {
                if !text().is_empty() {
                    let mut e = Event::text(EventKind::Assistant, text()).at(at.clone());
                    e.model = model;
                    out.push(e);
                }
            }
            ("assistant", "reasoning") => {
                if !text().is_empty() {
                    out.push(Event::text(EventKind::Reasoning, text()).at(at.clone()))
                }
            }
            ("assistant", "tool-call") => {
                let mut e = Event::new(EventKind::ToolCall).at(at.clone());
                e.tool = p
                    .get("toolName")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                e.call_id = p
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                e.input = p.get("args").or_else(|| p.get("input")).cloned();
                e.model = model;
                out.push(e);
            }
            ("tool", "tool-result") => {
                let mut e = Event::new(EventKind::ToolResult).at(at.clone());
                e.tool = p
                    .get("toolName")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                e.call_id = p
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let result = p
                    .get("result")
                    .filter(|r| !r.is_null())
                    .or_else(|| p.get("experimental_content"))
                    .unwrap_or(&Value::Null);
                e.text = Some(result_text(result));
                if let Some(err) = p.get("isError").and_then(Value::as_bool) {
                    e.is_error = Some(err);
                }
                out.push(e);
            }
            (role, ty) => out.skip_in(role, ty, p),
        }
    }
}

/// Context Cursor injects as user messages.
const CONTEXT_TAGS: &[&str] = &[
    "user_info",
    "system_reminder",
    "timestamp",
    "manually_attached_skills",
    "attached_files",
    "rules",
    "agent_skills",
    "open_and_recently_viewed_files",
    "git_status",
];

fn push_user_text(out: &mut Parsed, text: &str, at: &mut Option<String>) {
    if let Some(stamp) = between(text, "<timestamp>", "</timestamp>")
        && let Some(t) = parse_timestamp(stamp)
    {
        *at = Some(t);
    }
    let queries: Vec<&str> = all_between(text, "<user_query>", "</user_query>");
    if !queries.is_empty() {
        // Context sent in the same text as the query (a reminder, the time) stays, as system.
        let mut rest = text.to_string();
        for q in &queries {
            rest = rest.replacen(&format!("<user_query>{q}</user_query>"), "", 1);
        }
        if !rest.trim().is_empty() {
            out.push(Event::text(EventKind::System, rest.trim()).at(at.clone()));
        }
        let q = queries
            .iter()
            .map(|q| q.trim())
            .collect::<Vec<_>>()
            .join("\n\n");
        out.push(Event::text(EventKind::User, q).at(at.clone()));
        return;
    }
    let head = text.trim_start();
    let context = head
        .strip_prefix('<')
        .and_then(|r| r.split_once('>'))
        .is_some_and(|(tag, _)| CONTEXT_TAGS.contains(&tag))
        || head.starts_with("Your conversation was summarized");
    let kind = if context {
        EventKind::System
    } else {
        EventKind::User
    };
    out.push(Event::text(kind, text).at(at.clone()));
}

fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open)? + open.len();
    let end = s[start..].find(close)? + start;
    Some(&s[start..end])
}

fn all_between<'a>(mut s: &'a str, open: &str, close: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    while let Some(start) = s.find(open) {
        let rest = &s[start + open.len()..];
        let Some(end) = rest.find(close) else { break };
        out.push(&rest[..end]);
        s = &rest[end + close.len()..];
    }
    out
}

/// `Monday, Jul 6, 2026, 8:54 AM (UTC-5)` → RFC 3339.
fn parse_timestamp(s: &str) -> Option<String> {
    let (local, zone) = s.trim().rsplit_once(" (UTC")?;
    let zone = zone.strip_suffix(')')?;
    let (sign, rest) = match zone.chars().next() {
        Some('+') => (1, &zone[1..]),
        Some('-') => (-1, &zone[1..]),
        _ => (1, zone),
    };
    let (h, m) = match rest.split_once(':') {
        Some((h, m)) => (h.parse::<u8>().ok()?, m.parse::<u8>().ok()?),
        None if rest.is_empty() => (0, 0),
        None => (rest.parse::<u8>().ok()?, 0),
    };
    // Real offsets run from UTC-12 to UTC+14; anything else is not a timestamp.
    if h > 14 || m >= 60 {
        return None;
    }
    let offset = chrono::FixedOffset::east_opt(sign * (i32::from(h) * 3600 + i32::from(m) * 60))?;
    let naive = chrono::NaiveDateTime::parse_from_str(local, "%A, %b %d, %Y, %I:%M %p").ok()?;
    let t = naive.and_local_timezone(offset).single()?;
    Some(
        t.with_timezone(&chrono::Utc)
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    )
}

enum Field<'a> {
    Varint,
    Bytes(&'a [u8]),
}

fn varint(b: &[u8], i: &mut usize) -> Result<u64> {
    let mut v = 0u64;
    for shift in (0..64).step_by(7) {
        let Some(&x) = b.get(*i) else {
            bail!("truncated varint");
        };
        *i += 1;
        v |= u64::from(x & 0x7f) << shift;
        if x < 0x80 {
            return Ok(v);
        }
    }
    bail!("varint too long")
}

/// The top-level fields of a protobuf message, without a schema.
fn fields(b: &[u8]) -> Result<Vec<(u64, Field<'_>)>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let key = varint(b, &mut i)?;
        let field = match key & 7 {
            0 => {
                varint(b, &mut i)?;
                Field::Varint
            }
            1 | 5 => {
                i += if key & 7 == 1 { 8 } else { 4 };
                Field::Varint
            }
            2 => {
                let n = varint(b, &mut i)? as usize;
                let end = i.checked_add(n).filter(|&e| e <= b.len());
                let Some(end) = end else {
                    bail!("truncated field");
                };
                let v = &b[i..end];
                i = end;
                Field::Bytes(v)
            }
            t => bail!("unsupported wire type {t}"),
        };
        out.push((key >> 3, field));
    }
    if i > b.len() {
        bail!("truncated message");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pb_bytes(field: u64, v: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut key = (field << 3) | 2;
        loop {
            let b = (key & 0x7f) as u8;
            key >>= 7;
            if key == 0 {
                out.push(b);
                break;
            }
            out.push(b | 0x80);
        }
        out.push(v.len() as u8);
        out.extend_from_slice(v);
        out
    }

    /// A store laid out the way Cursor writes one.
    fn store(path: &std::path::Path, messages: &[&str]) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();
        let mut root = Vec::new();
        for (i, m) in messages.iter().enumerate() {
            let id = [i as u8 + 1; 32];
            conn.execute(
                "INSERT INTO blobs VALUES (?1, ?2)",
                rusqlite::params![hex::encode(id), m.as_bytes()],
            )
            .unwrap();
            root.extend(pb_bytes(1, &id));
        }
        root.extend(pb_bytes(9, b"file:///w/my%20app"));
        let mut ws = pb_bytes(1, b"/w/my app");
        ws.extend(pb_bytes(2, b"feat/x"));
        root.extend(pb_bytes(21, &ws));
        root.extend(pb_bytes(22, b"cli"));
        conn.execute(
            "INSERT INTO blobs VALUES ('root', ?1)",
            rusqlite::params![root],
        )
        .unwrap();
        let meta = r#"{"agentId":"a-1","latestRootBlobId":"root","name":"Fix the build","createdAt":1783328040000}"#;
        conn.execute("INSERT INTO meta VALUES ('0', ?1)", [hex::encode(meta)])
            .unwrap();
    }

    #[test]
    fn reads_messages_in_root_order_and_separates_context_from_queries() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("store.db");
        store(
            &db,
            &[
                r#"{"role":"system","content":"You are a coding agent."}"#,
                r#"{"role":"user","content":"<user_info>\nOS: darwin\n</user_info>"}"#,
                r#"{"role":"user","content":[{"type":"text","text":"<system_reminder>be careful</system_reminder>"},{"type":"text","text":"<timestamp>Monday, Jul 6, 2026, 8:54 AM (UTC-5)</timestamp>\n<user_query>\nfix the build\n</user_query>"}]}"#,
                r#"{"role":"assistant","id":"m1","content":[{"type":"reasoning","text":"","signature":"x"},{"type":"reasoning","text":"run it","providerOptions":{"cursor":{"modelName":"composer-2.5"}}},{"type":"text","text":"Running make."},{"type":"tool-call","toolCallId":"c1","toolName":"Shell","args":{"command":"make"}}]}"#,
                r#"{"role":"tool","content":[{"type":"tool-result","toolCallId":"c1","toolName":"Shell","result":"ok"}]}"#,
            ],
        );
        let p = parse(
            "/u/.cursor/chats/h/a-1/store.db",
            &std::fs::read(&db).unwrap(),
        )
        .unwrap();
        assert_eq!(p.session_id.as_deref(), Some("a-1"));
        assert_eq!(p.cwd.as_deref(), Some("/w/my app"));
        assert_eq!(p.git_branch.as_deref(), Some("feat/x"));
        assert_eq!(p.models, vec!["composer-2.5"]);
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::System,
                EventKind::System,
                EventKind::System,
                EventKind::System,
                EventKind::User,
                EventKind::Reasoning,
                EventKind::Assistant,
                EventKind::ToolCall,
                EventKind::ToolResult,
            ]
        );
        // The timestamp sent with the query stays, as context.
        assert!(
            p.events[3]
                .text
                .as_deref()
                .unwrap()
                .starts_with("<timestamp>")
        );
        assert_eq!(p.events[4].text.as_deref(), Some("fix the build"));
        // The chat's creation time dates events until a turn's own timestamp.
        assert_eq!(
            p.events[0].ts.as_deref(),
            Some(millis_to_rfc3339(1783328040000).as_str())
        );
        assert_eq!(p.events[4].ts.as_deref(), Some("2026-07-06T13:54:00.000Z"));
        assert_eq!(p.events[7].input.as_ref().unwrap()["command"], "make");
        assert_eq!(p.events[8].text.as_deref(), Some("ok"));
        assert_eq!(p.events[8].call_id.as_deref(), Some("c1"));

        // A message the root lists but the store lacks fails the parse.
        let conn = Connection::open(&db).unwrap();
        conn.execute("DELETE FROM blobs WHERE id = ?1", [hex::encode([3u8; 32])])
            .unwrap();
        drop(conn);
        let err = parse(
            "/u/.cursor/chats/h/a-1/store.db",
            &std::fs::read(&db).unwrap(),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("missing"), "{err:#}");
    }

    #[test]
    fn transcripts_are_copies_of_their_agent_store() {
        let src = r#"{"role":"user","message":{"content":[{"type":"text","text":"<timestamp>Monday, Jul 6, 2026, 8:54 AM (UTC-5)</timestamp>\n<user_query>\nlist the docs\n</user_query>"}]}}
{"role":"assistant","message":{"content":[{"type":"text","text":"Looking."},{"type":"tool_use","name":"Glob","input":{"glob_pattern":"**/*.md"}}]}}
{"type":"status","status":"error","error":"rate limited"}
{"type":"status","status":"done"}"#;
        let p = parse(
            "/u/.cursor/projects/w/agent-transcripts/agent-1/agent-1.jsonl",
            src.as_bytes(),
        )
        .unwrap();
        assert_eq!(p.session_id.as_deref(), Some("transcript-agent-1"));
        assert_eq!(p.mirror_refs, vec!["cursor/agent-1"]);
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::System,
                EventKind::User,
                EventKind::Assistant,
                EventKind::ToolCall,
                EventKind::ToolResult
            ]
        );
        assert_eq!(p.events[1].text.as_deref(), Some("list the docs"));
        assert_eq!(p.events[1].ts.as_deref(), Some("2026-07-06T13:54:00.000Z"));
        assert_eq!(p.events[4].is_error, Some(true));
    }

    #[test]
    fn timestamps_with_odd_or_impossible_offsets() {
        assert_eq!(
            parse_timestamp("Monday, Jul 6, 2026, 8:54 AM (UTC+5:30)").as_deref(),
            Some("2026-07-06T03:24:00.000Z")
        );
        assert_eq!(
            parse_timestamp("Monday, Jul 6, 2026, 8:54 PM (UTC)").as_deref(),
            Some("2026-07-06T20:54:00.000Z")
        );
        for bad in [
            "Monday, Jul 6, 2026, 8:54 AM (UTC+2147483647)",
            "Monday, Jul 6, 2026, 8:54 AM (UTC-15)",
            "Monday, Jul 6, 2026, 8:54 AM (UTC+1:75)",
            "Monday, Jul 6, 2026, 8:54 AM",
        ] {
            assert_eq!(parse_timestamp(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_missing_root_fails_but_an_unsent_chat_is_empty() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("store.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);
             CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO meta VALUES ('0', ?1)",
            [hex::encode(r#"{"agentId":"a","latestRootBlobId":"gone"}"#)],
        )
        .unwrap();
        let bytes = std::fs::read(&db).unwrap();
        assert!(parse("/c/h/x/store.db", &bytes).is_err());
        conn.execute(
            "UPDATE meta SET value = ?1",
            [hex::encode(r#"{"agentId":"a","latestRootBlobId":""}"#)],
        )
        .unwrap();
        drop(conn);
        let p = parse("/c/h/x/store.db", &std::fs::read(&db).unwrap()).unwrap();
        assert!(p.events.is_empty());
    }
}

/// Cursor's Export Transcript writes a Markdown title and byline, followed by
/// speaker blocks. A horizontal rule alone is also ordinary message content.
fn parse_export(bytes: &[u8]) -> Result<Parsed> {
    let text = std::str::from_utf8(bytes)?.replace("\r\n", "\n");
    let (header, turns) = text
        .split_once("\n\n---\n")
        .context("missing Cursor export header")?;
    let (title, byline) = header
        .rsplit_once('\n')
        .context("missing Cursor export byline")?;
    anyhow::ensure!(
        title.starts_with("# ")
            && byline.starts_with("_Exported on ")
            && byline.contains(" from Cursor (")
            && byline.ends_with(")_"),
        "invalid Cursor export header"
    );
    let mut lines = turns.trim_start_matches('\n').lines().peekable();
    let mut out = Parsed::default();
    let mut role = None;
    let mut body = Vec::new();
    let mut fence = None;
    while let Some(line) = lines.next() {
        if fence.is_none() && (role.is_none() || line == "---") {
            let mut rest = lines.clone();
            let label = if role.is_none() {
                Some(line)
            } else {
                rest.next_if_eq(&"");
                rest.next()
            };
            let next = match label {
                Some("**User**") => Some(EventKind::User),
                Some("**Cursor**") => Some(EventKind::Assistant),
                _ => None,
            };
            if let Some(next) = next
                && rest.next() == Some("")
            {
                if let Some(previous) = role.replace(next) {
                    out.push(Event::text(
                        previous,
                        body.join("\n").trim_end_matches('\n'),
                    ));
                    body.clear();
                }
                lines = rest;
                continue;
            }
        }
        anyhow::ensure!(role.is_some(), "unknown speaker in Cursor export");
        update_export_fence(line, &mut fence);
        body.push(line);
    }
    anyhow::ensure!(fence.is_none(), "unclosed code fence in Cursor export");
    if let Some(last) = role {
        out.push(Event::text(last, body.join("\n").trim_end_matches('\n')));
    }
    Ok(out)
}

/// Store the marker, fence width and list-item content indentation.
fn update_export_fence(line: &str, fence: &mut Option<(char, usize, usize)>) {
    static LIST_MARKER: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"^(?:[-*+]|[0-9]{1,9}[.)])").unwrap());
    let (column, mut code) = after_indent(line, 0);
    let mut indent = 0;
    if let Some((_, _, container)) = *fence {
        if !(container..=container + 3).contains(&column) {
            return;
        }
    } else if column > 3 {
        return;
    } else if let Some(marker) = LIST_MARKER.find(code) {
        let end = column + marker.end();
        let (body, rest) = after_indent(&code[marker.end()..], end);
        if !(1..=4).contains(&(body - end)) {
            return;
        }
        indent = body;
        code = rest;
    }
    let Some(marker @ ('`' | '~')) = code.chars().next() else {
        return;
    };
    let width = code.chars().take_while(|c| *c == marker).count();
    if width < 3 {
        return;
    }
    let info = &code[width..];
    match *fence {
        None if marker != '`' || !info.contains('`') => *fence = Some((marker, width, indent)),
        Some((m, n, _)) if m == marker && width >= n && info.trim().is_empty() => *fence = None,
        _ => {}
    }
}

/// Markdown advances a tab to the next four-column stop. Slice only the
/// structural prefix so archived message text retains its original whitespace.
fn after_indent(text: &str, mut column: usize) -> (usize, &str) {
    for (i, byte) in text.bytes().enumerate() {
        match byte {
            b' ' => column += 1,
            b'\t' => column += 4 - column % 4,
            _ => return (column, &text[i..]),
        }
    }
    (column, "")
}

#[test]
fn parses_an_export_without_splitting_speaker_examples_inside_code() {
    let parsed = parse_export(b"# Example\r\n_Exported on Monday at noon from Cursor (1)_\r\n\r\n---\r\n\r\n**User**\r\n\r\nShow me.\n\n---\n\n**Cursor**\n\n```md\n---\n\n**User**\n\nexample\n```\n").unwrap();
    assert_eq!(parsed.events.len(), 2);
    assert_eq!(parsed.events[0].kind, EventKind::User);
    assert_eq!(parsed.events[1].kind, EventKind::Assistant);
    assert!(parsed.events[1].text.as_ref().unwrap().contains("**User**"));
    assert!(parse_export(b"# Ordinary markdown").is_err());
}

#[test]
fn compact_separators_and_incomplete_code_in_exports() {
    let export = "# Example\n_Exported on Monday at noon from Cursor (1)_\n\n---\n**User**\n\nOne\n\n---\n**Cursor**\n\nTwo";
    assert_eq!(parse_export(export.as_bytes()).unwrap().events.len(), 2);
    let inline = parse_export(format!("{export}\n ```foo```").as_bytes()).unwrap();
    assert_eq!(inline.events[1].text.as_deref(), Some("Two\n ```foo```"));
    assert!(parse_export(format!("{export}\n```rs\n---\n**User**\n\nThree").as_bytes()).is_err());
}

#[test]
fn list_item_code_fences_keep_the_following_turn_searchable() {
    for (marker, indent) in [
        ("-", "  "),
        ("*", "  "),
        ("+", "  "),
        ("1.", "   "),
        ("12)", "    "),
        ("-", "\t"),
    ] {
        let text = format!(
            "# Example\n_Exported on Monday at noon from Cursor (1)_\n\n---\n\n**Cursor**\n\n{marker} ```rs\n{indent}example\n{indent}```\n\n---\n\n**User**\n\nThanks"
        );
        let parsed = parse_export(text.as_bytes()).unwrap();
        assert_eq!(parsed.events.len(), 2);
        assert!(
            parsed.events[0]
                .text
                .as_ref()
                .unwrap()
                .ends_with(&format!("{indent}```"))
        );
        assert_eq!(parsed.events[1].kind, EventKind::User);
        assert_eq!(parsed.events[1].text.as_deref(), Some("Thanks"));
    }
}
