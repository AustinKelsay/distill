# Issue Session — #48 Packaged Hermetic Multi-Source Smoke

## Issue

- Issue: [#48](https://github.com/AustinKelsay/distill/issues/48)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: post-#47 on `feature/distill-clean-rebuild`
- Implementation commits: `dbfe7b0`, `8d84209`, `53c3372`, `cec7876`, `b84bde4`, `56b613b`, `0abed5a`, `3b07134`, `0e0ca51`, `7dca071`
- Worker session: Cursor Grok 4.5 bounded implementation sidecar for Codex
- Status: Implementation complete locally; Codex review passed. The unsigned
  Darwin package built, but its AX journey is blocked in this runner because
  System Events reports no accessible window. Exact-head Ubuntu package runs
  built and installed successfully but failed at the packaged AT-SPI status
  assertion on earlier exact-head runs. The final exact-head run
  `29233489395` is green through Detect, redaction, Sync, per-Source outcomes,
  and the retained journey after the accessibility remediations; Linux rows
  are complete while Darwin AX remains manual-required.
- Review packet: `docs/runs/reviews/48-packaged-hermetic-multisource-smoke.md`

## Intended Contract

Extend the shipped Tauri package smoke beyond the Fixture-only journey so the
installed/package runtime exercises the already-landed hermetic multi-Source
caller path without claiming host-installed provider, release-signing, or
assistive-technology evidence.

Smoke harnesses seed temporary file-backed Codex, Claude Code, OpenCode, and
Droid roots. OpenCode uses `{root}/bin/opencode` stub only. Packaged UI drives
existing Source preference/Detect/Sync controls, then retains Fixture
search/detail/curation/export/restart/containment. Detect sibling-failure
isolation/redaction is asserted. Attempt-history/renormalize is an explicit
bounded non-goal for this packaged harness.

## Testing Seam

- Primary: packaged macOS AX and Linux AT-SPI/`xdotool` smoke scripts.
- Hermetic fixture helper: `packaged-hermetic-multisource.mjs` with
  `node --test` coverage.
- Forbidden shortcuts: Rust/product edits, bridge/UI product logic, Electron
  `src/**`, signing, Windows packaging, host-installed providers, inventing
  Attempt-history/renormalize controls.

## Verification Plan

- `npm --prefix apps/distill-desktop run test:hermetic-fixtures`
- Node `--check` on changed smoke/helper scripts; Python compile on
  `linux-atspi-find.py`
- Local Darwin `desktop:package:macos` + `desktop:smoke:macos` (unsigned);
  package metadata passed, while the AX journey remains a manual gate when
  this runner cannot expose the packaged window
- Exact-head Ubuntu `linux-package-smoke` workflow after push; run
  `29233489395` is green through the complete installed-host hermetic journey
  after the status, redaction-sentinel, Sync-status, and per-Source outcome
  accessibility remediations
- Matrix/evidence `PKG-004`/`PKG-005`/`LPKG-004`/`LPKG-005`; Fixture-only
  `PKG-001..003` / `LPKG-001..003` retain their historical evidence while the
  Linux combined harness is green and macOS remains a manual AX gate

## Evidence Symbols

- `seedHermeticMultisourceRoots` /
  `packaged-hermetic-multisource.node-test.mjs`
- `macos-package-smoke.mjs` (`hermetic_multisource`, `detect_sibling_isolation`)
- `linux-package-smoke.mjs` (`hermetic_multisource`, `detect_sibling_isolation`)
- Matrix/evidence IDs: `PKG-004`, `PKG-005`, `LPKG-004`, `LPKG-005`

## Local Verification

- Hermetic fixture `node --test`: passed.
- Node syntax checks on changed smoke/helper scripts: passed.
- Darwin package build/static checks: passed. The automated AX journey returned
  `Distill window did not appear` even though the packaged app window was
  visible to Core Graphics; rerunning with `DISTILL_MACOS_ALLOW_MANUAL=1`
  correctly records `ui: manual_required` and leaves the hermetic rows pending.
- Exact-head Ubuntu CI package build/install and installed-host UI journey:
  passed on run `29233489395`. It observed Detect warning/sibling isolation,
  redaction, Sync success, every `fixture/codex/claude_code/opencode/droid:
  completed` outcome, and the retained search/detail/curation/export/restart
  journey. Earlier status-discovery and stderr-protocol failures remain
  historical evidence in the review packet.

## Non-goals / residuals

- Packaged Attempt-history / Capture-renormalize journey (bounded non-goal;
  covered by host/renderer contracts).
- Host-installed/real-machine provider roots, VoiceOver/Narrator speech,
  Developer ID signing/notarization/stapling, Windows packaging, Electron
  retirement, and #17/#38 closure.
