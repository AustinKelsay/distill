# Distill Current-State Gap Register

This document is normative for acknowledged drift between the canonical specs and the current implementation.

## Electron Baseline

All Electron baseline gaps currently listed here are historical. No open spec-alignment gaps are currently tracked in this register for the Electron baseline.

## Rebuild Gaps

### GAP-R001: Dual Runtime During Rebuild

- Status: open
- Rule: the rebuild Library is the target product interface; Electron remains until cutover.
- Current drift: Electron under `src/**` still serves the shipping baseline while the native Library, thin CLI, and first-run Tauri/React Fixture callers exist beside it. Packaging and the full product loop remain incomplete.
- Impacted files/modules: legacy `src/**`; native `crates/distill-library`; `crates/distill-cli`; `apps/distill-desktop`.
- Severity: high — the rebuild cannot replace the shipped runtime until provider Sources, Sync Runs, Curation, export, migration, and packaging paths are complete.
- Target branch/ticket: `feature/distill-clean-rebuild`, final cutover gate #37.
- Acceptance criteria: native desktop and CLI pass the contract matrix and packaged routine source-to-export smoke; Electron remains read-only migration evidence rather than a routine dependency.

### GAP-R002: Remaining Provider SourceAdapters Not Yet In Library

- Status: resolved
- Rule: Codex, Claude Code, OpenCode, Droid, and Fixture share one SourceAdapter seam.
- Current drift: none for the five v1 Sources. Fixture, Codex, Claude Code, OpenCode, and Droid are implemented in the Rust Library tracer and use the shared Library preservation path.
- Impacted files/modules: `crates/distill-library/src/adapter`; provider fixture corpus and Source contract tests.
- Severity: resolved for the provider-adapter slice.
- Target branch/tickets: `feature/distill-clean-rebuild`, #18 and #26–#29.
- Resolution: Droid #29 adds the file-backed adapter, default/override roots, exact replay, mixed-block parsing, sidecar metadata, typed diagnostics, and the `library_droid_source` contract suite.
- Acceptance criteria: each launch Source passes its appendix and shared conformance corpus through the same internal adapter and Library preservation path.

### GAP-R003: Final Cutover Deferred

