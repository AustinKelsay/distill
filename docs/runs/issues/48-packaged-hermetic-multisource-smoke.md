# Issue Session — #48 Packaged Hermetic Multi-Source Smoke

## Issue

- Issue: [#48](https://github.com/AustinKelsay/distill/issues/48)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: post-#47 on `feature/distill-clean-rebuild`
- Implementation commit: pending commit after Codex review
- Worker session: Cursor Grok 4.5 bounded implementation sidecar for Codex
- Status: Implementation complete locally; Codex review passed. The unsigned
  Darwin package built, but its AX journey is blocked in this runner because
  System Events reports no accessible window. Exact-head Ubuntu package runs
  built and installed successfully but failed at the packaged AT-SPI status
  assertion (`Status: warning` not found); the Linux rows remain blocked.
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
- Exact-head Ubuntu `linux-package-smoke` workflow after push; the first three
  exact-head runs are recorded as failed at the packaged status assertion and
  the latest diagnostic run is also non-green
- Matrix/evidence `PKG-004`/`PKG-005`/`LPKG-004`/`LPKG-005`; Fixture-only
  `PKG-001..003` / `LPKG-001..003` retain their historical evidence while the
  combined harness remains pending package runs

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
- Exact-head Ubuntu package build/install: passed. Installed-host UI journey:
  failed at `AT-SPI accessible not found: Status: warning` on runs
  `29229103417`, `29229486473`, and `29229940050`; the later diagnostic run
  `29230286887` also failed. No Linux hermetic coverage is claimed.

## Non-goals / residuals

- Packaged Attempt-history / Capture-renormalize journey (bounded non-goal;
  covered by host/renderer contracts).
- Host-installed/real-machine provider roots, VoiceOver/Narrator speech,
  Developer ID signing/notarization/stapling, Windows packaging, Electron
  retirement, and #17/#38 closure.
