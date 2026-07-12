//! Library scale and latency harness for issue #34 / SCALE-001..004.
//!
//! Default smoke: fixed-seed small SQLite/FTS corpus through real migrations, public
//! Library measurements for wiring/counts, plus Sync progress-gap and cancel-ack
//! evidence. Full 25k/1M/10 GiB budgets are `#[ignore]` and gated by
//! `DISTILL_SCALE_BENCH=1`. No Criterion/divan, private histories, or committed artifacts.

#[path = "support/scale_corpus.rs"]
mod scale_corpus;

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use distill_library::{
    ExportDataset, ExportProgress, ExportProgressControl, Library, SessionCurationRequest,
    SessionListRequest, SyncProgress, SyncRequest, WorkflowLane,
};
use scale_corpus::{
    assign_dataset_labels, bootstrap_home, budget_failure_json, durations_to_ms, full_spec,
    machine_string, percentile_ms, seed_projection_corpus, smoke_spec, write_progress_fixture,
    CorpusReport, BUDGET_CURATION_MS, BUDGET_DETAIL_MS, BUDGET_LIST_MS, BUDGET_SEARCH_MS,
    CANCEL_ACK_BUDGET, PROGRESS_GAP_BUDGET, SCALE_SOURCE_KIND, SEARCH_PROBE,
};
use tempfile::TempDir;

const SMOKE_WARM_SAMPLES: usize = 5;
const FULL_WARM_SAMPLES: usize = 21;

/**
 * SCALE-001/002 smoke: deterministic small corpus + public API wiring/counts.
 * Does not claim 25k/1M/10 GiB or warm p95 budgets.
 */
#[test]
fn scale_smoke_corpus_and_public_api_wiring() {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    bootstrap_home(&home);
    let corpus = seed_projection_corpus(&home, smoke_spec());
    assert_eq!(corpus.session_count, smoke_spec().session_count);
    assert_eq!(corpus.message_count, smoke_spec().message_count);

    eprintln!(
        "{}",
        measure_operations(&home, &corpus, SMOKE_WARM_SAMPLES, false)
    );

    let library = Library::open(&home).expect("open");
    let list = library
        .list_sessions(SessionListRequest {
            limit: 50,
            lane: WorkflowLane::All,
            query: None,
            cursor: None,
        })
        .expect("list");
    assert!(!list.items.is_empty());

    let search = library
        .list_sessions(SessionListRequest {
            limit: 50,
            lane: WorkflowLane::All,
            query: Some(SEARCH_PROBE.to_string()),
            cursor: None,
        })
        .expect("search");
    assert!(!search.items.is_empty());

    let detail = library
        .session_slice(SCALE_SOURCE_KIND, &corpus.first_external_session_id, 20, 20)
        .expect("detail")
        .expect("session");
    assert!(!detail.messages.is_empty());
    assert_eq!(detail.summary.source_kind, SCALE_SOURCE_KIND);
}

/**
 * SCALE-003/004 smoke: Library Sync progress cadence and safe-checkpoint cancel ack.
 */
#[test]
fn scale_smoke_progress_cadence_and_cancel_ack() {
    assert_sync_progress_and_cancel();
}

/**
 * Full SCALE-001/002 budget run: 25k sessions, 1M messages, >=10 GiB logical home.
 * Ignored by default. Requires `DISTILL_SCALE_BENCH=1` and `--ignored`.
 */
#[test]
#[ignore = "full scale bench; run with DISTILL_SCALE_BENCH=1 -- --ignored --nocapture"]
fn scale_full_corpus_latency_budgets() {
    if std::env::var("DISTILL_SCALE_BENCH").ok().as_deref() != Some("1") {
        panic!(
            "full scale bench selected without DISTILL_SCALE_BENCH=1: {}",
            serde_json::json!({
                "error": "DISTILL_SCALE_BENCH_required",
                "reason": "DISTILL_SCALE_BENCH!=1",
                "machine": machine_string(),
            })
        );
    }

    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    bootstrap_home(&home);
    let corpus = seed_projection_corpus(&home, full_spec());
    assert!(corpus.session_count >= scale_corpus::FULL_SESSION_TARGET);
    assert!(corpus.message_count >= scale_corpus::FULL_MESSAGE_TARGET);
    assert!(corpus.logical_bytes >= scale_corpus::FULL_LOGICAL_BYTES);

    eprintln!(
        "{}",
        measure_operations(&home, &corpus, FULL_WARM_SAMPLES, true)
    );
    assert_sync_progress_and_cancel();
}

