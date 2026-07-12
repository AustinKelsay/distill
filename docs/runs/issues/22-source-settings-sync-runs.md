# Issue Session — #22 Source Settings and Sync Runs

## Issue

- Issue: [#22](https://github.com/AustinKelsay/distill/issues/22)
- Fixed point before session: `3e420df`
- Worker session: Grok 4.5 xhigh AFK implementation + independent audit remediation
- Commit: `ab2cc83`
- Status: Complete

## Inputs

- Spec issue: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Ticket: Deliver async source settings and Sync Runs
- Relevant glossary: Source, Capture Candidate, Sync Run, Activity Event
- Relevant ADR: `0001-rust-library-with-tauri-shell`, `0003-sqlite-and-content-addressed-files-are-library-internals`
- Health handoff: #21 `operations_status=not_applicable` replaced by real `ok`/`active`/`failed` Sync Run health (`LHR-007` updated; `OSR-*` added)

## Intended Contracts

- Persist enabled/disabled and canonical configured-root preferences per Source without exposing adapter or storage internals. Fixture proves the generic seam; provider adapters remain #26–#29.
- Detection returns one independent typed outcome per requested Source, including executable when applicable, effective data root, and health. Diagnostics use stable generic messages/classes with no path or provider-payload fragments. One failed Source never blocks another healthy Source.
- A Sync Run is durable operational bookkeeping distinct from Activity. Queue/start/terminal transitions and canonical sync Activity commit together where required. Unknown/empty/disabled selections fail before any Sync Run or Activity side effects.
- Safe cancellation checkpoints are before each Source and before each Capture Candidate. Cancellation never interrupts snapshot/Capture acceptance/Attempt/projection transactions; a request during a candidate takes effect before the next candidate. Lease ownership is re-asserted after CandidateStarted progress before candidate work.
- Starting while queued/running fails atomically with `sync_already_running` and creates no Sync Run or Activity side effects.
- Partial candidate/Source success terminates as `warning` with `sync_completed`; cancellation terminates as `cancelled`; fatal no-progress execution terminates as `failed`; stale active leases are accurately classified and idempotently failed on reopen without an injectable public clock.
- Background lease heartbeat keeps live long candidates active past the stale window; terminal runs are never kept alive by heartbeat.
- CLI and Tauri run work off the renderer thread, emit typed Source/Candidate progress, and expose cancellation. Renderer remains bridge-only.
- Configured roots reject empty/traversal escapes after canonicalization. Provider subprocess policy is bounded and redacted at the Library-internal seam (stdin written after readers start; full join/cleanup; Unix process-group kill preferred), ready for #26–#29 without implementing those adapters early.

## Implementation

- Public interface used: `Library::list_sources`, `set_source_preference`, `detect_sources`, `start_sync`, `request_sync_cancel`, `sync_status`, plus thin CLI `sources`/`sync` and Tauri sync/source commands
- Removed from public API: `open_with_clock`, `set_clock_for_test`, exported `LibraryClock` (production uses system UTC only)
- Test-only seam: Cargo feature `test-leases` for short lease/heartbeat intervals (absent from production default API)
- `tdd` used: Library Sync Run concurrency/cancellation/restart/warning/heartbeat contracts first (`library_ops_sync.rs`), then thin CLI/host/renderer callers
- Migration: additive `0002_source_prefs_sync_runs.sql`; `0001` untouched
- Commands run:
  - `cargo fmt --all`
  - `cargo clippy --workspace --all-targets -- -D warnings`
  - `cargo test --workspace`
  - `cargo test -p distill-library --features test-faults`
  - `cargo test -p distill-library --test library_ops_sync --features test-leases`
  - `cargo test -p distill-library --features 'test-faults test-leases'`
  - `cargo test -p distill-cli --test cli_fixture_journey`
  - `cargo test -p distill-desktop --test host_fixture_journey`
  - `npm run desktop:typecheck && npm run desktop:lint && npm run desktop:format && npm run desktop:test && npm run desktop:frontend:build`
  - `cargo build --workspace`
  - desktop/CLI caller gates as previously documented
- Evidence highlights: Library `library_ops_sync` 17 pass (including malformed-lease reopen, warning details, OSR-009–014 audit contracts); host 7 and CLI 7 pass; renderer 11 pass; health LHR-007 remains `operations_status=ok` on clean homes.

## Review

- Review fixed point: `ab2cc83` plus the final focused rereview evidence
- Standards findings: Grok xhigh initially FAIL (7 concrete findings); all high/medium findings were applied, and the final focused rereview PASS found no concrete issues.
- Spec findings: Grok xhigh PASS for Library acceptance; evidence gaps for reopen, warning visibility, cancelled payload, and thin host/CLI seams were closed with tests/surfaces.
- Focused rereview: Grok xhigh PASS after malformed-lease reopen and exact-cancel-id fixes. No concrete findings remained; the reviewer noted only optional evidence gaps (a fixed-id spy and no host-only warning-detail assertion).
- Worthy fixes applied:
  - removed public clock injection authority
  - SyncRequest validation before insert
  - terminal guarantees / lease ownership / background heartbeat
  - warning + fatal contracts
  - subprocess stdin/cleanup hardening
  - stable detection diagnostics
  - cancel/status edge typing
  - collision-resistant UUID owner ids, verified process-group setup, direct-child subprocess wording, disabled-by-default ingest Source creation
  - bounded SQLite busy timeout, malformed-lease health/open recovery, warning detail persistence and caller visibility
- Findings ignored with reasons: None. The final pre-commit CodeRabbit attempt was rate-limited for 10 minutes; prior completed CodeRabbit findings were all applied and local/Grok checks are green.

## Risks

- Only Fixture has a concrete adapter in #22. Codex, Claude Code, OpenCode, and Droid detection returns typed `unavailable` (`adapter_not_registered`) without parsers.
- OS signal-driven CLI cancellation is not implemented; durable `sync cancel --id` is the supported path.
- Background scheduling beyond explicit user starts is not required in v1.
- Subprocess duration bound tests are Unix-gated (`/bin/sleep`); output-byte caps and large-stdin contracts are cross-platform / Unix respectively. Grandchild processes outside the child's process group remain a later audit item.
- Concurrent Sync cancel/overlap tests use real second Library connections against one temp home; they can be timing-sensitive under extreme load though they avoid flaky multi-second sleeps.
- Stale-lease tests age durable SQLite lease columns then reopen normally; heartbeat timing proofs use `test-leases` only.
- CLI `sources set` without `--root`/`--clear-root` preserves the existing configured root by re-reading preferences before upsert.
- No commit created per AFK worker instructions.
