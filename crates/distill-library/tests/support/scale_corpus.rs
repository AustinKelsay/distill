//! Deterministic synthetic Distill-home corpus helpers for `library_scale_budgets`.
//!
//! Seeds real SQLite schema + FTS projection rows via bulk SQL only. Measured work
//! must still go through the public `Library` API. Sparse padding is benchmark-owned
//! and never allocated as a dense artifact.

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{params, Connection};

/// Fixed corpus seed used by both smoke and full scale runs.
pub const SCALE_SEED: u64 = 34;

/// Warm p95 budgets from `docs/specs/scale-and-latency.md` (milliseconds).
pub const BUDGET_LIST_MS: f64 = 150.0;
/// Warm p95 budget for current-projection session search pages.
pub const BUDGET_SEARCH_MS: f64 = 150.0;
/// Warm p95 budget for the first Session detail slice.
pub const BUDGET_DETAIL_MS: f64 = 150.0;
/// Warm p95 budget for one manual curation mutation.
pub const BUDGET_CURATION_MS: f64 = 100.0;

/// Maximum observed Library progress callback gap.
pub const PROGRESS_GAP_BUDGET: Duration = Duration::from_millis(500);
/// Maximum Sync/export cancellation acknowledgement latency.
pub const CANCEL_ACK_BUDGET: Duration = Duration::from_secs(1);

/// Full-corpus targets from the scale contract.
pub const FULL_SESSION_TARGET: u64 = 25_000;
/// Full-corpus current-projection message target.
pub const FULL_MESSAGE_TARGET: u64 = 1_000_000;
/// Full-corpus logical Distill-home size target.
pub const FULL_LOGICAL_BYTES: u64 = 10 * 1024 * 1024 * 1024;

/// Stable FTS probe token embedded in a deterministic subset of synthetic messages.
pub const SEARCH_PROBE: &str = "scaleprobe";

/// Probe appears in the first message of every 97th Session so search remains
/// selective while still spanning the full-size corpus deterministically.
pub const SEARCH_PROBE_SESSION_STRIDE: u64 = 97;

/// Source kind used for synthetic Session Identities.
pub const SCALE_SOURCE_KIND: &str = "scale";

/// Corpus sizing knobs shared by smoke and full generators.
#[derive(Clone, Copy, Debug)]
pub struct CorpusSpec {
    /// Fixed generator seed reported in evidence JSON.
    pub seed: u64,
    /// Number of Session rows to insert.
    pub session_count: u64,
    /// Number of current-projection message rows to insert.
    pub message_count: u64,
    /// Minimum logical Distill-home byte size after sparse padding.
    pub min_logical_bytes: u64,
}

/// Counts and logical size observed after seeding.
#[derive(Clone, Debug)]
pub struct CorpusReport {
    /// Generator seed.
    pub seed: u64,
    /// Session rows present.
    pub session_count: u64,
    /// Current-projection message rows present.
    pub message_count: u64,
    /// Sum of file metadata lengths under the Distill home.
    pub logical_bytes: u64,
    /// First synthetic Session external id for detail/curation samples.
    pub first_external_session_id: String,
}

/**
 * Bounded smoke corpus: proves generator/API wiring without claiming full budgets.
 */
pub fn smoke_spec() -> CorpusSpec {
    CorpusSpec {
        seed: SCALE_SEED,
        session_count: 40,
        message_count: 200,
        min_logical_bytes: 0,
    }
}

/**
 * Env-gated full corpus targets from the scale-and-latency contract.
 */
pub fn full_spec() -> CorpusSpec {
    CorpusSpec {
        seed: SCALE_SEED,
        session_count: FULL_SESSION_TARGET,
        message_count: FULL_MESSAGE_TARGET,
        min_logical_bytes: FULL_LOGICAL_BYTES,
    }
}

/**
 * Open or create a Distill home through `Library::open`, then drop the handle.
 *
 * Parameters:
 * - `home`: temporary Distill home path.
 */
pub fn bootstrap_home(home: &Path) {
    let library = distill_library::Library::open(home).expect("bootstrap Library::open");
    drop(library);
}

/**
 * Seed sessions, current projection messages, and FTS rows with a fixed seed.
 *
 * Uses bulk SQL only. Callers must bootstrap migrations first and measure only
 * through public Library APIs afterward.
 *
 * Parameters:
 * - `home`: Distill home that already has a migrated `distill.db`.
 * - `spec`: deterministic corpus sizing.
 */
