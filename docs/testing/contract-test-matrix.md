# Distill Contract Test Matrix

This document is normative for planned and required contract tests.

Where a `Primary Branch` or `Target Branch` is listed below, it records the first branch that claimed the contract, even when the scenario is now implemented and passing in the current tree.

## Suite Index

| Suite | Purpose | Primary Branch |
| --- | --- | --- |
| `connector_contract` | Validate connector inputs and outputs remain within the canonical boundary. | `test/connector-contract-hardening` |
| `raw_capture_persistence` | Validate Distill-owned recoverable raw capture storage. | `test/raw-capture-contracts` |
| `projection_replacement` | Validate replace-on-success and rollback-on-failure projection semantics. | `test/raw-capture-contracts` |
| `activity_audit` | Validate canonical audit event coverage. | `impl/activity-and-curation-audit` |
| `search_indexing` | Validate FTS and query behavior against the current projection. | `impl/query-and-search-alignment` |
| `session_read_model` | Validate session detail exposes projection metadata and provenance safely. | `impl/projection-fidelity-export` |
| `manual_curation` | Validate manual tags and labels as the normative curation layer. | `docs/test-matrix` |
| `export_contract` | Validate export payloads against the current projection and manual curation state. | `docs/test-matrix` |
| `sync_jobs_and_logs` | Validate operational sync reporting without treating logs as the canonical audit trail. | `docs/test-matrix` |
| `doc_truthfulness` | Validate the canonical docs package stays present and wired together. | `docs/spec-foundation` |
| `library_fixture_tracer` | Validate the Rust Library Fixture ingest/projection/query/replay/health seam. | `feature/distill-clean-rebuild` |
| `library_attempt_retry` | Validate duplicate inertness, changed Captures, failed Attempts, renormalize, and full projection replacement. | `feature/distill-clean-rebuild` |
| `library_health_repair` | Validate typed health classification, safe open reconciliation, and explicit idempotent repair. | `feature/distill-clean-rebuild` |
| `library_fault_injection` | Validate ingest crash-boundary reopen states behind `--features test-faults`. | `feature/distill-clean-rebuild` |
| `thin_cli_fixture_caller` | Validate the thin Rust CLI Fixture journey against a real binary. | `feature/distill-clean-rebuild` |
| `thin_tauri_host_fixture_caller` | Validate Tauri host input validation, progress, and typed error/result translation. | `feature/distill-clean-rebuild` |
| `thin_react_fixture_caller` | Validate the React first-run UI against one typed bridge fake. | `feature/distill-clean-rebuild` |

## Fixture Requirements

Shared fixture requirements:

- at least one Codex live session
- at least one Codex archived duplicate of a live session
- at least one Claude Code session with mixed text and structured blocks
- at least one OpenCode export-backed virtual session
- one parse-failure fixture after successful snapshot
- one snapshot-failure fixture
- one large capture fixture that requires blob-backed persistence

Executable fixture sources:

- shared fixture manifest: `src/test/fixtures/ingest/manifest.json`
- fixture install/helper surface: `src/test/support/ingest_fixtures.ts`
- connector contract executable suite: `src/test/connector_contract.test.ts`

Every fixture must document:

- source kind
- source path or virtual path
- expected external session id
- whether it should create transcript messages
- whether it should create artifacts
- whether it should trigger failure behavior

## Scenario Matrix

