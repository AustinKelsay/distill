//! T3 Code keeps every thread it runs, whatever the harness, in one SQLite database
//! (`~/.t3/userdata/statev2.sqlite`). The sweep exports each thread as JSONL: one header line of
//! fields that don't change as the thread runs, then its turn items in order, so a thread that
//! grows appends. Threads touched in the last few minutes wait for the next sweep.
//!
//! A thread whose harness Distill also captures natively (Codex, Claude Code, …) is a mirror of
//! that session: archived and searchable, but left out of datasets and steward cards so nothing
//! is counted twice. Threads from harnesses with no local store of their own are the only copy.

use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

use crate::model::{Event, EventKind, Parsed};
use crate::source::Candidate;

/// Providers whose sessions Distill also reads from their own files.
const NATIVE: [&str; 7] = [
    "codex",
    "claudeAgent",
    "opencode",
    "pi",
    "grok",
    "antigravity",
    "droid",
];

/// How long a thread must be idle before it is captured.
const SETTLE_MINUTES: i64 = 10;

fn open(db: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open {}", db.display()))?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    Ok(conn)
}

pub fn list_threads(db: &Path) -> Result<Vec<Candidate>> {
    let conn = open(db)?;
    let cutoff = (chrono::Utc::now() - chrono::Duration::minutes(SETTLE_MINUTES))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut stmt = conn.prepare(
        "SELECT thread_id, updated_at FROM orchestration_v2_projection_threads
         WHERE deleted_at IS NULL AND updated_at < ?1 ORDER BY thread_id",
    )?;
    let rows = stmt.query_map([cutoff], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (thread_id, updated) = row?;
        out.push(Candidate::T3Thread {
            db: db.to_path_buf(),
            thread_id,
            updated,
        });
    }
    Ok(out)
}

pub struct Export {
    pub bytes: Vec<u8>,
    pub cwd: Option<String>,
}

pub fn export_thread(db: &Path, thread_id: &str) -> Result<Export> {
    let conn = open(db)?;
    let (payload, provider_instance, created_at, project_root, provider): (
        String,
        String,
        String,
        Option<String>,
        Option<String>,
    ) = conn.query_row(
        "SELECT t.payload_json, t.default_provider, t.created_at, p.workspace_root,
                    (SELECT pt.provider FROM orchestration_v2_projection_provider_threads pt
                     WHERE pt.thread_id = t.thread_id ORDER BY pt.updated_at DESC LIMIT 1)
             FROM orchestration_v2_projection_threads t
             LEFT JOIN projection_projects p ON p.project_id = t.project_id
             WHERE t.thread_id = ?1",
        [thread_id],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
    )?;
    let t: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
    let cwd = t
        .get("worktreePath")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(project_root.clone());
    // Only fields that stay put while the thread runs, so the export grows by appending.
    let header = json!({
        "row": "thread",
        "thread_id": thread_id,
        "title": t.get("title"),
        "created_at": created_at,
        "provider_instance": provider_instance,
        "provider": provider,
        "model": t.pointer("/modelSelection/model"),
        "project_root": project_root,
        "worktree_path": t.get("worktreePath"),
        "branch": t.get("branch"),
        "lineage": t.get("lineage"),
        "forked_from": t.get("forkedFrom"),
        "created_by": t.get("createdBy"),
    });
    let mut bytes = Vec::new();
    serde_json::to_writer(&mut bytes, &header)?;
    bytes.push(b'\n');
    let mut stmt = conn.prepare(
        "SELECT payload_json FROM orchestration_v2_projection_turn_items
         WHERE thread_id = ?1 ORDER BY ordinal, turn_item_id",
    )?;
    let rows = stmt.query_map([thread_id], |r| r.get::<_, String>(0))?;
    for row in rows {
        let item: Value = serde_json::from_str(&row?).unwrap_or(Value::Null);
        serde_json::to_writer(&mut bytes, &json!({"row": "item", "item": item}))?;
        bytes.push(b'\n');
    }
    Ok(Export { bytes, cwd })
}

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_string)
}

fn text(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(x)) => x.clone(),
        Some(Value::Null) | None => String::new(),
        Some(other) => other.to_string(),
    }
}