pub fn seed_projection_corpus(home: &Path, spec: CorpusSpec) -> CorpusReport {
    assert!(spec.session_count > 0, "session_count must be > 0");
    assert!(spec.message_count > 0, "message_count must be > 0");
    assert!(
        spec.message_count.is_multiple_of(spec.session_count),
        "message_count must divide evenly across sessions"
    );

    let db_path = home.join("distill.db");
    let mut conn = Connection::open(&db_path).expect("open distill.db for scale seed");
    conn.execute_batch(
        "PRAGMA foreign_keys = ON;
         PRAGMA synchronous = OFF;
         PRAGMA journal_mode = MEMORY;",
    )
    .expect("seed pragmas");

    let messages_per_session = spec.message_count / spec.session_count;
    let mut rng = Lcg::new(spec.seed);
    let first_external_session_id = session_external_id(spec.seed, 0);

    let tx = conn.transaction().expect("seed tx");
    let mut insert_session = tx
        .prepare(
            "INSERT INTO sessions (
                source_kind, external_session_id, title, project_path, summary,
                started_at, updated_at, metadata_json,
                accepted_capture_count, normalization_attempt_count,
                successful_projection_generation
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, '{}', 1, 1, 1)",
        )
        .expect("prepare session insert");
    let mut insert_message = tx
        .prepare(
            "INSERT INTO projection_messages (
                session_id, projection_generation, ordinal, role, message_kind, text,
                external_message_id, created_at, metadata_json
             ) VALUES (?1, 1, ?2, ?3, 'text', ?4, ?5, ?6, '{}')",
        )
        .expect("prepare message insert");
    let mut insert_fts = tx
        .prepare(
            "INSERT INTO projection_fts (session_id, message_id, title, project_path, role, text)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .expect("prepare fts insert");

    for session_index in 0..spec.session_count {
        let external_session_id = session_external_id(spec.seed, session_index);
        let title = format!("Scale Session {session_index}");
        let project_path = format!("/scale/project/{}", session_index % 97);
        let stamp = deterministic_stamp(spec.seed, session_index);
        insert_session
            .execute(params![
                SCALE_SOURCE_KIND,
                external_session_id,
                title,
                project_path,
                format!("summary {session_index}"),
                stamp,
                stamp,
            ])
            .expect("insert session");
        let session_id = tx.last_insert_rowid();

        for ordinal in 0..messages_per_session {
            let role = if ordinal % 2 == 0 {
                "user"
            } else {
                "assistant"
            };
            let nonce = rng.next();
            let probe = if session_index % SEARCH_PROBE_SESSION_STRIDE == 0 && ordinal == 0 {
                format!("{SEARCH_PROBE} ")
            } else {
                String::new()
            };
            let text =
                format!("{probe}scale session-{session_index} msg-{ordinal} nonce-{nonce:#x}");
            let message_stamp = deterministic_stamp(spec.seed, session_index * 1_000 + ordinal);
            insert_message
                .execute(params![
                    session_id,
                    ordinal as i64,
                    role,
                    text,
                    format!("msg-{session_index}-{ordinal}"),
                    message_stamp,
                ])
                .expect("insert message");
            let message_id = tx.last_insert_rowid();
            insert_fts
                .execute(params![
                    session_id,
                    message_id,
                    title,
                    project_path,
                    role,
                    text,
                ])
                .expect("insert fts");
        }
    }

    drop(insert_session);
    drop(insert_message);
    drop(insert_fts);
    tx.commit().expect("commit scale seed");
    drop(conn);

    ensure_sparse_padding(home, spec.min_logical_bytes);

    let report = observe_corpus(home, spec.seed, &first_external_session_id);
    assert_eq!(
        report.session_count, spec.session_count,
        "session count drift"
    );
    assert_eq!(
        report.message_count, spec.message_count,
        "message count drift"
    );
    assert!(
        report.logical_bytes >= spec.min_logical_bytes,
        "logical home size {} < {}",
        report.logical_bytes,
        spec.min_logical_bytes
    );
    report
}

/**
 * Read session/message counts and logical home size from a seeded home.
 *
 * Parameters:
 * - `home`: Distill home path.
 * - `seed`: fixed seed to echo in the report.
 * - `first_external_session_id`: identity used by detail/curation samples.
 */