| Scenario ID | Suite | Scenario | Expected DB State | Expected Query / UI Outcome | Failure Expectations | Target Branch |
| --- | --- | --- | --- | --- | --- | --- |
| `CC-001` | `connector_contract` | Codex connector emits only canonical parsed shapes. | Capture parse output contains one session payload plus raw records/messages/artifacts in canonical shapes. | No source-specific storage logic leaks into shared layers. | Test fails if connector writes non-canonical fields into shared contracts. | `test/connector-contract-hardening` |
| `CC-002` | `connector_contract` | Claude connector preserves text blocks and structured artifacts. | Parsed output includes text messages plus image/tool artifacts. | Session detail can show transcript and artifacts. | Test fails if tool/image blocks become transcript text unexpectedly. | `test/connector-contract-hardening` |
| `CC-003` | `connector_contract` | OpenCode connector preserves visible meta parts and structured artifacts. | Parsed output includes messages/artifacts with canonical roles and kinds. | Session detail can show structured parts without provider leakage. | Test fails if unknown structured parts are dropped. | `test/connector-contract-hardening` |
| `RCP-001` | `raw_capture_persistence` | File-backed capture persists recoverable raw content. | Capture row resolves to a valid `CaptureContentRef` with checksum and byte size. | Replay tooling can recover the original raw content. | Test fails if only hashes/metadata are stored. | `test/raw-capture-contracts` |
| `RCP-002` | `raw_capture_persistence` | Virtual OpenCode capture persists recoverable raw content. | Capture row resolves to Distill-owned content, not only transient process output. | Replay tooling can recover exported session JSON. | Test fails if replay depends on rerunning the source CLI. | `test/raw-capture-contracts` |
| `PR-001` | `projection_replacement` | Exact duplicate re-import is skipped. | No new capture row, or new row is explicitly not inserted per dedupe policy; projection rows unchanged. | Session list/detail/search remain unchanged. | Test fails if duplicate import mutates projection rows. | `test/raw-capture-contracts` |
| `PR-002` | `projection_replacement` | Changed capture appends history and replaces projection. | New capture row exists; session capture count increments; messages/artifacts reflect only newest successful projection. | Search and session detail show only current projection data. | Test fails if stale message rows remain visible. | `test/raw-capture-contracts` |
| `PR-003` | `projection_replacement` | Parse failure after snapshot preserves prior projection. | Capture exists with failure status; new capture records or projection rows are rolled back as required. | Existing session detail remains unchanged. | Test fails if partial rows remain. | `test/raw-capture-contracts` |
| `PR-004` | `projection_replacement` | Imported artifacts link directly to projected messages while retaining capture provenance. | `artifacts.message_id` and `artifacts.capture_record_id` are both populated when a projected message association and capture provenance exist. | Session detail can show artifact/message relationships without indirect reconstruction. | Test fails if artifact/message relationships depend on capture-record joins alone or provenance is dropped. | `impl/projection-cleanup` |
| `PR-005` | `projection_replacement` | Sources without stable external ids synthesize deterministic ids and record synthetic provenance. | Session row exists with a deterministic external session id and session metadata records synthetic provenance. | Session detail and re-import remain stable for captures without a source-provided id. | Test fails if projection materializes without a stable fallback id or provenance marker. | `impl/projection-cleanup` |
| `AA-001` | `activity_audit` | Successful capture and projection emit audit events. | `activity_events` includes `capture_recorded` and `projection_replaced`. | Audit views can attribute session updates to the import run. | Test fails if successful import lacks canonical audit rows. | `impl/activity-and-curation-audit` |
| `AA-002` | `activity_audit` | Snapshot, raw-persistence, or parse failure emits canonical failure audit. | `activity_events` includes `capture_failed`. | Sync detail can show failure without mutating projection. | Test fails if failures only appear in jobs/logs or abort later healthy captures. | `impl/activity-and-curation-audit` |
| `AA-003` | `activity_audit` | Manual tag and label changes emit audit rows. | `activity_events` includes `tag_added`, `tag_removed`, and `label_toggled`. | Curation history is auditable. | Test fails if curation changes are silent. | `impl/activity-and-curation-audit` |
| `AA-004` | `activity_audit` | Sync lifecycle emits canonical audit rows. | `activity_events` includes `sync_queued`, `sync_started`, `sync_completed`, or `sync_failed`. | Audit and ops summaries can be reconciled. | Test fails if sync lifecycle is visible only through jobs. | `impl/activity-and-curation-audit` |
| `SI-001` | `search_indexing` | Search returns current projected transcript rows only. | FTS rows correspond to current message projection. | Search results exclude stale superseded rows. | Test fails if replaced messages remain searchable. | `impl/query-and-search-alignment` |
| `SI-002` | `search_indexing` | Search safely handles punctuation-heavy and zero-token queries. | No DB corruption or invalid FTS query state. | Results still resolve for quoted and dashed input, and all-non-token input returns no results. | Test fails if queries crash, over-match, or treat zero-token input as a broad query. | `impl/query-and-search-alignment` |
| `SRM-001` | `session_read_model` | Session detail exposes stored projection metadata safely. | Session row fields remain readable and malformed legacy `metadata_json` reads back as `{}`. | Session detail includes external session id, timestamps, source URL, summary, raw capture count, and parsed session metadata from the current projection. | Test fails if session detail hides projection metadata or bad JSON breaks the read model. | `impl/projection-fidelity-export` |
| `MC-001` | `manual_curation` | Manual tags appear in session detail and export. | Tag rows and assignments exist with manual origin. | Session detail and export payloads agree. | Test fails if export and detail diverge. | `docs/test-matrix` |
| `MC-002` | `manual_curation` | Manual labels remain session-level only. | Label assignments target sessions and preserve origin. | Dataset-export behavior matches session detail state, and query read models ignore non-manual label origins. | Test fails if label scope drifts silently. | `docs/test-matrix` |
| `MC-003` | `manual_curation` | Enabling a dataset label removes conflicting dataset labels and audits both transitions. | Only one of `train`, `holdout`, or `exclude` remains assigned after the toggle; audit rows capture the disable and enable events. | Session detail and workflow state reflect the winning dataset label immediately. | Test fails if conflicting dataset labels coexist after a manual enable or if the automatic removal is silent. | `impl/curation-policy-export-safety` |
| `MC-004` | `manual_curation` | Review-only labels preserve orthogonal labels while taking workflow priority. | `favorite` and `sensitive` can coexist with one dataset label, and `exclude` removes only conflicting dataset labels. | `Needs Review` shows `exclude`, `sensitive`, and conflicting dataset-label sessions; `Favorites` still includes favorite sessions. | Test fails if orthogonal labels are dropped, conflicting dataset labels surface as export-ready, or workflow priority is wrong. | `impl/curation-policy-export-safety` |
| `EC-001` | `export_contract` | Export uses current session projection, not raw history. | Export bookkeeping row exists; payload matches current projection. | Exported messages equal current session detail transcript. | Test fails if superseded rows appear in output. | `docs/test-matrix` |
| `EC-002` | `export_contract` | Export includes manual curation metadata. | Export row and output include tags and labels. | Consumers can trust export metadata without re-querying Distill. | Test fails if tags/labels are missing or inconsistent. | `docs/test-matrix` |
| `EC-003` | `export_contract` | Export preserves projection metadata and per-message transcript semantics. | Export payload includes session metadata plus message kind and message metadata from the current projection. | Consumers can distinguish text from meta messages, derive turn pairs from real assistant replies only, and recover session provenance without re-querying Distill. | Test fails if export drops session metadata, collapses `message_kind`, pairs turns from assistant meta rows, or omits message metadata. | `impl/projection-fidelity-export` |
| `EC-004` | `export_contract` | Standard dataset export excludes review-only or conflicting dataset-label sessions. | Export rows are written for `train` and `holdout` targets only, and sessions with `exclude`, `sensitive`, or conflicting dataset labels are omitted even when they carry a matching dataset label. | Operators get safe-by-default dataset exports from the main UI. | Test fails if `exclude`, `sensitive`, or conflicting dataset-label sessions appear in standard dataset export. | `impl/curation-policy-export-safety` |
| `EC-005` | `export_contract` | Favorite sessions remain exportable only through their dataset label. | A session labeled `favorite` plus `train` or `holdout` exports with both labels preserved in metadata. | `Favorites` remains an organizational lane, not an export target. | Test fails if favorite-only sessions export or if favorite disappears from exported metadata. | `impl/curation-policy-export-safety` |
| `SL-001` | `sync_jobs_and_logs` | Sync job summaries remain operational, not canonical audit. | Job rows contain sync status and metrics. | Logs show sync state while audit remains the source of truth. | Test fails if log behavior depends on missing audit guarantees. | `docs/test-matrix` |
| `SL-002` | `sync_jobs_and_logs` | Export summaries remain visible in logs. | Export bookkeeping is preserved. | Logs show operational export summaries. | Test fails if export operations disappear from ops surfaces. | `docs/test-matrix` |
| `SL-003` | `sync_jobs_and_logs` | Warning-only syncs remain visible as non-fatal warnings. | Job/log surfaces preserve `status = "warning"` plus sync metrics and warning details without flipping canonical audit. | Operators can distinguish partial success from fatal sync failure. | Test fails if warning-only syncs are treated as errors or disappear from ops surfaces. | `docs/test-matrix` |
| `DT-001` | `doc_truthfulness` | Canonical docs package exists and is linked from root docs. | Required markdown files exist. | Contributors can navigate from root docs to canonical docs. | Test fails if a required canonical doc is removed or unlinked. | `docs/spec-foundation` |
| `DT-002` | `doc_truthfulness` | Root docs remain non-authoritative summaries. | Root docs contain the required links and disclaimers. | Readers are directed to `docs/` for canonical truth. | Test fails if root docs re-assume canonical authority. | `docs/spec-foundation` |
| `LFT-001` | `library_fixture_tracer` | Fresh Distill home bootstraps with checksummed migrations and restrictive Unix modes. | `schema_migrations` rows exist with matching checksums; home dirs are `0o700` and db/files are `0o600`. | Library opens and reports healthy schema status. | Test fails if legacy Electron schema is applied or modes are permissive. | `feature/distill-clean-rebuild` |
| `LFT-002` | `library_fixture_tracer` | Fixture SourceAdapter detect/discover/snapshot/parse through production ingest yields Capture, Attempt, Facts, Projection, FTS, and Activity. | Capture row and `capture_recorded` Activity commit together over recoverable content; successful Attempt has Facts; Session Projection and FTS match. | `session_slice` and bounded `search` return the projected transcript. | Test fails if Fixture bypasses the SourceAdapter seam or projection publishes without an Attempt. | `feature/distill-clean-rebuild` |
| `LFT-003` | `library_fixture_tracer` | Blob-backed replay after source deletion and Library reopen remains checksum-verified. | A Capture above the inline threshold resolves from Distill-owned content-addressed storage after fixture files are removed. | `replay_capture` and `health` succeed after reopen. | Test fails if replay depends on the original Fixture files or only exercises inline storage. | `feature/distill-clean-rebuild` |
| `LFT-004` | `library_fixture_tracer` | Symlink and missing-parent configured-root escapes plus capture-size limits fail with typed errors. | No Capture row is accepted for rejected candidates. | Callers observe `PathOutsideConfiguredRoot` before snapshot or `CaptureTooLarge` before acceptance. | Test fails if escapes or oversized bytes are accepted or misclassified. | `feature/distill-clean-rebuild` |
| `LFT-005` | `library_fixture_tracer` | `run_fixture_journey` returns source, sync, session, and health with typed progress phases. | Journey result carries detected Fixture Source, ingest counters/Session Identities, projected Session, and healthy report. | Thin callers can render the first-run outcome without SQL. | Test fails if progress phases are skipped or storage handles leak into the result. | `feature/distill-clean-rebuild` |
| `AR-001` | `library_attempt_retry` | Exact duplicate Fixture re-ingest is inert. | No new Capture, Attempt, projection Activity, or FTS change. | Session counters and transcript remain unchanged. | Test fails if duplicate ingest mutates Capture/Attempt/projection/Activity/FTS. | `feature/distill-clean-rebuild` |
| `AR-002` | `library_attempt_retry` | Changed bytes append a Capture and replace the projection. | New Capture and Attempt exist; superseded transcript leaves FTS. | Session detail shows only the newest successful projection. | Test fails if stale messages remain visible or searchable. | `feature/distill-clean-rebuild` |
| `AR-003` | `library_attempt_retry` | Parse or projection failure records a typed safe Attempt and preserves last-good projection/FTS. | Failed Attempt has `parse_failed` or `projection_failed`; prior generation unchanged. | Session counters update honestly without projection mutation. | Test fails if last-good rows change or diagnostics are missing/unsafe. | `feature/distill-clean-rebuild` |
| `AR-004` | `library_attempt_retry` | Strictly newer registered Fixture parser renormalizes the same Capture from Distill-owned bytes. | No new Capture; prior successful Attempts/Facts remain immutable; successful retry publishes a projection. | `capture_attempts` exposes history and Fact counts; duplicate re-ingest stays inert. | Test fails if retry invents a Capture, mutates prior Attempts/Facts, permits a non-advancing version, or accepts arbitrary caller parser ids. | `feature/distill-clean-rebuild` |
| `AR-005` | `library_attempt_retry` | Successful shorter and empty projections fully replace prior slices. | Current projection message/artifact sets match the newest successful Attempt only. | Search excludes cleared superseded text; CLI/UI expose separately named Capture, Attempt, and generation counters. | Test fails if old rows leak or counters are collapsed into one ambiguous field. | `feature/distill-clean-rebuild` |
| `LHR-001` | `library_health_repair` | Healthy Fixture home reports ok across schema/content/FTS/staging/orphan/incomplete plus explicit operations handoff. | Typed `HealthReport` statuses are all `ok` with `operations_status=not_applicable` and empty issues. | Callers can trust a green health panel after ordinary ingest. | Test fails if category statuses are missing or falsely failed. | `feature/distill-clean-rebuild` |
| `LHR-002` | `library_health_repair` | Safe open removes only canonical `{64 lowercase hex}.partial` staging files and reports the count. | `OpenReconciliation.removed_staging_partials` matches deleted canonical partials; noncanonical `*.partial` remain and are reported as `unrecognized_staging_entry`. | Reopen leaves no canonical staging partials and does not delete CAS blobs or junk staging names. | Test fails if open deletes orphans/referenced content/noncanonical partials or omits reconciliation counts. | `feature/distill-clean-rebuild` |
| `LHR-003` | `library_health_repair` | Unreferenced CAS blobs are repairable only via explicit repair; repair is idempotent; CAS symlinks/malicious `blob_path` never touch external targets. | Health reports `orphan_blob` / `cas_unrecognized_entry` / `content_invalid_blob_path`; first repair removes only in-root regular canonical orphans; external symlink targets survive. | Open never silently deletes orphans; health/repair never follow symlinks or leave the Distill home. | Test fails if open deletes orphans, repair deletes external targets, or summaries leak raw paths. | `feature/distill-clean-rebuild` |
| `LHR-004` | `library_health_repair` | Projection/FTS disagreement covers every searchable field and is rebuildable from Session title/project_path. | Tampered FTS text/title/project_path fails health; `rebuild_fts` restores agreement from Session rows. | Count-only equality is insufficient. | Test fails if health passes on title/path mismatch or rebuild hardcodes empty project_path. | `feature/distill-clean-rebuild` |
| `LHR-005` | `library_health_repair` | Incomplete Captures and pending Attempts are classified and resolved without inventing Attempts; empty successful projections stay healthy; counter drift is repaired. | Health reports `incomplete_capture` / `pending_attempt` / `session_counter_mismatch`; repair fails pending Attempts, appends Capture-scoped `capture_failed`, recomputes counters, preserves last-good generation; metadata-only empty projections remain `ok`. | Repeated repair is idempotent; exact duplicates stay inert. | Test fails if repair invents `parser_id=repair` Attempts, empty projections are marked incomplete, or counters stay stale. | `feature/distill-clean-rebuild` |
| `LHR-006` | `library_health_repair` | Missing referenced content is blocking and not deleted/fixed by repair. | Health reports `content_missing`; repair leaves content status failed. | Operators see integrity failure without silent data loss. | Test fails if repair claims success or deletes Captures. | `feature/distill-clean-rebuild` |
| `LHR-007` | `library_health_repair` | Operations status is explicit `not_applicable` until #22; no invented Sync stale codes. | `operations_status=not_applicable`; no `stale_sync_operation` / sync-category issues. | Sync handoff is documented typed state, not omitted or fabricated. | Test fails if Sync job health is invented early or the category is silently absent. | `feature/distill-clean-rebuild` |
| `FIR-001` | `library_fault_injection` | Stage write before rename leaves only a staging partial; open reconciles it. | No Capture/CAS; one `.partial`; reopen removes it. | Safe recovery without destructive repair. | Test fails if Capture is accepted or partial survives open. | `feature/distill-clean-rebuild` |
| `FIR-002` | `library_fault_injection` | Rename before Capture acceptance leaves an orphan CAS blob repaired explicitly. | No Capture; one unreferenced blob; repair deletes it. | Health classifies orphan; open does not delete it. | Test fails if Capture exists or open deletes the orphan. | `feature/distill-clean-rebuild` |
| `FIR-003` | `library_fault_injection` | Capture insert before `capture_recorded` rolls back the shared SQLite transaction. | No Capture and no `capture_recorded` Activity. | Impossible mid-tx split is not simulated as durable. | Test fails if half-committed Capture/Activity remains. | `feature/distill-clean-rebuild` |
| `FIR-004` | `library_fault_injection` | After Capture+Activity before Attempt leaves incomplete Capture repaired via `capture_failed`. | Capture + `capture_recorded` durable; no Attempt; repair appends `capture_failed` only. | Health reports `incomplete_capture` until recovery Activity exists. | Test fails if a fake Attempt appears or Capture is lost. | `feature/distill-clean-rebuild` |
| `FIR-005` | `library_fault_injection` | Pending Attempt before publish preserves last-good projection. | New Capture + pending Attempt; prior generation/FTS unchanged. | Repair fails the pending Attempt safely. | Test fails if projection mutates or pending becomes succeeded. | `feature/distill-clean-rebuild` |
| `FIR-006` | `library_fault_injection` | Facts/projection before FTS rolls back the publication transaction. | Pending Attempt remains; last-good projection preserved. | Mid-tx rows are not durable. | Test fails if partial projection/FTS rows remain. | `feature/distill-clean-rebuild` |
| `FIR-007` | `library_fault_injection` | After FTS before Attempt success/Activity rolls back publication. | Pending Attempt remains; last-good projection preserved. | Mid-tx rows are not durable. | Test fails if Attempt succeeds or projection replaces. | `feature/distill-clean-rebuild` |
| `FIR-008` | `library_fault_injection` | After `projection_replaced` before commit rolls back publication. | Pending Attempt remains; last-good projection preserved. | Activity without commit is not durable. | Test fails if `projection_replaced` persists without success. | `feature/distill-clean-rebuild` |
| `TCC-001` | `thin_cli_fixture_caller` | CLI Fixture journey prints stable human and JSON results with exit `0`. | Chosen temp home contains accepted Capture and Session Projection. | Human/JSON stdout includes source, sync, session, and health. | Test fails if the CLI duplicates Library policy or invents unstable output. | `feature/distill-clean-rebuild` |
| `TCC-002` | `thin_cli_fixture_caller` | CLI usage and Library failures return documented exit codes `2` and `1`. | No Capture is accepted for invalid Fixture roots. | JSON stderr carries a stable error class. | Test fails if exit codes or error classes drift. | `feature/distill-clean-rebuild` |
| `TCC-003` | `thin_cli_fixture_caller` | CLI `health` and `repair --confirm` print stable human/JSON with documented exits. | Repair without `--confirm` exits `2`; confirmed repair returns named actions. | Health statuses include staging/orphan/incomplete. | Test fails if repair skips confirmation or invents policy. | `feature/distill-clean-rebuild` |
| `THC-001` | `thin_tauri_host_fixture_caller` | Host runner validates inputs, emits progress, and returns typed journey results. | Temp home matches Library Fixture outcomes. | Progress phases and source/sync/session/health are observed at the host seam. | Test fails if work runs without validation or errors lose their codes. | `feature/distill-clean-rebuild` |
| `THC-002` | `thin_tauri_host_fixture_caller` | Host health/repair validate home and require repair confirmation. | Temp home health matches Library; denied repair is validation. | Typed health/repair results reach the host seam. | Test fails if repair runs without confirm or leaks storage handles. | `feature/distill-clean-rebuild` |
| `TRC-001` | `thin_react_fixture_caller` | React first-run UI renders idle/running/success/error plus source/sync/session/health via one typed bridge fake. | N/A — renderer has no storage authority. | Panels and status reflect bridge results/errors only. | Test fails if the UI reaches ambient filesystem/process/SQLite APIs. | `feature/distill-clean-rebuild` |
| `TRC-002` | `thin_react_fixture_caller` | Production renderer bridge invokes the exact Tauri command shape and cleans up asynchronous event listeners. | N/A — bridge exposes typed values only. | Snake-case Rust parameters receive the required camel-case invoke key; unmount cannot leak a late listener. | Test fails if production IPC differs from the fake or cleanup races registration. | `feature/distill-clean-rebuild` |
| `TRC-003` | `thin_react_fixture_caller` | React health/repair surface requires confirmation and renders typed issues/actions. | N/A — renderer has no storage authority. | Repair stays disabled until confirm; bridge-only data is shown. | Test fails if repair can run without confirm or ambient authority appears. | `feature/distill-clean-rebuild` |

