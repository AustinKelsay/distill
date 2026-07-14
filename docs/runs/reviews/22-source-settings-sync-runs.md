# Review Packet — #22 Source Settings and Sync Runs

## Issue

- Issue: [#22](https://github.com/AustinKelsay/distill/issues/22)
- Slice type: AFK tracer bullet
- Acceptance criteria: source preferences and independent detection; durable asynchronous Sync Runs; safe cancellation and overlap rejection; stale leases and heartbeat; warning/partial success; bounded subprocess policy; thin CLI/Tauri/React callers
- Baseline: `3e420df`
- Implementation: `ab2cc83`

## Implementation Evidence

- `implement` session: Grok 4.5 xhigh worker, independently audited and integrated by Codex
- `tdd` used: Yes — 17 Library contracts (including feature-gated lease/process seams), CLI 7, Tauri host 7, and renderer 11
- Rust gates: fmt check, Clippy warnings denied, workspace tests/build, `test-faults`/`test-leases` suites
- Desktop gates: typecheck, lint, Prettier, Vitest, frontend build, optimized Tauri `--no-bundle`
- Legacy compatibility: Node 26 build and 103 tests pass
- CodeRabbit: prior completed passes’ findings applied; final pre-commit attempt rate-limited for 10 minutes

## Review Instructions

Review only this issue's slice unless a severe cross-slice regression is demonstrated. Keep standards and spec findings separate.

Check:

- Source preferences survive reopen and configured roots are canonicalized and bounded.
- Detection returns independent typed outcomes and redacted diagnostics.
- Sync validation, queue/start/terminal transitions, Activity, overlap rejection, cancellation checkpoints, stale-lease repair, owner reassertion, and heartbeat are durable and race-safe.
- Warning and failed outcomes preserve source/candidate details, metrics, and caller visibility.
- Provider subprocess bounds, process-group cleanup, stdin ordering, and reader joins do not deadlock or leak.
- CLI, Tauri, and React remain thin typed callers; test-only authority is feature-gated and absent from production defaults.

## Reviewer Output

Initial standards review:

```text
FAIL — findings included public test authority, renderer cancel race, missing overlap-row assertion, SQLite contention, preference-default inconsistency, dead warning schema, subprocess cleanup/error handling, and maintainability issues.
```

Resolution: all concrete high/medium findings were applied, with focused tests and caller surfaces added for warning details, exact cancellation, malformed lease reopen, overlap rows, heartbeat, bounded process cleanup, and preference persistence.

Independent spec review:

```text
PASS — Library acceptance met; evidence gaps for reopen, warning visibility, cancelled Activity payload, and thin caller seams were closed.
```

Final focused rereview:

```text
PASS
Findings: None.
Evidence gaps: only optional test-strength notes (fixed-id cancel spy and no host-only warning-detail assertion).
```
