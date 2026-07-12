# Issue Session — #21 Health, Repair, and Ingest Fault Recovery

## Issue

- Issue: [#21](https://github.com/AustinKelsay/distill/issues/21)
- Fixed point before session: `b5713cc`
- Worker session: Grok 4.5 xhigh AFK implementation + independent-audit remediation
- Commits: `1799b5b`, `f1d3244`
- Status: Complete — both review axes pass

## Inputs

- Spec issue: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Ticket: Add Library health, repair, and ingest fault recovery
- Relevant glossary terms: Capture, Normalization Attempt, Session Projection, Activity Event, Distill home
- Relevant ADRs: `0002-captures-attempts-and-projections-are-distinct`, `0003-sqlite-and-content-addressed-files-are-library-internals`
- Relevant gap: `GAP-R004` (resolved for ingest health/fault/repair; Sync stale handoff #22; export #25)

## Intended Contracts

- Health distinguishes migration/schema integrity (checksums + SQLite quick/integrity/foreign-key), referenced content presence/checksum without symlink follow or home escape, projection/FTS agreement across every searchable field, incomplete durable state (including counter drift; empty successful projections valid), temporary staging files, unreferenced CAS blobs, and explicit `operations_status=not_applicable` until #22.
- Test-only fault injection covers blob stage write, blob rename, Capture acceptance, `capture_recorded`, Fact/projection rows, FTS replacement, Attempt success, and `projection_replaced` boundaries; reopen observes the documented durable state. Production default builds contain no message-prefix fault special case.
- Open performs only safe non-destructive reconciliation of canonical `{64 hex}.partial` files and reports it. Explicit repair removes or reconstructs only documented repairable state; interrupted Captures before Attempt resolve via `capture_failed` Activity (never invented Attempts); destructive actions require caller opt-in and repeated repair is idempotent/transactional.
- Library owns policy. CLI and desktop host/renderer remain thin typed health/repair callers with no SQLite or filesystem authority in the renderer.

## Implementation

- Public interface used:
  - `Library::open` / `open_reconciliation()` — safe canonical staging-partial cleanup
  - `Library::health()` — typed `HealthReport` + `HealthIssue` including `operations_status`
  - `Library::repair(RepairOptions)` — idempotent named actions (`appended_capture_failed_recoveries`, `recomputed_session_counters`, …)
  - CLI: `distill health --home`, `distill repair --home --confirm` (+ journey preserved)
  - Tauri: `health_command`, `repair_command`; bridge + React confirmation surface
  - `#[cfg(feature = "test-faults")] distill_library::faults` + `LibraryError::InjectedTestFault`
- `tdd` used: yes — regressions first for audit items, then implementation; fault suite behind `test-faults`
- Schema: no new migration required; derived from `0001` + filesystem. `0001` untouched.
- Legacy `src/**`: untouched

## Commands run / evidence

### Audit remediation verification (2026-07-11)

| Gate | Command | Exit | Summary |
| --- | --- | --- | --- |
| Library health suite | `cargo test -p distill-library --test library_health_repair` | 0 | 13 passed |
| Fault suite | `cargo test -p distill-library --features test-faults --test library_fault_injection` | 0 | 8 passed |
| Library + faults | `cargo test -p distill-library --features test-faults` | 0 | attempt/fixture/health/fault green |
| fmt | `cargo fmt --all -- --check` | 0 | PASS |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings` | 0 | PASS |
| workspace tests | `cargo test --workspace` | 0 | PASS — CLI/host/library suites green |
| workspace build | `cargo build --workspace` | 0 | PASS |
| desktop checks | `npm run desktop:typecheck` / `desktop:lint` / `desktop:format` / `desktop:test` / `desktop:frontend:build` | 0 | vitest 2 files / 7 tests; production renderer built |
| Tauri release host | `npm run desktop:build` (`tauri build --no-bundle`) | 0 | optimized host built |
| legacy Electron | `PATH=/opt/homebrew/Cellar/node/26.0.0/bin:$PATH npm test` | 0 | 103 passed; `src/**` untouched |
| local CodeRabbit | `coderabbit review --agent --light --type uncommitted` | rate-limited | attempted before commit; service requested a 17-minute wait |

## Review

- Review fixed point: `b5713cc...1799b5b`; focused re-review `1799b5b...f1d3244`
- Standards findings: Symlinked staging-root traversal was a blocker; unrecognized staging severity was a judgement call
- Spec findings: Pass; no findings
- Worthy fixes applied: audit items 1–10 plus home-layout symlink rejection, staging-root non-traversal, blocking unsafe/unrecognized staging classification, and an external-target regression
- Findings ignored with reasons: Counter-query duplication and the large cohesive contract file were non-blocking judgement calls; deferred until a second consumer or navigability cost justifies extraction
- Final review result: Standards pass; Spec pass; focused re-review pass on both axes

## Risks

- Sync Run stale-operation semantics remain `operations_status=not_applicable` until #22; #21 documents the typed handoff (`LHR-007`) and does not invent jobs.
- Export crash recovery remains #25.
- Fault injection is feature-gated via typed `InjectedTestFault`; production default builds contain no fault arming or message-prefix special case.
- Mid-SQLite-transaction fault points assert rollback rather than durable partial Facts/FTS rows, matching real transaction boundaries.
