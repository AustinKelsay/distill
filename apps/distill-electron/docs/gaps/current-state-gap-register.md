# Distill Electron Current-State Gap Register

This document is normative for acknowledged drift between the canonical specs and the current implementation.

All gaps currently listed here are historical. No open spec-alignment gaps are currently tracked in this register.

## GAP-001: Raw Capture Recoverability

- Status: resolved in the current implementation.
- Historical rule: every successfully snapshotted capture must store recoverable raw content owned by Distill Electron.
- Resolution notes:
- canonical captures persist a `CaptureContentRef` in Distill Electron-owned storage
- inline and blob-backed raw payloads are recoverable from Distill Electron without rereading the source
- file-backed captures and virtual OpenCode exports can be replayed from Distill Electron-owned data alone
- Implemented in: `src/distill-electron/raw_capture.ts`, `src/distill-electron/db.ts`, `src/distill-electron/import.ts`, `src/test/import.test.ts`

## GAP-002: Snapshot Failure Modeling

- Status: resolved in the current implementation.
- Historical rule: snapshot failures are audit and operational events, not canonical captures.
- Resolution notes:
- snapshot failures emit `capture_failed` activity events without inserting canonical capture rows
- failed snapshot attempts still appear in import reports and sync summaries
- existing projection state remains unchanged when snapshotting fails
- Implemented in: `src/distill-electron/import.ts`, `src/test/import.test.ts`

## GAP-003: Projection Semantics Are Implicit

- Status: resolved in the current implementation.
- Historical rule: `sessions`, `messages`, and `artifacts` are the latest successful materialized projection and replace atomically on success.
- Resolution notes:
- projection replacement is represented as a first-class `replaceSessionProjection` write path
- normalization commits through an explicit transaction boundary instead of scattered helper sequencing
- rollback-on-failure and replace-on-success semantics remain enforced by import tests
- Implemented in: `src/distill-electron/db.ts`, `src/distill-electron/import.ts`, `src/test/import.test.ts`

## GAP-004: Activity Audit Coverage Is Incomplete

- Status: resolved in the current implementation.
- Historical rule: `activity_events` must cover capture, failure, projection, manual curation, export, and sync lifecycle.
- Resolution notes:
- projection success emits `projection_replaced`
- capture failures emit `capture_failed`
- tag add/remove emits `tag_added` and `tag_removed`
- label enable/disable emits `label_toggled`
- sync jobs emit `sync_queued`, `sync_started`, `sync_completed`, and `sync_failed`
- Implemented in: `src/distill-electron/import.ts`, `src/distill-electron/export.ts`, `src/distill-electron/curation.ts`, `src/distill-electron/jobs.ts`

## GAP-005: Jobs And Logs Overlap With Audit Semantics

- Status: resolved in the current implementation.
- Historical rule: jobs and logs are operational views; `activity_events` are the canonical audit trail.
- Resolution notes:
- sync lifecycle domain events are written to `activity_events`
- jobs remain the operational execution record
- logs remain an operational surface derived from jobs and exports
- warning-only sync outcomes are stored and surfaced as first-class `warning` job/log state without being treated as fatal errors
- legacy `completed` job rows with failure details are read back as warning state for backward compatibility
- Implemented in: `src/distill-electron/jobs.ts`, `src/distill-electron/logs.ts`, `src/renderer/app.ts`, `src/shared/types.ts`

## GAP-006: Manual Curation Is Not Audited

- Status: resolved in the current implementation.
- Historical rule: manual tags and labels are canonical curation actions and must be auditable.
- Resolution notes:
- every manual tag add/remove is audited
- every manual label enable/disable is audited
- session detail and export behavior remain unchanged apart from added auditability
- invalid session ids now remain true no-ops and do not create partial curation side effects
- Implemented in: `src/distill-electron/curation.ts`, `src/test/activity_audit.test.ts`, `src/test/export.test.ts`, `src/test/query.test.ts`

## GAP-007: Artifact Linkage Is Partial

- Status: resolved in the current implementation.
- Historical rule: artifacts should use `message_id` when tied to a user-visible message and `capture_record_id` whenever provenance exists.
- Resolution notes:
- imported artifacts now populate `message_id` when a projected message association exists
- `capture_record_id` remains populated for provenance when capture records exist
- session detail queries use direct message linkage, and legacy rows are backfilled on database open
- Implemented in: `src/distill-electron/db.ts`, `src/distill-electron/query.ts`, `src/test/import.test.ts`, `src/test/query.test.ts`

## GAP-008: Root Docs Still Drifted From The Intended Spec Shape

- Status: resolved in the current implementation.
- Historical rule: root docs are concise summaries and entrypoints; authoritative architecture lives under `docs/`.
- Resolution notes:
- root docs now point readers to the canonical docs package under `docs/`
- informative files stop claiming stale implementation gaps that have already been closed
- discovery remains explicitly non-normative and machine-specific
- Implemented in: `README.md`, `PLAN.md`, `IMPLEMENTATION.md`, `DISCOVERY.md`, `src/test/docs.test.ts`

## GAP-009: Projection Fidelity Was Missing From Session Detail And Export

- Status: resolved in the current implementation.
- Historical rule: session detail and labeled export must preserve session-level projection metadata and per-message transcript semantics from the current materialized projection.
- Resolution notes:
- session detail now exposes `external_session_id`, `started_at`, `source_url`, `summary`, `raw_capture_count`, and parsed session metadata
- malformed legacy `metadata_json` values in session rows now read back as `{}` instead of breaking the read model
- labeled export now includes `source_url`, `summary`, parsed session metadata, and per-message `message_kind` plus parsed message metadata
- export payload ordering now lists labels before tags to match the canonical curation precedence guidance
- Implemented in: `docs/specs/search-curation-export.md`, `docs/testing/contract-test-matrix.md`, `src/shared/types.ts`, `src/distill-electron/query.ts`, `src/distill-electron/export.ts`, `src/renderer/app.ts`, `src/test/query.test.ts`, `src/test/export.test.ts`, `src/test/docs.test.ts`
