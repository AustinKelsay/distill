# Issue Session — #44 Tauri/React Multi-Source Thin-Caller Product Loop

## Issue

- Issue: [#44](https://github.com/AustinKelsay/distill/issues/44)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: `68e1133`
- Worker session: Grok 4.5 xhigh bounded implementation pass; Codex integration
- Commit: pending
- Status: Implementation complete locally; final-head CI and review closeout pending
- Review packet: `docs/runs/reviews/44-tauri-multisource-thin-caller.md` (pending)

## Intended Contract

The Tauri host and React renderer are thin callers of the public
`distill-library` seams. Source preferences carry only Source kind, root, and
enabled state; provider detection, parsing, redaction, projection, curation,
export, activity, and operations remain Library-owned. Sync accepts the
selected source set and drives the same public path for Fixture, Codex, Claude
Code, OpenCode, and Droid.

Host and bridge contracts must keep diagnostics safe: mixed-source failures
surface as warnings without source-content leakage, and projections remain
available after at least one file-backed root is removed. Existing Fixture
restart/operations behavior remains unchanged.

## Testing Seam

- Primary seam: Tauri host commands/bridge types calling public `Library` APIs.
- Hermetic roots: synthetic Codex, Claude Code, OpenCode, and Droid fixtures;
  OpenCode uses the existing fake subprocess seam where required.
- Forbidden shortcuts: provider parsing or policy in React/Tauri, direct
  repository/SQL access, real user provider directories, or packaged-machine
  dependencies.
- Vertical slice: source preferences → Sync → search/detail → curation →
  export → Activity/Operations, with isolation/redaction assertions.

## Verification Plan

- Focused Rust host integration tests for all four providers plus Fixture
  regression and root-removal projection survival.
- Focused bridge/React typecheck and Vitest coverage for source selection and
  warning rendering.
- Rust workspace, fault, format, clippy, dependency-tree, and diff checks;
  Linux package smoke on the final pushed head.
- Two-axis Grok standards/spec review against issue #44 and Matt Pocock v1.1
  quality rules; local CodeRabbit attempt with Grok fallback if it stalls.

## Evidence Symbols

- `host_codex_provider_journey_and_projection_survival`
- `host_claude_code_provider_journey`
- `host_opencode_provider_journey_and_projection_survival`
- `host_droid_provider_journey`
- `host_provider_failure_isolation_redacts_diagnostics`
- `App.multisource.test.tsx::persists selected codex and opencode roots and renders safe warning details`
- `App.multisource.test.tsx::hydrates an existing enabled provider root before persisting untouched drafts`
- Matrix/evidence IDs: `THC-003`, `THC-004`, `TRC-004`

Local host/renderer evidence is green. Keep these rows pending until the final
head has the relevant CI/package checks and the independent two-axis review is
recorded.

## Non-goals / residuals

Renormalize UI/Attempt history, human VoiceOver/Narrator speech, Developer ID
signing/notarization/stapling, Windows packaging, production deployment,
Electron retirement, and GTK advisory cleanup remain outside this slice.
