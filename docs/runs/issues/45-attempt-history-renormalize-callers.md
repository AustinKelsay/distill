# Issue Session — #45 Attempt History and Capture Renormalization Callers

## Issue

- Issue: [#45](https://github.com/AustinKelsay/distill/issues/45)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: `d93eb66`
- Worker session: Grok 4.5 High bounded implementation; Codex integration and remediation
- Commit: `a58449c`
- Status: Complete — local implementation, review, and exact implementation-head CI green; staging handoff remains
- Review packet: `docs/runs/reviews/45-attempt-history-renormalize-callers.md`

## Intended Contract

Expose the existing Library-owned `capture_attempts` and
`renormalize_capture` seams through the thin CLI and Tauri/React callers. The
callers discover Capture ids from Activity, never own SQL, parser ids,
provider parsing, Source-root rereads, or OpenCode subprocesses. Successful
retry appends an immutable Attempt and publishes the next Projection without a
new Capture; failed retry preserves the last-good Projection. File-backed
provider roots may be removed before replay, while unknown persisted kinds
return typed safe errors without mutation or path dumps.

The React surface remains bridge-only and explicitly renders Attempt history
and renormalization idle/loading/ready/empty/warning/error states. In-flight
requests are invalidated when the selected Session or explorer is reset.

## Testing Seam

- CLI: Fixture Sync → Activity Capture discovery → Attempt summaries →
  same-process parser-version advance → renormalize → detail/search/Activity.
- Tauri host: mixed Fixture/Codex Sync, Codex root removal, same-Capture
  renormalize, and unknown-kind no-mutation isolation.
- React/bridge: bridge-only success, failure, warning, pagination, and
  cross-Session request-race behavior.
- Forbidden shortcuts: SQL/storage access, parser-id inputs, provider policy,
  Source-root reread, ambient renderer authority, or real user provider roots.

## Verification Plan

- Focused CLI, host, bridge, and renderer tests.
- Rust workspace, fault-injection, format, clippy, dependency-tree, and diff
  gates; desktop typecheck/lint/format/frontend build.
- Independent Grok standards/spec review against Matt Pocock skills v1.1 and
  the Plebdev Feature Dev loop; local CodeRabbit attempt with Grok fallback.
- Exact implementation-head Linux package/smoke and Rust advisory CI before
  closing the issue and marking evidence rows passed.

## Evidence Symbols

- `cli_fixture_capture_attempts_and_renormalize_json_journey`
- `cli_codex_renormalize_after_source_removal_json_journey`
- `host_capture_attempts_and_renormalize_after_source_removal`
- `host_unknown_source_kind_renormalize_isolates_without_mutation`
- `App.test.tsx::loads Attempt history and renormalize states through the bridge only (TRC-005)`
- `App.test.tsx::ignores Attempt results when the selected Session changes mid-flight`
- `App.states.test.tsx::renders Attempt history and failed renormalize warning states`
- Matrix/evidence IDs: `TCC-006`, `TCC-007`, `THC-005`, `THC-006`, `TRC-005`

## Final-head CI

- Linux package/install/smoke: [run 29222714955](https://github.com/AustinKelsay/distill/actions/runs/29222714955) — passed on implementation closeout head `afcd4a7`.
- Rust advisory scan: [run 29222714957](https://github.com/AustinKelsay/distill/actions/runs/29222714957) — passed on implementation closeout head `afcd4a7`.

## Local Verification

- Focused CLI journey: 19 passed.
- Focused Tauri host journey: 7 passed.
- Renderer Vitest: 45 passed across 6 files.
- `npm run desktop:typecheck`, `desktop:lint`, `desktop:format`, and
  `desktop:frontend:build`: passed.
- `cargo test --workspace`: passed; the existing full-scale benchmark remains
  ignored by default.
- `cargo test -p distill-library --features test-faults`: passed.
- `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo tree --workspace --locked`, and `git diff --check`: passed.

## Non-goals / residuals

Packaged real-provider roots, VoiceOver/Narrator speech, Developer ID
signing/notarization/stapling, Windows packaging, Electron retirement, and
user-facing parser-version preference remain release or human residuals.