pub fn observe_corpus(home: &Path, seed: u64, first_external_session_id: &str) -> CorpusReport {
    let conn = Connection::open(home.join("distill.db")).expect("observe db");
    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .expect("count sessions");
    let message_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM projection_messages WHERE projection_generation = 1",
            [],
            |row| row.get(0),
        )
        .expect("count messages");
    CorpusReport {
        seed,
        session_count: session_count as u64,
        message_count: message_count as u64,
        logical_bytes: logical_home_bytes(home),
        first_external_session_id: first_external_session_id.to_string(),
    }
}

/**
 * Assign a dataset label to the first `count` synthetic sessions for export tests.
 *
 * This is benchmark setup only; the measured export still runs through `Library`.
 */
pub fn assign_dataset_labels(home: &Path, count: u64, label: &str) {
    let mut conn = Connection::open(home.join("distill.db")).expect("open labels db");
    let tx = conn.transaction().expect("label tx");
    let label_id: i64 = tx
        .query_row("SELECT id FROM labels WHERE name = ?1", [label], |row| {
            row.get(0)
        })
        .expect("dataset label");
    let mut sessions = tx
        .prepare(
            "SELECT id FROM sessions
             WHERE source_kind = ?1
             ORDER BY id
             LIMIT ?2",
        )
        .expect("session ids");
    let ids = sessions
        .query_map(params![SCALE_SOURCE_KIND, count as i64], |row| {
            row.get::<_, i64>(0)
        })
        .expect("session rows")
        .collect::<Result<Vec<_>, _>>()
        .expect("session ids collect");
    drop(sessions);
    for session_id in ids {
        tx.execute(
            "INSERT INTO label_assignments (object_type, object_id, label_id, origin, created_at)
             VALUES ('session', ?1, ?2, 'manual', '2024-01-01T00:00:00Z')",
            params![session_id, label_id],
        )
        .expect("dataset assignment");
    }
    tx.commit().expect("label commit");
}

/**
 * Create or extend a sparse padding file so logical home size meets `min_logical_bytes`.
 *
 * Never allocates a dense 10 GiB artifact; relies on filesystem sparse semantics.
 *
 * Parameters:
 * - `home`: Distill home root.
 * - `min_logical_bytes`: required logical byte floor (0 skips padding).
 */
pub fn ensure_sparse_padding(home: &Path, min_logical_bytes: u64) {
    if min_logical_bytes == 0 {
        return;
    }
    let current = logical_home_bytes(home);
    if current >= min_logical_bytes {
        return;
    }
    let need = min_logical_bytes - current;
    let path = home.join("scale_bench_padding.sparse");
    let file = File::create(&path).expect("create sparse padding");
    file.set_len(need).expect("sparse set_len");
}

/**
 * Sum metadata lengths under a Distill home (logical size, including sparse holes).
 *
 * Parameters:
 * - `home`: Distill home root.
 */
