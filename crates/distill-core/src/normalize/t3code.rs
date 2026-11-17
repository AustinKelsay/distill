//! T3 Code keeps every thread it runs, whatever the harness, in one SQLite database
//! (`~/.t3/userdata/statev2.sqlite`). The sweep exports each thread as JSONL: one header line of
//! fields that don't change as the thread runs, then its turn items in order, so a thread that
//! grows appends. Threads touched in the last few minutes wait for the next sweep.
//!
//! A thread whose harness Distill also captures natively (Codex, Claude Code, …) is a mirror of
//! that session: archived, but it gets no card and is left out of search unless asked for, so
//! nothing is counted twice. Threads from harnesses with no local store of their own are the
//! only copy.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

use crate::harness::{Candidate, Harness};
use crate::model::{Event, EventKind, Parsed};

/// Item types the parser skips on purpose. Any other type it skips is reported as format drift.
pub const IGNORED: &[&str] = &[];

/// The harness Distill reads a T3 Code driver's sessions from directly, if any.
fn native(driver: &str) -> Option<Harness> {
    Some(match driver {
        "codex" => Harness::Codex,
        "claudeAgent" => Harness::ClaudeCode,
        "opencode" => Harness::OpenCode,
        "pi" => Harness::Pi,
        "grok" => Harness::Grok,
        "antigravity" => Harness::Antigravity,
        "droid" => Harness::Droid,
        // Its agents run through Cursor's SDK, whose stores Distill reads.
        "cursor" => Harness::Cursor,
        _ => return None,
    })
}

/// Suffix of the origin that holds a thread's native session references (see
/// [`export_native_refs`]), archived beside the thread's own export.
pub const NATIVE_SUFFIX: &str = "#native";

/// The harness session behind each run of every thread (`nativeThreadRef.nativeId`), one JSON
/// line per run in order, keyed by thread. A run with no provider-thread row names its
/// provider, as `export_thread` does, so it still counts (as unresolved). Read in one query each sweep, so a native id that is
/// filled in later is picked up even when the thread itself didn't change. Kept apart from the
/// thread's export so that export keeps growing by appending; this is what matches a mirror to
/// its original.
pub fn export_native_refs(db: &Path) -> Result<HashMap<String, Vec<u8>>> {
    let conn = open(db)?;
    let mut stmt = conn.prepare(
        "SELECT r.thread_id, COALESCE(pt.driver, r.provider), pt.payload_json
         FROM orchestration_v2_projection_runs r
         LEFT JOIN orchestration_v2_projection_provider_threads pt
           ON pt.provider_thread_id = r.provider_thread_id
         ORDER BY r.thread_id, r.ordinal, r.run_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<String>>(1)?,
            r.get::<_, Option<String>>(2)?,
        ))
    })?;
    let mut out: HashMap<String, Vec<u8>> = HashMap::new();
    for row in rows {
        let (thread_id, driver, payload) = row?;
        let native_id = payload
            .and_then(|p| serde_json::from_str::<Value>(&p).ok())
            .and_then(|p| s(p.get("nativeThreadRef")?, "nativeId"));
        let buf = out.entry(thread_id).or_default();
        serde_json::to_writer(
            &mut *buf,
            &json!({"driver": driver, "native_id": native_id}),
        )?;
        buf.push(b'\n');
    }
    Ok(out)
}