/**
 * Assert Sync progress gaps <=500ms and cancel ack <=1s, printing JSON evidence.
 */
fn assert_sync_progress_and_cancel() {
    let progress = run_progress_cadence_case();
    eprintln!("{}", progress.1);
    assert!(
        progress.0 <= PROGRESS_GAP_BUDGET,
        "progress gap {:?} exceeds {:?}",
        progress.0,
        PROGRESS_GAP_BUDGET
    );
    assert!(
        progress.2 >= 3,
        "need multiple checkpoints, saw {}",
        progress.2
    );

    let cancel = run_cancel_ack_case();
    eprintln!("{}", cancel.1);
    assert!(
        cancel.0 <= CANCEL_ACK_BUDGET,
        "cancel ack {:?} exceeds {:?}",
        cancel.0,
        CANCEL_ACK_BUDGET
    );

    let export_progress = run_export_progress_cadence_case();
    eprintln!("{}", export_progress.1);
    assert!(
        export_progress.0 <= PROGRESS_GAP_BUDGET,
        "export progress gap {:?} exceeds {:?}",
        export_progress.0,
        PROGRESS_GAP_BUDGET
    );
    assert!(
        export_progress.2 >= 3,
        "need multiple export Writing checkpoints, saw {}",
        export_progress.2
    );

    let export = run_export_cancel_ack_case();
    eprintln!("{}", export.1);
    assert!(
        export.0 <= CANCEL_ACK_BUDGET,
        "export cancel ack {:?} exceeds {:?}",
        export.0,
        CANCEL_ACK_BUDGET
    );
}

/**
 * Measure cold/warm samples for list, search, detail, and curation via public APIs.
 */
fn measure_operations(
    home: &std::path::Path,
    corpus: &CorpusReport,
    warm_samples: usize,
    enforce_budgets: bool,
) -> String {
    let first_id = corpus.first_external_session_id.clone();
    let timings = vec![
        measure_op(
            home,
            "list_sessions_first_page",
            BUDGET_LIST_MS,
            warm_samples,
            |lib| {
                lib.list_sessions(SessionListRequest {
                    limit: 50,
                    lane: WorkflowLane::All,
                    query: None,
                    cursor: None,
                })
                .expect("list");
            },
        ),
        measure_op(
            home,
            "list_sessions_search_page",
            BUDGET_SEARCH_MS,
            warm_samples,
            |lib| {
                lib.list_sessions(SessionListRequest {
                    limit: 50,
                    lane: WorkflowLane::All,
                    query: Some(SEARCH_PROBE.to_string()),
                    cursor: None,
                })
                .expect("search");
            },
        ),
        measure_op(
            home,
            "session_detail_first_slice",
            BUDGET_DETAIL_MS,
            warm_samples,
            {
                let first_id = first_id.clone();
                move |lib| {
                    lib.session_slice(SCALE_SOURCE_KIND, &first_id, 20, 20)
                        .expect("detail");
                }
            },
        ),
        measure_op(
            home,
            "manual_curation_mutation",
            BUDGET_CURATION_MS,
            warm_samples,
            {
                let first_id = first_id.clone();
                move |lib| {
                    lib.toggle_session_label(SessionCurationRequest {
                        source_kind: SCALE_SOURCE_KIND.to_string(),
                        external_session_id: first_id.clone(),
                        name: "favorite".to_string(),
                    })
                    .expect("curation");
                }
            },
        ),
    ];

    if enforce_budgets {
        for timing in &timings {
            let p50 = percentile_ms(&timing.2, 0.50);
            let p95 = percentile_ms(&timing.2, 0.95);
            if p95 > timing.1 {
                panic!(
                    "scale budget miss: {}",
                    budget_failure_json(
                        timing.0,
                        "warm",
                        p50,
                        p95,
                        timing.1,
                        corpus,
                        timing.2.len()
                    )
                );
            }
        }
    }

    serde_json::json!({
        "kind": if enforce_budgets { "full_budget_run" } else { "smoke_wiring" },
        "claims_full_budgets": enforce_budgets,
        "machine": machine_string(),
        "corpus": {
            "seed": corpus.seed,
            "sessions": corpus.session_count,
            "messages": corpus.message_count,
            "logical_bytes": corpus.logical_bytes,
        },
        "operations": timings.iter().map(|timing| serde_json::json!({
            "operation": timing.0,
            "budget_ms": timing.1,
            "cold": {
                "sample_count": timing.3.len(),
                "p50_ms": percentile_ms(&timing.3, 0.50),
                "p95_ms": percentile_ms(&timing.3, 0.95),
                "samples_ms": timing.3,
            },
            "warm": {
                "sample_count": timing.2.len(),
                "p50_ms": percentile_ms(&timing.2, 0.50),
                "p95_ms": percentile_ms(&timing.2, 0.95),
                "samples_ms": timing.2,
            },
        })).collect::<Vec<_>>(),
    })
    .to_string()
}

