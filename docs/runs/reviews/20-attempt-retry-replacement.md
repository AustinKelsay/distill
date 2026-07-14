# Review Packet — #20 Attempt Retry and Projection Replacement

This review packet is historical evidence. Its old `src/**` paths were removed
before beta; native coverage now lives under `crates/` and
`apps/distill-desktop/`.

## Issue

- Issue: [#20](https://github.com/AustinKelsay/distill/issues/20)
- Slice type: AFK tracer bullet
- Acceptance criteria: inert exact duplicates; immutable changed Captures; typed safe failed Attempts preserving last-good projection/FTS; strictly newer same-Capture retry with immutable Attempt/Facts history; full shorter/empty replacement; separately named caller counters
- Baseline: `4564d28`
- Current diff: `git diff 4564d28...f4b3514`

## Implementation Summary

The Rust Library now records versioned Normalization Attempts separately from immutable Captures, re-normalizes the same Capture from Distill-owned bytes with a strictly newer registered Fixture parser, and atomically publishes full Session Projection replacements. Failed Attempts retain safe typed diagnostics without mutating the last-good projection or FTS. CLI and desktop callers expose separately named Capture, Attempt, and projection-generation counters.

## Implementation Evidence

- `implement` session: Grok 4.5 xhigh worker, independently audited and integrated by Codex
- `tdd` used: Yes — eight public `Library` contracts over real temporary homes, SQLite, inline/CAS replay, Fixture paths, and caller-facing summaries
- Red/green behaviors: duplicate inertness, changed Capture replacement, parse/projection failure rollback, failed-then-successful retry, successful Fact-history immutability, shorter/empty replacement, and parser-registry validation
- Refactor: ingest internals split into Capture acceptance, Attempt/projection publication, and retry modules behind the existing deep `Library` seam
- Commands run: Cargo fmt check, Clippy warnings denied, workspace tests/build, renderer typecheck/test/build, Tauri release `--no-bundle`, and legacy TypeScript build plus 103 Node 26 tests

## Review Instructions

Review only this issue's slice unless you find a severe cross-slice regression. Keep standards and spec findings separate.

Check:

- Acceptance criteria are met through public seams.
- Exact duplicates cause no Capture, Attempt, projection, FTS, or Activity mutation.
- Capture acceptance and its Activity commit together; successful Facts/projection/FTS/Activity commit together.
- Failed Attempts cannot partially publish Facts, projection rows, FTS, or generation changes.
- Retry reads only checksum-verified Distill-owned bytes, creates no Capture, uses a Library-owned parser id, and rejects malformed/equal/older versions.
- Prior successful Attempts and Capture Facts remain observable and immutable.
- Shorter and empty successful projections remove all superseded messages, artifacts, and FTS rows.
- Diagnostics are typed and do not expose raw capture payloads or low-level projection constraints.
- No unrelated legacy `src/**` behavior changed.

Local CodeRabbit was attempted before commit and rate-limited with a two-minute retry window.

## Reviewer Output

Initial review:

```text
STANDARDS_STATUS: changes_requested
STANDARDS_FINDINGS:
1. Hard: architecture method inventory omitted the three #20 public methods.
2. Judgement: ingest/retry attempt-publication control flow is duplicated.
3. Judgement: substring-based failure scrubbing was weaker than its safety claim.

SPEC_STATUS: pass
SPEC_FINDINGS: None
```

Resolution:

- Added all three #20 methods to the architecture inventory.
- Replaced low-level error-string filtering with stable generic parse/projection diagnostics that cannot retain raw Capture, path, or SQLite details; strengthened the public-seam assertions.
- Kept the small ingest/retry orchestration duplication because extracting it would introduce an abstraction before provider adapters establish the reusable shape.

Focused re-review:

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS: None

SPEC_STATUS: pass
SPEC_FINDINGS: None
```

Local CodeRabbit review of the follow-up patch completed with 0 findings across 7 files.
