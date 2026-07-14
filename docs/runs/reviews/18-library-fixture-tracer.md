# Review Packet — #18 Library Fixture Tracer

This review packet is historical evidence. Its old `src/**` paths were removed
before beta; native coverage now lives under `crates/` and
`apps/distill-desktop/`.

## Issue

- Issue: [#18](https://github.com/AustinKelsay/distill/issues/18)
- Slice type: AFK tracer bullet
- Acceptance criteria: fresh checksummed native home; production Fixture adapter path; recoverable Capture before acceptance; Attempt/Facts/Projection/FTS/Activity; replay/reopen/health; typed root/size rejection; Rust gates
- Baseline: `b471a77`
- Current diff: `git diff b471a77...a13bf74`

## Implementation Summary

The Rust `Library` can now open a fresh native Distill home and ingest a synthetic file-backed Source through the internal production adapter seam. The accepted blob-backed Capture remains replayable after source deletion and reopen, while callers can read bounded projection, search, Activity, and health results. Root escapes and oversized captures fail before acceptance.

## Implementation Evidence

- `implement` session: Grok 4.5 xhigh worker, integrated and tightened by Codex
- `tdd` used: Yes, at the public Library seam
- Red test, if applicable: omitted Fixture identity initially failed to resolve the specified deterministic synthetic Session Identity
- Green implementation, if applicable: removed the manifest-ID fallback so the parser records deterministic synthetic identity and provenance
- Refactor, if applicable: narrowed public exports and bounded all first-tracer read slices; Capture plus `capture_recorded` now commit together
- Commands run: `cargo fmt --all -- --check`; Clippy with warnings denied; five Library contracts; Library build; TypeScript build; 93 unaffected legacy tests; local CodeRabbit pre-commit review

## Review Instructions

Review only this issue's slice unless you find a severe cross-slice regression. Keep standards and spec findings separate.

Check:

- Acceptance criteria are met.
- Tests verify behavior through public interfaces.
- No implementation-only tests are masquerading as behavior tests.
- No obvious incomplete work, TODO placeholders, or unrelated changes.
- Relevant test, typecheck, build, or visual verification commands pass.

Known baseline limitation: on Node `v22.22.3`, ten unchanged Electron DB-inspector tests fail because `DatabaseSync.setAuthorizer` is unavailable; no `src/**` or Node package file is changed by this slice.

## Reviewer Output

```text
STANDARDS_STATUS: changes_requested
STANDARDS_FINDINGS:
- Gap entries lacked governed impacted-files, severity, target, and acceptance fields.
- Canonical API list omitted `open_with_limits` and `recent_activity`.
- Generic ingestion hard-coded Fixture parser identity.
- Pre-accept integrity used magic Capture id `-1`.
- Capture size limit was checked twice.

SPEC_STATUS: changes_requested
SPEC_FINDINGS:
- Capture Fact provenance was written but not asserted through the public seam.
- The committed format helper mutated files rather than checking them.
```

Resolution: every finding was applied. The focused re-review compares the full corrected range from `b471a77` to the review-fix commit.

Focused re-review result for `b471a77...b87f5cb`:

```text
STANDARDS_STATUS: pass
SPEC_STATUS: pass
```