Executable Library Fixture suite: `crates/distill-library/tests/library_fixture_tracer.rs`.
Executable Library Attempt Retry suite: `crates/distill-library/tests/library_attempt_retry.rs`.
Executable Library Health/Repair suite: `crates/distill-library/tests/library_health_repair.rs`.
Executable Library Fault Injection suite: `crates/distill-library/tests/library_fault_injection.rs` (`cargo test -p distill-library --features test-faults`).
Executable CLI suite: `crates/distill-cli/tests/cli_fixture_journey.rs`.
Executable Tauri host suite: `apps/distill-desktop/src-tauri/tests/host_fixture_journey.rs`.
Executable React suites: `apps/distill-desktop/src/App.test.tsx`, `apps/distill-desktop/src/bridge.test.ts`.

## Expected DB State Guidance

For every executable scenario, the test implementation should explicitly assert:

- capture row count and status changes
- capture content ref presence where required
- session row stability or replacement
- message and artifact row membership in the current projection
- activity event coverage
- job/log summaries when relevant

## Expected UI / Query Outcome Guidance

For every executable scenario, the test implementation should explicitly assert:

- session list title and preview behavior where relevant
- session detail transcript correctness
- session detail projection metadata and provenance visibility
- artifact visibility and linkage
- search result freshness after re-imports
- export payload correctness

## Branch Mapping Rule

Scenarios become executable in the first branch that claims their acceptance criteria. A future implementation branch is not complete until its mapped scenarios are executable and passing.
