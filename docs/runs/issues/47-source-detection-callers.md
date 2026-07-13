# Issue Session — #47 Source Detection Callers

## Issue

- Issue: [#47](https://github.com/AustinKelsay/distill/issues/47)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: post-#46 on `feature/distill-clean-rebuild`
- Implementation commit: `c6b9a3c`
- Worker session: Grok 4.5 High bounded implementation sidecar for Codex
- Status: Complete — implementation, focused verification, and exact-head CI green; staging handoff remains the broader PR residual
- Review packet: `docs/runs/reviews/47-source-detection-callers.md`

## Intended Contract

Expose the existing Library-owned `detect_sources` seam through the thin CLI and
Tauri/React callers. Callers remain provider-neutral bridges over
`SourceDetectRequest` / `SourceDetectResult`. Detection is read-only: it never
mutates Sync Runs, Captures, Projections, or Activity. One unhealthy Source never
aborts siblings. Caller messages stay redacted (no path/token dumps in
`error_message`).

CLI `sources detect --request KIND[=ROOT]` returns stable human/JSON output.
Tauri validates home/requests, runs off the UI thread, and registers
`detect_sources_command`. The React bridge adds typed `detectSources`, and the
renderer exposes the smallest bridge-only detection control/status surface.

## Testing Seam

- CLI: mixed Fixture ok/unhealthy + disabled Droid + missing Codex; usage exits;
  Activity/Sync no-mutation.
- Tauri host: validated detect batch with sibling isolation, redaction, and empty
  Activity after detect.
- Bridge: exact `detect_sources_command` invoke shape.
- Forbidden shortcuts: adapter edits, Sync internals, ambient renderer authority,
  packaging/signing/Windows/Electron/merge policy.

## Verification Plan

- Focused CLI detect suite, host unit test in `host.rs`, and bridge Vitest.
- Desktop typecheck/lint/format and focused bridge test.
- Canonical architecture/matrix/evidence/gap and issue/review/ledger packets.
- Exact-head rebuild CI: [29226422580](https://github.com/AustinKelsay/distill/actions/runs/29226422580).
- Exact-head Linux package smoke: [29226422562](https://github.com/AustinKelsay/distill/actions/runs/29226422562).
- Exact-head Rust advisory scan: [29226422558](https://github.com/AustinKelsay/distill/actions/runs/29226422558).

## Evidence Symbols

- `cli_sources_detect_json_isolates_and_redacts`
- `cli_sources_detect_usage_and_no_mutation`
- `host_detect_sources_isolates_siblings_without_mutation`
- `bridge.test.ts` detectSources invoke coverage
- `App.test.tsx` detection ready/loading/empty/warning/error state coverage
- Matrix/evidence IDs: `TCC-008`, `TCC-009`, `THC-007`, `TRC-006`

## Local Verification

- Focused CLI detect suite: 2 passed.
- Focused host detect unit test: 1 passed.
- Bridge Vitest: 5 passed.
- App detection Vitest: `App.test.tsx` 28 passed.
- `npm run desktop:typecheck`, `desktop:lint`, and `desktop:format`: passed.
- Independent final Grok standards/spec rereview: PASS on both axes; no required remediations.
- CodeRabbit CLI: initial attempt rate-limited; bounded retry stalled in summarization and was terminated; final Grok review used as fallback.

## Non-goals / residuals

Packaged real-provider roots, VoiceOver/Narrator speech,
Developer ID signing/notarization/stapling, Windows packaging, Electron
retirement, and #38 staging merge policy remain release or human residuals.