- Status: open (Fixture/Codex/Claude/OpenCode/Droid Sync Runs, Curation, export, and Activity/Operations diagnostics delivered in #22/#24/#25/#26/#27/#28/#29/#30; final cutover remains)
- Rule: full product loop includes Sync Runs, Curation, and Export Artifacts.
- Current drift: durable Sync Runs, Source preferences, independent detection, CLI/Tauri/React Sync surfaces, Sync lease health with system-UTC stale repair and background heartbeat, warning/partial-success terminals, typed selection/lease-lost edges, transactional manual Curation, the previewed crash-recoverable Export Artifact path, and separate cursor-paged Activity/Operations diagnostics are implemented for all five v1 Sources. Final packaging/cutover remains #37.
- Impacted files/modules: Library ops, curation, export, provider adapters, and packaging/cutover surfaces.
- Severity: medium — the Fixture loop is proven, but real provider coverage and final cutover are incomplete.
- Target branch/tickets: `feature/distill-clean-rebuild`, Sync #22, Curation #24, export #25, provider #26–#29, diagnostics #30, final cutover #37.
- Acceptance criteria: async Sync Runs, transactional manual Curation, previewed crash-recoverable JSONL export, and Activity/Operations diagnostics pass their public Library, CLI, host, and renderer contracts; provider adapters and packaging then pass the final cutover gate.

### GAP-R004: Fault Injection And Crash-Point Repair Deferred

- Status: resolved
- Rule: interrupted Capture acceptance, projection publication, and related transitions reopen into a defined repair state.
- Current drift: none for ingest, Sync Run, or Export fault/health/recovery contracts. Sync Run stale-lease reopen is covered by #22 (`operations_status` is `ok`/`active`/`failed`), and export reopen checksum reconciliation is covered by #25.
- Impacted files/modules: `crates/distill-library` health/repair/fault/Sync seams; thin CLI/Tauri/React callers.
- Severity: resolved for #21 ingest recovery, #22 Sync lease health, and #25 export recovery.
- Target branch/ticket: `feature/distill-clean-rebuild`, #21 / #22.
- Acceptance criteria: fault-injection contracts interrupt staging/rename/acceptance/projection/FTS/activity transitions and reopen into the documented repair state; Sync stale leases fail idempotently on reopen.

### GAP-R005: Legacy Electron Home Migration

- Status: resolved for the migration seam; final runtime cutover remains open in GAP-R001/GAP-R003.
- Rule: a legacy Electron home is read-only evidence and must not be opened or mutated as a destination database.
- Resolution: issue #31 adds a WAL-safe private SQLite snapshot, path-alias/traversal rejection, representative Capture/Attempt/Fact/Projection/Curation/Activity/export mapping, redacted reports, fingerprint markers, and import-owned CAS/export rollback cleanup.
- Impacted files/modules: `crates/distill-library/src/migrate`; Library/CLI/Tauri/React callers; `docs/specs/legacy-migration.md`.
- Severity: resolved for the import contract; final Electron retirement remains a cutover concern.
- Acceptance criteria: WAL and rollback-journal homes remain byte-for-byte unchanged; repeated imports reuse markers; unsafe/missing content is skipped with stable redacted reasons; mapped sessions are searchable, curated, activity-visible, and export-metadata complete.

### GAP-R006: Hostile Inputs And Desktop Capabilities

- Status: resolved for the v1 Library, CLI, Tauri host, and renderer bridge boundary; final packaged cutover remains open in GAP-R001/GAP-R003.
- Rule: every SourceAdapter and thin caller must bound hostile input, preserve typed failure semantics, redact caller/Activity/Operations diagnostics, and deny ambient renderer authority.
- Resolution: issue #32 adds the shared privacy policy, pre-snapshot Capture-size gate, bounded JSON document/line/depth parsing, symlink-safe discovery, secret/path/SQL/payload redaction, safe CLI/Tauri messages, typed Tauri path/enumeration validation, events-only capabilities, and hostile-input/bridge contracts. The v1 privacy boundary is explicit: `sensitive` is export-only; no application encryption, per-session delete, retention purge, or secure-forget.
- Impacted files/modules: `crates/distill-library/src/privacy.rs`; SourceAdapters/ingest/query/migration; `crates/distill-cli`; Tauri host/error/capabilities; React bridge; `docs/specs/privacy-and-capabilities.md`.
- Severity: resolved for the hostile-input/capability slice; packaged runtime verification remains a cutover concern.
- Target branch/ticket: `feature/distill-clean-rebuild`, #32.
- Acceptance criteria: the shared hostile corpus and provider-bound suites pass; no false Captures or raw diagnostic payload leaks occur; Tauri capabilities remain events-only and bridge calls remain typed invoke/listen.

### GAP-R007: Packaged Accessibility Runtime Evidence

- Status: open until packaging tickets #35/#36; renderer contract resolved in #33.
- Rule: keyboard, focus, semantic status, visual-state, and reduced-motion behavior must be proven at the thin React seam, while packaged WebView and assistive-technology claims require runtime evidence.
- Current implementation: `App.a11y.test.tsx`, `App.states.test.tsx`, and `styles.a11y.test.ts` cover keyboard activation, focus return, semantic names/live regions, dialog Tab fallback, contrast tokens, reduced motion, 200% text-size DOM presence, and deterministic major-state markers. `npm run a11y:smoke` builds the renderer and runs these suites.
- Remaining drift: the smoke does not launch a signed macOS/Linux Tauri bundle, and CI does not automate VoiceOver or Narrator. Human validation is documented in `apps/distill-desktop/docs/a11y-human-checklist.md`.
- Impacted files/modules: `apps/distill-desktop/src/App.tsx`, `apps/distill-desktop/src/a11y/confirm-dialog.tsx`, `apps/distill-desktop/src/styles.css`, packaging smoke harnesses.
- Severity: medium — the renderer seam is covered, but packaged focus behavior and screen-reader output remain release gates.
- Target branch/tickets: `feature/distill-clean-rebuild`, #33, #35, and #36.
- Acceptance criteria: macOS and Linux packaged smoke proves launch, keyboard traversal, dialog focus containment, cancellation focus return, and one search/detail/curation/export path; human checklist evidence records supported screen readers without converting manual observations into automated claims.

### GAP-R008: Scale And Latency Budget Evidence

- Status: resolved for #34 on the recorded host; keep the full benchmark as a scheduled/manual regression gate for other hardware.
- Rule: the rebuild must remain usable at 25,000 Sessions, 1,000,000 current-projection messages, and 10 GiB logical Distill-home content without exposing private histories or claiming UI performance from Library-only measurements.
- Current drift: existing query, curation, Sync, and export contracts are correct at small fixtures, but no deterministic large-corpus generator, p95 report, 500 ms progress-gap evidence, or 1 s safe-checkpoint cancellation evidence existed before #34.
- Resolution: `library_scale_budgets` seeds a fixed synthetic SQLite/FTS corpus and benchmark-owned sparse padding in a temporary home, measures public Library APIs with cold/warm separation, and reports actionable JSON. Migration `0006_sessions_list_page_index.sql` keeps current-session paging bounded at scale; full logical-size runs remain ignored/env-gated rather than a default PR cost.
- Impacted files/modules: `crates/distill-library/tests/library_scale_budgets.rs`, Library query/curation/ops/export seams, `docs/specs/scale-and-latency.md`, and scheduled benchmark commands.
- Severity: high for cutover confidence; the default smoke is bounded and cannot stand in for the full 25k/1M/10 GiB scheduled evidence.
- Target branch/ticket: `feature/distill-clean-rebuild`, #34.
- Acceptance evidence: deterministic targets, warm p95 budgets (150 ms page/search/detail; 100 ms curation), progress gaps ≤500 ms, cancellation acknowledgement ≤1 s at safe checkpoints, and reproducible hardware/cold/warm/actionable reports all pass without private or committed corpus data. Full run evidence is recorded in `docs/runs/issues/34-scale-latency.md`.

## Historical Electron Gaps

## GAP-001: Raw Capture Recoverability

- Status: resolved in the current implementation.
- Historical rule: every successfully snapshotted capture must store recoverable raw content owned by Distill.
- Resolution notes:
- canonical captures persist a `CaptureContentRef` in Distill-owned storage
- inline and blob-backed raw payloads are recoverable from Distill without rereading the source
- file-backed captures and virtual OpenCode exports can be replayed from Distill-owned data alone
- Implemented in: `src/distill/raw_capture.ts`, `src/distill/db.ts`, `src/distill/import.ts`, `src/test/import.test.ts`

## GAP-002: Snapshot Failure Modeling

- Status: resolved in the current implementation.
- Historical rule: snapshot failures are audit and operational events, not canonical captures.
- Resolution notes:
- snapshot failures emit `capture_failed` activity events without inserting canonical capture rows
- failed snapshot attempts still appear in import reports and sync summaries
- existing projection state remains unchanged when snapshotting fails
- Implemented in: `src/distill/import.ts`, `src/test/import.test.ts`

## GAP-003: Projection Semantics Are Implicit

- Status: resolved in the current implementation.
- Historical rule: `sessions`, `messages`, and `artifacts` are the latest successful materialized projection and replace atomically on success.
- Resolution notes:
- projection replacement is represented as a first-class `replaceSessionProjection` write path
- normalization commits through an explicit transaction boundary instead of scattered helper sequencing
- rollback-on-failure and replace-on-success semantics remain enforced by import tests
- Implemented in: `src/distill/db.ts`, `src/distill/import.ts`, `src/test/import.test.ts`

## GAP-004: Activity Audit Coverage Is Incomplete

- Status: resolved in the current implementation.
- Historical rule: `activity_events` must cover capture, failure, projection, manual curation, export, and sync lifecycle.
- Resolution notes:
- projection success emits `projection_replaced`
- capture failures emit `capture_failed`
- tag add/remove emits `tag_added` and `tag_removed`
- label enable/disable emits `label_toggled`
- sync jobs emit `sync_queued`, `sync_started`, `sync_completed`, and `sync_failed`
- Implemented in: `src/distill/import.ts`, `src/distill/export.ts`, `src/distill/curation.ts`, `src/distill/jobs.ts`

## GAP-005: Jobs And Logs Overlap With Audit Semantics

- Status: resolved in the current implementation.
- Historical rule: jobs and logs are operational views; `activity_events` are the canonical audit trail.
- Resolution notes:
- sync lifecycle domain events are written to `activity_events`
- jobs remain the operational execution record
- logs remain an operational surface derived from jobs and exports
- warning-only sync outcomes are stored and surfaced as first-class `warning` job/log state without being treated as fatal errors
- legacy `completed` job rows with failure details are read back as warning state for backward compatibility
- Implemented in: `src/distill/jobs.ts`, `src/distill/logs.ts`, `src/renderer/app.ts`, `src/shared/types.ts`

## GAP-006: Manual Curation Is Not Audited

- Status: resolved in the current implementation.
- Historical rule: manual tags and labels are canonical curation actions and must be auditable.
- Resolution notes:
- every manual tag add/remove is audited
- every manual label enable/disable is audited
- session detail and export behavior remain unchanged apart from added auditability
- invalid session ids now remain true no-ops and do not create partial curation side effects
- Implemented in: `src/distill/curation.ts`, `src/test/activity_audit.test.ts`, `src/test/export.test.ts`, `src/test/query.test.ts`

## GAP-007: Artifact Linkage Is Partial

- Status: resolved in the current implementation.
- Historical rule: artifacts should use `message_id` when tied to a user-visible message and `capture_record_id` whenever provenance exists.
- Resolution notes:
- imported artifacts now populate `message_id` when a projected message association exists
- `capture_record_id` remains populated for provenance when capture records exist
- session detail queries use direct message linkage, and legacy rows are backfilled on database open
- Implemented in: `src/distill/db.ts`, `src/distill/query.ts`, `src/test/import.test.ts`, `src/test/query.test.ts`

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
- Implemented in: `docs/specs/search-curation-export.md`, `docs/testing/contract-test-matrix.md`, `src/shared/types.ts`, `src/distill/query.ts`, `src/distill/export.ts`, `src/renderer/app.ts`, `src/test/query.test.ts`, `src/test/export.test.ts`, `src/test/docs.test.ts`

## GAP-010: Rebuild Query Surfaces Were Limited To First Slices

- Status: resolved for the current rebuild slice.
- Historical rule: large libraries need deterministic current-projection search/list cursors, workflow-lane intersection, and bounded session detail pages without exposing storage authority to callers.
- Resolution notes:
- Rust Library now exposes typed list/search pages with Unicode-safe quoted-AND FTS normalization, keyset cursors, manual-origin labels/tags, and shared workflow derivation.
- Rust Library detail pages expose named projection metadata, explicit raw-capture counts, curation read models, and message/artifact continuation cursors.
- CLI, Tauri host, bridge, and React explorer use typed page/detail surfaces and preserve selected sessions across refresh.
- Implemented in: `crates/distill-library/src/query/mod.rs`, `crates/distill-library/tests/library_query_paging.rs`, `apps/distill-desktop/src/App.tsx`, `crates/distill-cli/src/lib.rs`