/// Where a thread ran, for privacy exclusions: its worktree, else its project's root.
pub fn thread_cwd(db: &Path, thread_id: &str) -> Result<Option<String>> {
    let conn = open(db)?;
    let (payload, root): (String, Option<String>) = conn.query_row(
        "SELECT t.payload_json, p.workspace_root
         FROM orchestration_v2_projection_threads t
         LEFT JOIN projection_projects p ON p.project_id = t.project_id
         WHERE t.thread_id = ?1",
        [thread_id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let t: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
    Ok(where_ran(&t, root))
}

/// A thread's worktree, else its project's root: whichever is a usable absolute path.
fn where_ran(thread: &Value, project_root: Option<String>) -> Option<String> {
    s(thread, "worktreePath")
        .into_iter()
        .chain(project_root)
        .find(|p| p.starts_with('/'))
}

/// What a thread's native references say about the sessions it copies.
#[derive(Debug, Default, PartialEq)]
pub struct NativeRefs {
    /// Named originals, as `harness/session id` keys.
    pub keys: Vec<String>,
    /// Harnesses of runs that name no original.
    pub unresolved: Vec<Harness>,
}

/// Read [`export_native_refs`] output for one thread. Runs on harnesses Distill doesn't read
/// are skipped: such a thread is not a mirror at all.
pub fn native_refs(refs: &[u8]) -> NativeRefs {
    let mut out = NativeRefs::default();
    for line in String::from_utf8_lossy(refs).lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let Some(kind) = s(&v, "driver").as_deref().and_then(native) else {
            continue;
        };
        let Some(id) = s(&v, "native_id").filter(|i| !i.is_empty()) else {
            out.unresolved.push(kind);
            continue;
        };
        // Pi names its session by file (`…/<time>_<session id>.jsonl`).
        let id = match kind {
            Harness::Pi => id
                .rsplit('/')
                .next()
                .and_then(|f| f.strip_suffix(".jsonl"))
                .map(|f| f.rsplit('_').next().unwrap_or(f).to_string())
                .unwrap_or(id),
            _ => id,
        };
        out.keys.push(format!("{}/{id}", kind.id()));
    }
    out.keys.sort();
    out.keys.dedup();
    out.unresolved.sort_by_key(|k| k.id());
    out.unresolved.dedup();
    out
}

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
    let cwd = where_ran(&t, project_root.clone());
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
            _ => out.skip_in("item", ty, it),
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
    let natives: Option<Vec<Harness>> = drivers.iter().map(|d| native(d)).collect();
    if let Some(mut kinds) = natives.filter(|k| !k.is_empty()) {
        kinds.sort_by_key(|k| k.id());
        kinds.dedup();
        out.mirror_of = kinds;
    }
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
               CREATE TABLE orchestration_v2_projection_provider_threads (provider_thread_id TEXT PRIMARY KEY, thread_id TEXT, provider TEXT, driver TEXT, updated_at TEXT, payload_json TEXT);
               CREATE TABLE orchestration_v2_projection_runs (run_id TEXT PRIMARY KEY, thread_id TEXT, ordinal INTEGER, provider TEXT, provider_thread_id TEXT, payload_json TEXT);
               CREATE TABLE orchestration_v2_projection_turn_items (turn_item_id TEXT PRIMARY KEY, thread_id TEXT, ordinal INTEGER, payload_json TEXT);
               INSERT INTO projection_projects VALUES ('p1', '/work/app');
               INSERT INTO orchestration_v2_projection_threads VALUES ('t1', 'p1', 'Fix it', '{instance}', '2026-01-01T00:00:00Z', '2026-01-01T00:05:00Z', NULL,
                 '{{"title":"Fix it","branch":"main","modelSelection":{{"model":"m1"}},"opens":3}}');
               INSERT INTO orchestration_v2_projection_threads VALUES ('t2', 'p1', 'Live', 'x', '2999-01-01T00:00:00Z', '2999-01-01T00:00:00Z', NULL, '{{}}');
               INSERT INTO orchestration_v2_projection_threads VALUES ('t3', 'p1', 'Gone', 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', '2026-01-02T00:00:00Z', '{{}}');
               INSERT INTO orchestration_v2_projection_provider_threads VALUES ('pt1', 't1', '{instance}', '{driver}', '2026-01-01T00:00:00Z',
                 '{{"nativeThreadRef":{{"driver":"{driver}","nativeId":"native-1"}}}}');
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
            !p.is_mirror(),
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
    fn exports_the_native_session_behind_each_run() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "claudeAgent_2", "claudeAgent");
        let refs = export_native_refs(&db).unwrap();
        assert_eq!(native_refs(&refs["t1"]).keys, vec!["claude-code/native-1"]);
        assert!(
            !refs.contains_key("t3"),
            "a thread with no runs has no references"
        );
        assert_eq!(thread_cwd(&db, "t1").unwrap().as_deref(), Some("/work/app"));
    }

    #[test]
    fn sweeps_refresh_references_but_not_for_excluded_threads() {
        use crate::archive::Archive;
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "codex", "codex");
        let conn = Connection::open(&db).unwrap();
        conn.execute(
            "UPDATE orchestration_v2_projection_provider_threads SET payload_json = '{}'",
            [],
        )
        .unwrap();
        let home = crate::Home::new(dir.path().join("home"));
        let mut config = crate::Config::default();
        for s in &mut config.harnesses {
            s.enabled = s.kind == Harness::T3Code;
            s.roots = vec![db.clone()];
        }
        let refs_of = |home: &crate::Home| {
            let a = Archive::open(home).unwrap();
            let r = a
                .latest()
                .iter()
                .find(|(o, _)| o.ends_with(NATIVE_SUFFIX))
                .map(|(_, r)| r.capture_hash.clone())
                .unwrap();
            native_refs(&a.read_blob(&r).unwrap())
        };
        crate::sweep::sweep(&home, &config, None).unwrap();
        assert_eq!(refs_of(&home).unresolved, vec![Harness::Codex]);

        // The native id is filled in later; the thread itself doesn't change.
        conn.execute(
            r#"UPDATE orchestration_v2_projection_provider_threads
               SET payload_json = '{"nativeThreadRef":{"nativeId":"n-1"}}'"#,
            [],
        )
        .unwrap();
        let r = crate::sweep::sweep(&home, &config, None).unwrap();
        assert_eq!(refs_of(&home).keys, vec!["codex/n-1"]);
        assert!(r.bytes_written > 0, "the reference capture is counted");

        // Excluded after it was captured: nothing new is written for it.
        conn.execute(
            r#"UPDATE orchestration_v2_projection_provider_threads
               SET payload_json = '{"nativeThreadRef":{"nativeId":"n-2"}}'"#,
            [],
        )
        .unwrap();
        config.privacy.exclude_paths = vec!["/work/app".into()];
        crate::sweep::sweep(&home, &config, None).unwrap();
        assert_eq!(refs_of(&home).keys, vec!["codex/n-1"]);

        // Nor once it no longer says where it ran (project gone, relative worktree).
        config.privacy.exclude_paths = vec!["/elsewhere".into()];
        conn.execute_batch(
            r#"DELETE FROM projection_projects;
               UPDATE orchestration_v2_projection_threads SET payload_json = '{"worktreePath":"rel/dir"}';"#,
        )
        .unwrap();
        crate::sweep::sweep(&home, &config, None).unwrap();
        assert_eq!(refs_of(&home).keys, vec!["codex/n-1"]);
    }

    #[test]
    fn a_run_without_a_provider_thread_still_counts() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "codex", "codex");
        Connection::open(&db)
            .unwrap()
            .execute_batch(
                r#"INSERT INTO orchestration_v2_projection_runs VALUES ('r2', 't1', 2, 'claudeAgent', NULL, '{}');"#,
            )
            .unwrap();
        let refs = native_refs(&export_native_refs(&db).unwrap()["t1"]);
        assert_eq!(refs.keys, vec!["codex/native-1"]);
        assert_eq!(refs.unresolved, vec![Harness::ClaudeCode]);
    }

    #[test]
    fn native_refs_name_the_sessions_a_mirror_copies() {
        let refs = br#"{"driver":"codex","native_id":"01a1-c"}
{"driver":"pi","native_id":"/h/.pi/agent/sessions/--w--/2026-10-10T21-47-05-111Z_01a1-p.jsonl"}
{"driver":"claudeAgent","native_id":"c-1"}
{"driver":"codex","native_id":"01a1-c"}
"#;
        assert_eq!(
            native_refs(refs).keys,
            vec!["claude-code/c-1", "codex/01a1-c", "pi/01a1-p"]
        );
        // A run that names no session leaves the others' names in place.
        let partial = native_refs(
            br#"{"driver":"claudeAgent","native_id":"c-1"}
{"driver":"claudeAgent","native_id":null}"#,
        );
        assert_eq!(partial.keys, vec!["claude-code/c-1"]);
        assert_eq!(partial.unresolved, vec![Harness::ClaudeCode]);
        assert_eq!(native_refs(b""), NativeRefs::default());
    }

    #[test]
    fn threads_without_runs_fall_back_to_their_provider_instance() {
        let mirror = r#"{"row":"thread","thread_id":"t8","provider_instance":"claudeAgent_2"}
{"row":"item","item":{"type":"user_message","text":"hi"}}"#;
        assert_eq!(
            parse(mirror.as_bytes()).unwrap().mirror_of,
            vec![Harness::ClaudeCode]
        );
        let own = r#"{"row":"thread","thread_id":"t7","provider_instance":"fragment"}
{"row":"item","item":{"type":"user_message","text":"hi"}}"#;
        assert!(!parse(own.as_bytes()).unwrap().is_mirror());
    }

    #[test]
    fn exports_in_the_earlier_header_format_still_classify() {
        let old = r#"{"row":"thread","thread_id":"t9","title":"Old","provider":"codex","model":"gpt-6","project_root":"/w"}
{"row":"item","item":{"type":"user_message","text":"hi"}}"#;
        let p = parse(old.as_bytes()).unwrap();
        assert!(p.is_mirror());
        assert_eq!(p.mirror_of, vec![Harness::Codex]);
        assert_eq!(p.models, vec!["gpt-6"]);
    }

    #[test]
    fn mirrors_follow_the_driver_not_the_instance_name() {
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "claudeAgent_2", "claudeAgent");
        assert!(
            parse(&export_thread(&db, "t1").unwrap().bytes)
                .unwrap()
                .is_mirror()
        );
        let dir = tempfile::tempdir().unwrap();
        let db = fixture(dir.path(), "maple", "pi");
        assert!(
            parse(&export_thread(&db, "t1").unwrap().bytes)
                .unwrap()
                .is_mirror()
        );
    }
}
