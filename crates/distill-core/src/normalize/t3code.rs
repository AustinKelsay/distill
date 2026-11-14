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

/// Soft-deleted threads are included: their content is still in the database, and this may be
/// the last chance to keep it.
pub fn list_threads(db: &Path) -> Result<Vec<Candidate>> {
    let conn = open(db)?;
    let cutoff = (chrono::Utc::now() - chrono::Duration::minutes(SETTLE_MINUTES))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut stmt = conn.prepare(
        "SELECT thread_id, updated_at FROM orchestration_v2_projection_threads
         WHERE updated_at < ?1 ORDER BY thread_id",
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
    let (payload, created_at, project_root, instance): (String, String, Option<String>, String) =
        conn.query_row(
            "SELECT t.payload_json, t.created_at, p.workspace_root, t.default_provider
         FROM orchestration_v2_projection_threads t
         LEFT JOIN projection_projects p ON p.project_id = t.project_id
         WHERE t.thread_id = ?1",
            [thread_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?;
    let t: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
    let cwd = t
        .get("worktreePath")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or(project_root.clone());
    // Only fields set when the thread is made, so the export grows by appending. Title and
    // model change while a thread lives; the model is recorded per run below.
    let header = json!({
        "row": "thread",
        "thread_id": thread_id,
        "created_at": created_at,
        // The provider instance the thread was made with; the fallback for threads whose runs
        // carry no provider (older threads imported into T3's current schema).
        "provider_instance": instance,
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

    // Each run: the model it asked for and the harness driver behind its provider instance
    // (`claudeAgent_2` runs on `claudeAgent`). Written just before the run's first item.
    let mut runs = std::collections::HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT r.run_id, r.ordinal, r.provider, r.payload_json, pt.driver
         FROM orchestration_v2_projection_runs r
         LEFT JOIN orchestration_v2_projection_provider_threads pt
           ON pt.provider_thread_id = r.provider_thread_id
         WHERE r.thread_id = ?1",
    )?;
    let rows = stmt.query_map([thread_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, Option<String>>(4)?,
        ))
    })?;
    for row in rows {
        let (run_id, ordinal, instance, payload, driver) = row?;
        let p: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
        runs.insert(
            run_id.clone(),
            json!({
                "row": "run",
                "run_id": run_id,
                "ordinal": ordinal,
                "provider_instance": instance,
                "driver": driver.unwrap_or_else(|| instance.clone()),
                "model": p.pointer("/modelSelection/model"),
            }),
        );
    }
    let mut stmt = conn.prepare(
        "SELECT payload_json FROM orchestration_v2_projection_turn_items
         WHERE thread_id = ?1 ORDER BY ordinal, turn_item_id",
    )?;
    let rows = stmt.query_map([thread_id], |r| r.get::<_, String>(0))?;
    for row in rows {
        let item: Value = serde_json::from_str(&row?).unwrap_or(Value::Null);
        if let Some(run) = item
            .get("runId")
            .and_then(Value::as_str)
            .and_then(|id| runs.remove(id))
        {
            serde_json::to_writer(&mut bytes, &run)?;
            bytes.push(b'\n');
        }
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
    let mut drivers: Vec<String> = Vec::new();
    let mut thread_instance: Option<String> = None;
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match v.get("row").and_then(Value::as_str) {
            Some("thread") => {
                out.session_id = s(&v, "thread_id");
                out.cwd = s(&v, "worktree_path").or_else(|| s(&v, "project_root"));
                out.git_branch = s(&v, "branch");
                out.parent_session = v
                    .pointer("/lineage/parentThreadId")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                // Exports made before per-run lines kept the provider and model in the header.
                // They stay in the archive until the thread changes, so read them too.
                if let Some(p) = s(&v, "provider") {
                    drivers.push(p);
                }
                if let Some(m) = s(&v, "model") {
                    model = Some(m.clone());
                    out.add_model(&m);
                }
                if out.title.is_none() {
                    out.title = s(&v, "title");
                }
                thread_instance = s(&v, "provider_instance");
                continue;
            }
            Some("run") => {
                model = s(&v, "model");
                if let Some(m) = &model {
                    out.add_model(m);
                }
                let driver = s(&v, "driver").unwrap_or_default();
                out.harness_version = s(&v, "provider_instance");
                drivers.push(driver);
                continue;
            }
            _ => {}
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
    // A mirror only when every run went through a harness Distill reads natively; a thread
    // with any other run holds content found nowhere else.
    if drivers.is_empty()
        && let Some(inst) = &thread_instance
    {
        // `claudeAgent_2` is a second Claude Code instance.
        let base = inst
            .rsplit_once('_')
            .filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit()));
        drivers.push(base.map_or(inst.as_str(), |(b, _)| b).to_string());
        if out.harness_version.is_none() {
            out.harness_version = Some(inst.clone());
        }
    }
    out.mirror = !drivers.is_empty() && drivers.iter().all(|d| NATIVE.contains(&d.as_str()));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(dir: &Path, instance: &str, driver: &str) -> std::path::PathBuf {
        let db = dir.join("statev2.sqlite");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(&format!(
            r#"CREATE TABLE orchestration_v2_projection_threads (thread_id TEXT PRIMARY KEY, project_id TEXT, title TEXT, default_provider TEXT, created_at TEXT, updated_at TEXT, deleted_at TEXT, payload_json TEXT);
               CREATE TABLE projection_projects (project_id TEXT PRIMARY KEY, workspace_root TEXT);
               CREATE TABLE orchestration_v2_projection_provider_threads (provider_thread_id TEXT PRIMARY KEY, thread_id TEXT, provider TEXT, driver TEXT, updated_at TEXT);
               CREATE TABLE orchestration_v2_projection_runs (run_id TEXT PRIMARY KEY, thread_id TEXT, ordinal INTEGER, provider TEXT, provider_thread_id TEXT, payload_json TEXT);
               CREATE TABLE orchestration_v2_projection_turn_items (turn_item_id TEXT PRIMARY KEY, thread_id TEXT, ordinal INTEGER, payload_json TEXT);
               INSERT INTO projection_projects VALUES ('p1', '/work/app');
               INSERT INTO orchestration_v2_projection_threads VALUES ('t1', 'p1', 'Fix it', '{instance}', '2026-01-01T00:00:00Z', '2026-01-01T00:05:00Z', NULL,
                 '{{"title":"Fix it","branch":"main","modelSelection":{{"model":"m1"}},"opens":3}}');
               INSERT INTO orchestration_v2_projection_threads VALUES ('t2', 'p1', 'Live', 'x', '2999-01-01T00:00:00Z', '2999-01-01T00:00:00Z', NULL, '{{}}');
               INSERT INTO orchestration_v2_projection_threads VALUES ('t3', 'p1', 'Gone', 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', '{{}}');
               INSERT INTO orchestration_v2_projection_provider_threads VALUES ('pt1', 't1', '{instance}', '{driver}', '2026-01-01T00:00:00Z');
               INSERT INTO orchestration_v2_projection_runs VALUES ('r1', 't1', 1, '{instance}', 'pt1', '{{"modelSelection":{{"model":"m1"}}}}');
               INSERT INTO orchestration_v2_projection_turn_items VALUES ('i1', 't1', 1, '{{"runId":"r1","type":"user_message","text":"run tests","startedAt":"2026-01-01T00:00:01Z"}}');
               INSERT INTO orchestration_v2_projection_turn_items VALUES ('i2', 't1', 2, '{{"runId":"r1","id":"i2","type":"command_execution","input":"cargo test","output":"1 failed","exitCode":1,"status":"failed"}}');
               INSERT INTO orchestration_v2_projection_turn_items VALUES ('i3', 't1', 3, '{{"runId":"r1","type":"assistant_message","text":"One test fails."}}');"#
        ))
        .unwrap();
        db
    }

    #[test]
    fn exports_settled_threads_and_parses_items() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "fragment", "acpRegistry");
        let ids: Vec<String> = list_threads(&db)
            .unwrap()
            .into_iter()
            .map(|c| c.origin())
            .collect();
        assert_eq!(
            ids.len(),
            2,
            "t2 is still active; soft-deleted t3 is kept: {ids:?}"
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
    }

    #[test]
    fn a_renamed_thread_resumed_on_another_model_appends() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "claudeAgent", "claudeAgent");
        let before = export_thread(&db, "t1").unwrap().bytes;
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch(
            r#"UPDATE orchestration_v2_projection_threads SET title = 'Renamed',
                 payload_json = '{"title":"Renamed","branch":"main","modelSelection":{"model":"m2"},"opens":9}' WHERE thread_id = 't1';
               INSERT INTO orchestration_v2_projection_runs VALUES ('r2', 't1', 2, 'claudeAgent', 'pt1', '{"modelSelection":{"model":"m2"}}');
               INSERT INTO orchestration_v2_projection_turn_items VALUES ('i4', 't1', 4, '{"runId":"r2","type":"user_message","text":"again"}');"#,
        )
        .unwrap();
        let after = export_thread(&db, "t1").unwrap().bytes;
        assert!(after.starts_with(&before), "the export grew by appending");
        let p = parse(&after).unwrap();
        assert_eq!(p.models, vec!["m1", "m2"]);
    }

    #[test]
    fn threads_without_runs_fall_back_to_their_provider_instance() {
        let mirror = r#"{"row":"thread","thread_id":"t8","provider_instance":"claudeAgent_2"}
{"row":"item","item":{"type":"user_message","text":"hi"}}"#;
        assert!(parse(mirror.as_bytes()).unwrap().mirror);
        let own = r#"{"row":"thread","thread_id":"t7","provider_instance":"cursor"}
{"row":"item","item":{"type":"user_message","text":"hi"}}"#;
        assert!(!parse(own.as_bytes()).unwrap().mirror);
    }

    #[test]
    fn exports_in_the_earlier_header_format_still_classify() {
        let old = r#"{"row":"thread","thread_id":"t9","title":"Old","provider":"codex","model":"gpt-6","project_root":"/w"}
{"row":"item","item":{"type":"user_message","text":"hi"}}"#;
        let p = parse(old.as_bytes()).unwrap();
        assert!(p.mirror);
        assert_eq!(p.models, vec!["gpt-6"]);
        assert_eq!(p.title.as_deref(), Some("Old"));
    }

    #[test]
    fn mirrors_follow_the_driver_not_the_instance_name() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "claudeAgent_2", "claudeAgent");
        assert!(
            parse(&export_thread(&db, "t1").unwrap().bytes)
                .unwrap()
                .mirror
        );
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "maple", "pi");
        assert!(
            parse(&export_thread(&db, "t1").unwrap().bytes)
                .unwrap()
                .mirror
        );
    }
}