/**
 * One cold sample on a fresh handle, then warm samples after one discarded warm-up.
 */
fn measure_op<F>(
    home: &std::path::Path,
    operation: &'static str,
    budget_ms: f64,
    warm_samples: usize,
    mut op: F,
) -> (&'static str, f64, Vec<f64>, Vec<f64>)
where
    F: FnMut(&mut Library),
{
    let cold = {
        let mut library = Library::open(home).expect("cold open");
        let started = Instant::now();
        op(&mut library);
        durations_to_ms(&[started.elapsed()])
    };
    let warm = {
        let mut library = Library::open(home).expect("warm open");
        op(&mut library);
        let mut samples = Vec::with_capacity(warm_samples);
        for _ in 0..warm_samples {
            let started = Instant::now();
            op(&mut library);
            samples.push(started.elapsed());
        }
        durations_to_ms(&samples)
    };
    (operation, budget_ms, warm, cold)
}

/**
 * Observe Sync progress callbacks; returns (max_gap, json, checkpoint_count).
 */
fn run_progress_cadence_case() -> (Duration, String, usize) {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    write_progress_fixture(&fixture, 4);

    let mut library = Library::open(&home).expect("open");
    library
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");

    let mut last_at = None;
    let mut max_gap = Duration::ZERO;
    let mut max_gap_checkpoint = String::from("none");
    let mut checkpoint_count = 0_usize;

    let result = library
        .start_sync(SyncRequest::default(), |progress| {
            checkpoint_count += 1;
            let label = progress_label(&progress);
            let now = Instant::now();
            if let Some(previous) = last_at {
                let gap = now.saturating_duration_since(previous);
                if gap > max_gap {
                    max_gap = gap;
                    max_gap_checkpoint = label;
                }
            }
            last_at = Some(now);
        })
        .expect("sync");
    assert_eq!(result.run.status, "completed");

    let json = serde_json::json!({
        "kind": "progress_cadence",
        "max_gap_ms": max_gap.as_secs_f64() * 1000.0,
        "budget_ms": PROGRESS_GAP_BUDGET.as_secs_f64() * 1000.0,
        "max_gap_checkpoint": max_gap_checkpoint,
        "checkpoint_count": checkpoint_count,
        "machine": machine_string(),
    })
    .to_string();
    (max_gap, json, checkpoint_count)
}

/**
 * Cancel at first CandidateStarted; returns (ack_latency, json).
 * Ack is `SourceFinished{cancelled}` (ingest breaks without CandidateFinished{cancelled}).
 */
fn run_cancel_ack_case() -> (Duration, String) {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    let fixture = temp.path().join("fixture");
    write_progress_fixture(&fixture, 4);

    let mut owner = Library::open(&home).expect("owner");
    owner
        .set_source_preference("fixture", true, Some(fixture.as_path()))
        .expect("pref");

    let home_for_cancel = home.clone();
    let cancel_requested_at = Arc::new(Mutex::new(None::<Instant>));
    let ack_at = Arc::new(Mutex::new(None::<(Instant, String)>));
    let cancel_armed = Arc::new(Mutex::new(false));
    let cancel_requested_at_cb = Arc::clone(&cancel_requested_at);
    let ack_at_cb = Arc::clone(&ack_at);
    let cancel_armed_cb = Arc::clone(&cancel_armed);

    let result = owner
        .start_sync(SyncRequest::default(), |progress| {
            if let SyncProgress::CandidateStarted { sync_run_id, .. } = &progress {
                let mut armed = cancel_armed_cb.lock().expect("armed");
                if !*armed {
                    *armed = true;
                    let mut other = Library::open(&home_for_cancel).expect("cancel library");
                    let requested = Instant::now();
                    other
                        .request_sync_cancel(*sync_run_id)
                        .expect("request cancel");
                    *cancel_requested_at_cb.lock().expect("requested") = Some(requested);
                }
            }
            if let SyncProgress::SourceFinished {
                status,
                source_kind,
                ..
            } = &progress
            {
                if status == "cancelled" {
                    let mut ack = ack_at_cb.lock().expect("ack");
                    if ack.is_none() {
                        *ack = Some((
                            Instant::now(),
                            format!("SourceFinished:{source_kind}:cancelled"),
                        ));
                    }
                }
            }
            if let SyncProgress::CandidateFinished {
                outcome,
                candidate_id,
                ..
            } = &progress
            {
                if outcome == "cancelled" {
                    let mut ack = ack_at_cb.lock().expect("ack");
                    if ack.is_none() {
                        *ack = Some((Instant::now(), format!("CandidateFinished:{candidate_id}")));
                    }
                }
            }
        })
        .expect("sync");
    let terminal_at = Instant::now();
    assert_eq!(result.run.status, "cancelled");

    let requested = cancel_requested_at
        .lock()
        .expect("requested")
        .expect("cancel was requested");
    let (acked, ack_checkpoint) = ack_at
        .lock()
        .expect("ack")
        .clone()
        .unwrap_or_else(|| (terminal_at, "durable_terminal_cancelled".to_string()));
    let ack_latency = acked.saturating_duration_since(requested);
    let json = serde_json::json!({
        "kind": "cancel_ack",
        "ack_latency_ms": ack_latency.as_secs_f64() * 1000.0,
        "budget_ms": CANCEL_ACK_BUDGET.as_secs_f64() * 1000.0,
        "ack_checkpoint": ack_checkpoint,
        "terminal_status": result.run.status,
        "machine": machine_string(),
    })
    .to_string();
    (ack_latency, json)
}

/**
 * Observe export Writing cadence across >=4 train sessions; returns (max_gap, json, writing_count).
 */
fn run_export_progress_cadence_case() -> (Duration, String, usize) {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    bootstrap_home(&home);
    let corpus = seed_projection_corpus(&home, smoke_spec());
    assign_dataset_labels(&home, 4, "train");

    let mut library = Library::open(&home).expect("open export cadence library");
    let mut last_at = None;
    let mut max_gap = Duration::ZERO;
    let mut max_gap_checkpoint = String::from("none");
    let mut writing_checkpoint_count = 0_usize;
    let mut checkpoint_count = 0_usize;

    let result = library
        .publish_export(ExportDataset::Train, |progress| {
            checkpoint_count += 1;
            let label = export_progress_label(&progress);
            if matches!(progress, ExportProgress::Writing { .. }) {
                writing_checkpoint_count += 1;
            }
            let now = Instant::now();
            if let Some(previous) = last_at {
                let gap = now.saturating_duration_since(previous);
                if gap > max_gap {
                    max_gap = gap;
                    max_gap_checkpoint = label;
                }
            }
            last_at = Some(now);
            ExportProgressControl::Continue
        })
        .expect("export cadence result");
    assert_eq!(result.status.as_str(), "published");
    assert!(
        result.record_count >= 4,
        "need >=4 labeled train sessions, wrote {}",
        result.record_count
    );

    let json = serde_json::json!({
        "kind": "export_progress_cadence",
        "max_gap_ms": max_gap.as_secs_f64() * 1000.0,
        "budget_ms": PROGRESS_GAP_BUDGET.as_secs_f64() * 1000.0,
        "max_gap_checkpoint": max_gap_checkpoint,
        "writing_checkpoint_count": writing_checkpoint_count,
        "checkpoint_count": checkpoint_count,
        "eligible_records": result.record_count,
        "corpus_sessions": corpus.session_count,
        "terminal_status": result.status.as_str(),
        "machine": machine_string(),
    })
    .to_string();
    (max_gap, json, writing_checkpoint_count)
}

/**
 * Cancel export at a later Writing checkpoint after >=2 records written; measure ack.
 */
fn run_export_cancel_ack_case() -> (Duration, String) {
    let temp = TempDir::new().expect("temp");
    let home = temp.path().join("home");
    bootstrap_home(&home);
    let corpus = seed_projection_corpus(&home, smoke_spec());
    assign_dataset_labels(&home, 4, "train");

    let mut library = Library::open(&home).expect("open export library");
    let requested_at = Arc::new(Mutex::new(None::<Instant>));
    let cancel_checkpoint = Arc::new(Mutex::new(None::<String>));
    let requested_at_cb = Arc::clone(&requested_at);
    let cancel_checkpoint_cb = Arc::clone(&cancel_checkpoint);
    let started = Instant::now();
    let result = library
        .publish_export(ExportDataset::Train, move |progress| {
            if let ExportProgress::Writing {
                records_written, ..
            } = &progress
            {
                if *records_written >= 2 {
                    let mut requested = requested_at_cb.lock().expect("export requested");
                    if requested.is_none() {
                        *requested = Some(Instant::now());
                        *cancel_checkpoint_cb.lock().expect("export checkpoint") =
                            Some(export_progress_label(&progress));
                        return ExportProgressControl::Cancel;
                    }
                }
            }
            ExportProgressControl::Continue
        })
        .expect("cancelled export result");
    let terminal_at = Instant::now();
    let requested = requested_at
        .lock()
        .expect("export requested")
        .expect("export cancel requested after >=2 records written");
    let ack_checkpoint = cancel_checkpoint
        .lock()
        .expect("export checkpoint")
        .clone()
        .expect("export cancel checkpoint");
    let ack_latency = terminal_at.saturating_duration_since(requested);
    assert_eq!(result.status.as_str(), "cancelled");
    let json = serde_json::json!({
        "kind": "export_cancel_ack",
        "ack_latency_ms": ack_latency.as_secs_f64() * 1000.0,
        "budget_ms": CANCEL_ACK_BUDGET.as_secs_f64() * 1000.0,
        "ack_checkpoint": ack_checkpoint,
        "terminal_status": result.status.as_str(),
        "eligible_records": corpus.session_count.min(4),
        "elapsed_ms": started.elapsed().as_secs_f64() * 1000.0,
        "machine": machine_string(),
    })
    .to_string();
    (ack_latency, json)
}

/**
 * Stable checkpoint label for Sync progress-gap evidence.
 */
fn progress_label(progress: &SyncProgress) -> String {
    match progress {
        SyncProgress::RunQueued { .. } => "RunQueued".into(),
        SyncProgress::RunStarted { .. } => "RunStarted".into(),
        SyncProgress::SourceStarted { source_kind, .. } => format!("SourceStarted:{source_kind}"),
        SyncProgress::SourceFinished {
            source_kind,
            status,
            ..
        } => format!("SourceFinished:{source_kind}:{status}"),
        SyncProgress::CandidateStarted { candidate_id, .. } => {
            format!("CandidateStarted:{candidate_id}")
        }
        SyncProgress::CandidateFinished {
            candidate_id,
            outcome,
            ..
        } => format!("CandidateFinished:{candidate_id}:{outcome}"),
    }
}

/**
 * Stable checkpoint label for export progress-gap and cancel-ack evidence.
 */
fn export_progress_label(progress: &ExportProgress) -> String {
    match progress {
        ExportProgress::Preparing { .. } => "Preparing".into(),
        ExportProgress::Writing {
            records_written, ..
        } => format!("Writing:{records_written}"),
        ExportProgress::Committed { .. } => "Committed".into(),
        ExportProgress::Renamed { .. } => "Renamed".into(),
        ExportProgress::Published { .. } => "Published".into(),
    }
}
