# Issue Session — #43 Multi-Source Parser Registry and Same-Capture Renormalization

## Issue

- Issue: [#43](https://github.com/AustinKelsay/distill/issues/43)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: `e800706`
- Worker session: Grok 4.5 xhigh bounded implementation pass; Codex integration
- Commit: pending
- Status: Implementation complete; awaiting verification + review
- Review packet: `docs/runs/reviews/43-multisource-parser-registry-renormalization.md`

## Intended Contract

The Rust Library owns one parser identity per closed v1 Source kind: Fixture,
Codex, Claude Code, OpenCode, and Droid. Source detection and Sync use those
registered identities; callers do not supply parser ids or reach into storage.
`renormalize_capture(capture_id)` uses only Distill-owned Capture bytes and the
persisted Capture identity, dispatching through the same provider parser seam
without rereading a source root or rerunning an OpenCode subprocess.

A newer parser Attempt is append-only and receives a new Attempt id. Successful
replay replaces the complete Session Projection atomically; parse or projection
failure preserves the prior successful Projection and records a safe failed
Attempt. Unknown or unregistered persisted Source kinds return a typed error
without mutating Attempts or Projection state.

## Testing Seam

- Primary seam: public `Library` methods and provider adapter contracts.
- Forbidden shortcuts: caller-supplied parser ids, SQL/repository access from
  tests (except the planted unknown-kind setup in
  `unknown_persisted_source_kind_rejects_without_mutation`), source-root
  rereads, provider subprocesses during replay.
- Vertical slice: registry model, provider replay dispatch, same-Capture
  immutability/failure tests, then canonical evidence updates.

## Verification Plan

- Targeted `distill-library` parser-registry/retry integration tests for all five
  v1 Source kinds and source-removal replay.
- Rust workspace, fault, format, clippy, dependency-tree, and diff checks.
- Two-axis Grok standards/spec review against issue #43 and Matt Pocock v1.1
  quality rules.
- Local CodeRabbit attempt with fresh Grok fallback if the service stalls or is
  unavailable.

## Evidence Symbols

- `library_parser_registry.rs::registered_parser_versions_are_typed_and_source_specific`
- `library_parser_registry.rs::fixture_renormalize_after_source_removal_preserves_prior_attempts`
- `library_parser_registry.rs::codex_renormalize_after_source_removal`
- `library_parser_registry.rs::claude_renormalize_after_source_removal`
- `library_parser_registry.rs::opencode_renormalize_after_source_removal_without_subprocess`
- `library_parser_registry.rs::droid_renormalize_after_source_removal`
- `library_parser_registry.rs::fixture_renormalize_parse_failure_preserves_last_good_projection`
- `library_parser_registry.rs::renormalize_projection_failure_preserves_last_good_projection`
- `library_parser_registry.rs::successful_empty_projection_fully_clears_messages_and_artifacts`
- `library_parser_registry.rs::unknown_persisted_source_kind_rejects_without_mutation`
- Existing Fixture gates in `library_attempt_retry.rs` remain green