pub fn logical_home_bytes(home: &Path) -> u64 {
    let mut total = 0_u64;
    let mut stack = vec![home.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir).expect("read_dir home");
        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            let meta = entry.metadata().expect("metadata");
            if meta.is_dir() {
                stack.push(path);
            } else if meta.is_file() {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total
}

/**
 * Build a stable machine string without claiming OS page-cache control.
 */
pub fn machine_string() -> String {
    if let Ok(value) = std::env::var("DISTILL_SCALE_MACHINE") {
        if !value.trim().is_empty() {
            return value;
        }
    }
    let uname = std::process::Command::new("uname")
        .args(["-sm"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .filter(|value| !value.is_empty());
    uname.unwrap_or_else(|| format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH))
}

/**
 * Compute a nearest-rank percentile over millisecond samples.
 *
 * Parameters:
 * - `samples_ms`: measured sample durations in milliseconds.
 * - `percentile`: value in `(0, 1]`, typically 0.50 or 0.95.
 */
pub fn percentile_ms(samples_ms: &[f64], percentile: f64) -> f64 {
    assert!(!samples_ms.is_empty(), "percentile requires samples");
    assert!((0.0..=1.0).contains(&percentile), "percentile out of range");
    let mut sorted = samples_ms.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite sample"));
    if percentile <= 0.0 {
        return sorted[0];
    }
    let rank = ((percentile * sorted.len() as f64).ceil() as usize)
        .saturating_sub(1)
        .min(sorted.len() - 1);
    sorted[rank]
}

/**
 * Convert a duration sample vector into milliseconds.
 *
 * Parameters:
 * - `samples`: wall-clock samples.
 */
pub fn durations_to_ms(samples: &[Duration]) -> Vec<f64> {
    samples
        .iter()
        .map(|sample| sample.as_secs_f64() * 1000.0)
        .collect()
}

/**
 * Build actionable JSON when a full-run warm p95 misses its budget.
 *
 * Parameters:
 * - `operation`: measured public operation name.
 * - `class`: `cold` or `warm`.
 * - `p50_ms` / `p95_ms`: observed percentiles.
 * - `budget_ms`: warm budget for the operation.
 * - `corpus`: seeded corpus metadata.
 * - `sample_count`: number of samples used for the percentile.
 */
pub fn budget_failure_json(
    operation: &str,
    class: &str,
    p50_ms: f64,
    p95_ms: f64,
    budget_ms: f64,
    corpus: &CorpusReport,
    sample_count: usize,
) -> String {
    serde_json::json!({
        "operation": operation,
        "class": class,
        "sample_count": sample_count,
        "p50_ms": p50_ms,
        "p95_ms": p95_ms,
        "budget_ms": budget_ms,
        "corpus": {
            "seed": corpus.seed,
            "sessions": corpus.session_count,
            "messages": corpus.message_count,
            "logical_bytes": corpus.logical_bytes,
        },
        "machine": machine_string(),
    })
    .to_string()
}

/**
 * Write a multi-candidate Fixture root for Sync progress/cancel evidence.
 *
 * Parameters:
 * - `root`: empty directory that becomes the Fixture root.
 * - `candidate_count`: number of Capture Candidates (must be >= 3 for multiple checkpoints).
 */
pub fn write_progress_fixture(root: &Path, candidate_count: usize) -> PathBuf {
    assert!(candidate_count >= 3, "need multiple safe checkpoints");
    fs::create_dir_all(root.join("sessions")).expect("fixture sessions");
    let mut captures = Vec::new();
    for index in 0..candidate_count {
        let relative = format!("sessions/c{index:02}.jsonl");
        let user =
            format!(r#"{{"record_type":"message","role":"user","text":"progress user {index}"}}"#);
        let assistant = format!(
            r#"{{"record_type":"message","role":"assistant","text":"progress assistant {index}"}}"#
        );
        let body = format!(
            "{}\n{user}\n{assistant}\n",
            r#"{"record_type":"session_meta","title":"Scale Progress","summary":"progress"}"#,
        );
        fs::write(root.join(&relative), body).expect("write candidate");
        captures.push(format!(
            r#"{{
  "id": "c{index:02}",
  "kind": "file",
  "relative_path": "{relative}",
  "external_session_id": "scale-progress-{index:02}",
  "title": "Scale Progress {index}"
}}"#
        ));
    }
    let manifest = format!(
        "{{\n  \"version\": 1,\n  \"captures\": [\n    {}\n  ]\n}}",
        captures.join(",\n    ")
    );
    fs::write(root.join("distill.fixture.json"), manifest).expect("write manifest");
    root.to_path_buf()
}

/**
 * Deterministic external Session id for a seed/index pair.
 *
 * Parameters:
 * - `seed`: fixed corpus seed.
 * - `session_index`: zero-based session ordinal.
 */
pub fn session_external_id(seed: u64, session_index: u64) -> String {
    format!("scale-{seed:x}-{session_index:08}")
}

/**
 * Deterministic RFC3339 stamp derived from seed and ordinal.
 *
 * Parameters:
 * - `seed`: fixed corpus seed.
 * - `ordinal`: monotonic ordinal within the corpus.
 */
fn deterministic_stamp(seed: u64, ordinal: u64) -> String {
    let seconds = 1_700_000_000_i64
        .saturating_add((seed % 10_000) as i64)
        .saturating_add((ordinal % 500_000) as i64);
    chrono::DateTime::from_timestamp(seconds, 0)
        .expect("deterministic timestamp")
        .to_rfc3339()
}

/// Tiny deterministic LCG so corpus text stays reproducible without extra crates.
struct Lcg {
    state: u64,
}

impl Lcg {
    /**
     * Create a generator from the fixed corpus seed.
     *
     * Parameters:
     * - `seed`: non-zero preferred; zero is accepted and advanced once.
     */
    fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }

    /**
     * Advance and return the next pseudo-random u64.
     */
    fn next(&mut self) -> u64 {
        self.state = self.state.wrapping_mul(6364136223846793005).wrapping_add(1);
        self.state
    }
}
