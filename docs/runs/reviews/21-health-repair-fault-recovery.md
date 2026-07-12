# Review Packet — #21 Health, Repair, and Ingest Fault Recovery

## Issue

- Issue: [#21](https://github.com/AustinKelsay/distill/issues/21)
- Slice type: AFK tracer bullet
- Acceptance criteria: complete typed health classification; real-boundary fault/reopen evidence; reported safe open reconciliation; explicit idempotent destructive repair; Library/CLI/desktop surfaces
- Baseline: `b5713cc`
- Current diff: `git diff b5713cc...1799b5b`

## Implementation Summary

The Library now classifies migration/SQLite integrity, referenced content, exact projection/FTS searchable fields, staging, CAS orphans/unsafe entries, incomplete Attempts/projection linkage, and materialized counter drift. Open removes and reports only canonical disposable staging partials. Explicit repair transactionally resolves pending/interrupted durable state, rebuilds FTS, and deletes only canonical unreferenced regular CAS files. Eight feature-gated fault contracts interrupt real ingest boundaries and prove reopen behavior. CLI, Tauri, and React expose typed health and confirmation-gated repair.

## Implementation Evidence

- `implement` sessions: Grok 4.5 xhigh AFK worker plus independent-audit remediation, integrated by Codex
- `tdd` used: Yes — 12 health/repair contracts and 8 feature-gated fault/reopen contracts over real temporary homes, SQLite, CAS, and Fixture ingest
- Independent audit fixes: valid empty projections; ancestor symlink/path escape; strict staging names; all FTS fields; canonical `capture_failed` recovery rather than fake Attempts; Session counter drift; SQLite integrity/FK checks; typed cfg-gated faults; repair transactions; explicit operations handoff
- Thin callers: CLI 6, Tauri host 5, renderer 7 tests
- Gates: fmt, Clippy warnings denied, workspace tests/build, feature fault suite, renderer typecheck/lint/format/test/build, optimized Tauri `--no-bundle`, legacy Node 26 tests 103/103

## Review Instructions

Review only this issue's slice unless a severe regression crosses the boundary. Keep Standards and Spec findings separate.

Check:

- Health is accurate for valid empty projections and detects schema/FK, referenced content, FTS field, unsafe CAS/staging, incomplete linkage, pending Attempt, and Session-counter drift.
- Health/repair never follows symlinks, reads outside the chosen home, exposes raw paths/payload/SQL, deletes referenced content, or mutates/deletes Captures or Capture Facts.
- Safe open reconciliation deletes only canonical 64-lowercase-hex `.partial` regular files and reports the count.
- Explicit repair is idempotent and transactional where related SQLite state changes together; an interrupted Capture before parser execution creates `capture_failed` Activity, not a fictional Attempt.
- Fault hooks and the typed injected error are absent from default builds and cannot be armed through production caller APIs.
- Fault tests match actual SQLite/file transaction boundaries and preserve last-good projection/FTS.
- CLI and desktop remain thin; repair requires explicit confirmation; renderer has typed bridge authority only.
- `operations_status=not_applicable` is an explicit handoff to #22 rather than a claim that Sync stale detection already exists.
- `src/**` legacy behavior is unchanged.

Local CodeRabbit was attempted before commit and rate-limited for 17 minutes.

## Reviewer Output

Initial review:

```text
STANDARDS_STATUS: changes_requested
STANDARDS_FINDINGS:
1. Blocker: a symlinked staging root was followed by open/health/repair and could delete a canonical partial outside the Distill home.

SPEC_STATUS: pass
SPEC_FINDINGS: None
```

Resolution:

- Home layout now rejects symlink/special directory and database entries before chmod/open.
- Open reconciliation and repair never traverse an unsafe staging root.
- Health reports `unsafe_staging_root` and unrecognized staging entries as blocking, because automatic repair intentionally cannot remove them.
- Added a contract proving health, repair, and reopen never touch a canonical-looking external partial behind a staging-root symlink.

Focused re-review: Pending.