pub fn parse(bytes: &[u8]) -> Result<Parsed> {
    let mut out = Parsed::default();
    let raw = String::from_utf8_lossy(bytes);
    let mut model = None;
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if v.get("row").and_then(Value::as_str) == Some("thread") {
            out.session_id = s(&v, "thread_id");
            out.title = s(&v, "title");
            out.cwd = s(&v, "worktree_path").or_else(|| s(&v, "project_root"));
            out.git_branch = s(&v, "branch");
            out.harness_version = s(&v, "provider_instance");
            out.parent_session = v
                .pointer("/lineage/parentThreadId")
                .and_then(Value::as_str)
                .map(str::to_string);
            model = s(&v, "model");
            if let Some(m) = &model {
                out.add_model(m);
            }
            let provider = s(&v, "provider").unwrap_or_default();
            out.mirror = NATIVE.contains(&provider.as_str());
            continue;
        }
        let it = &v["item"];
        let at = s(it, "startedAt").or_else(|| s(it, "updatedAt"));
        let ty = it.get("type").and_then(Value::as_str).unwrap_or("");
        let failed = it.get("status").and_then(Value::as_str) == Some("failed");
        let call = |name: &str, input: Option<Value>| {
            let mut e = Event::new(EventKind::ToolCall).at(at.clone());
            e.tool = Some(name.to_string());
            e.call_id = s(it, "id");
            e.input = input;
            e.model = model.clone();
            e
        };
        let result = |name: &str, body: String, error: bool| {
            let mut e = Event::new(EventKind::ToolResult).at(at.clone());
            e.tool = Some(name.to_string());
            e.call_id = s(it, "id");
            e.text = Some(body);
            e.is_error = Some(error);
            e
        };
        match ty {
            "user_message" => out.push(Event::text(EventKind::User, text(it.get("text"))).at(at)),
            "assistant_message" => {
                let mut e = Event::text(EventKind::Assistant, text(it.get("text"))).at(at);
                e.model = model.clone();
                out.push(e);
            }
            "reasoning" => out.push(Event::text(EventKind::Reasoning, text(it.get("text"))).at(at)),
            "system_notice" | "notification" | "compaction" | "handoff" => out.push(
                Event::text(EventKind::System, text(it.get("text").or(it.get("title")))).at(at),
            ),
            "command_execution" => {
                out.push(call("command", Some(json!({"command": it.get("input")}))));
                let code = it.get("exitCode").and_then(Value::as_i64);
                out.push(result(
                    "command",
                    text(it.get("output")),
                    failed || code.is_some_and(|c| c != 0),
                ));
            }
            "dynamic_tool" => {
                let name = s(it, "toolName").unwrap_or_else(|| "tool".into());
                out.push(call(&name, it.get("input").cloned()));
                let body = it
                    .pointer("/output/content")
                    .map(|c| text(Some(c)))
                    .unwrap_or_else(|| text(it.get("output")));
                out.push(result(&name, body, failed));
            }
            "file_change" => {
                out.push(call(
                    "file_change",
                    Some(json!({"file": it.get("fileName"), "changes": it.get("changes")})),
                ));
                out.push(result("file_change", text(it.get("diffStr")), failed));
            }
            "web_search" => out.push(call("web_search", it.get("patterns").cloned())),
            "file_search" => out.push(call("file_search", None)),
            "subagent" => {
                out.push(call(
                    "subagent",
                    Some(json!({"prompt": it.get("prompt"), "driver": it.get("driver"), "child_thread": it.get("childThreadId")})),
                ));
                out.push(result("subagent", text(it.get("result")), failed));
            }
            "error" => out.push(result("error", text(it.get("failure")), true)),
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path, provider: &str) -> std::path::PathBuf {
        let db = dir.join("statev2.sqlite");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(&format!(
            r#"CREATE TABLE orchestration_v2_projection_threads (thread_id TEXT PRIMARY KEY, project_id TEXT, title TEXT, default_provider TEXT, created_at TEXT, updated_at TEXT, deleted_at TEXT, payload_json TEXT);
               CREATE TABLE projection_projects (project_id TEXT PRIMARY KEY, workspace_root TEXT);
               CREATE TABLE orchestration_v2_projection_provider_threads (provider_thread_id TEXT PRIMARY KEY, thread_id TEXT, provider TEXT, updated_at TEXT);
               CREATE TABLE orchestration_v2_projection_turn_items (turn_item_id TEXT PRIMARY KEY, thread_id TEXT, ordinal INTEGER, payload_json TEXT);
               INSERT INTO projection_projects VALUES ('p1', '/work/app');
               INSERT INTO orchestration_v2_projection_threads VALUES ('t1', 'p1', 'Fix it', 'inst', '2026-01-01T00:00:00Z', '2026-01-01T00:05:00Z', NULL,
                 '{{"title":"Fix it","branch":"main","modelSelection":{{"model":"m1"}},"opens":3}}');
               INSERT INTO orchestration_v2_projection_threads VALUES ('t2', 'p1', 'Live', 'inst', '2999-01-01T00:00:00Z', '2999-01-01T00:00:00Z', NULL, '{{}}');
               INSERT INTO orchestration_v2_projection_provider_threads VALUES ('pt1', 't1', '{provider}', '2026-01-01T00:00:00Z');
               INSERT INTO orchestration_v2_projection_turn_items VALUES ('i1', 't1', 1, '{{"type":"user_message","text":"run tests","startedAt":"2026-01-01T00:00:01Z"}}');
               INSERT INTO orchestration_v2_projection_turn_items VALUES ('i2', 't1', 2, '{{"id":"i2","type":"command_execution","input":"cargo test","output":"1 failed","exitCode":1,"status":"failed"}}');
               INSERT INTO orchestration_v2_projection_turn_items VALUES ('i3', 't1', 3, '{{"type":"assistant_message","text":"One test fails."}}');"#
        ))
        .unwrap();
        db
    }

    #[test]
    fn exports_settled_threads_and_parses_items() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "acpRegistry");
        let threads = list_threads(&db).unwrap();
        assert_eq!(
            threads.len(),
            1,
            "a thread touched in the last minutes waits"
        );
        let export = export_thread(&db, "t1").unwrap();
        assert_eq!(export.cwd.as_deref(), Some("/work/app"));
        let p = parse(&export.bytes).unwrap();
        assert_eq!(p.session_id.as_deref(), Some("t1"));
        assert_eq!(p.models, vec!["m1"]);
        assert!(
            !p.mirror,
            "an ACP agent has no native store: this is the only copy"
        );
        let kinds: Vec<_> = p.events.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            vec![
                EventKind::User,
                EventKind::ToolCall,
                EventKind::ToolResult,
                EventKind::Assistant
            ]
        );
        assert_eq!(p.events[2].is_error, Some(true));
        // The header holds no fields that change while a thread is open (like `opens`).
        assert!(
            !String::from_utf8_lossy(&export.bytes)
                .lines()
                .next()
                .unwrap()
                .contains("opens")
        );
    }

    #[test]
    fn threads_from_natively_captured_harnesses_are_mirrors() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "codex");
        let p = parse(&export_thread(&db, "t1").unwrap().bytes).unwrap();
        assert!(p.mirror);
    }
}
