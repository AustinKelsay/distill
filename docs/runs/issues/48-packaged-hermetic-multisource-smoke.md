# Issue Session — #48 Packaged Hermetic Multi-Source Smoke

## Issue

- Issue: [#48](https://github.com/AustinKelsay/distill/issues/48)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: post-#47 on `feature/distill-clean-rebuild`
- Implementation commits: `dbfe7b0`, `8d84209`, `53c3372`, `cec7876`, `b84bde4`, `56b613b`, `0abed5a`
- Worker session: Cursor Grok 4.5 bounded implementation sidecar for Codex
- Status: Implementation complete locally; Codex review passed. The unsigned
  Darwin package built, but its AX journey is blocked in this runner because
  System Events reports no accessible window. Exact-head Ubuntu package runs
  built and installed successfully but failed at the packaged AT-SPI status
  assertion. The latest exact-head run observed `Status: warning` and all
  sibling statuses, then failed only because the redaction miss sentinel was
  prefixed by a PyGObject deprecation warning; `0abed5a` suppresses that known
  warning and the Linux rows remain pending the next exact-head run.
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
- Exact-head Ubuntu `linux-package-smoke` workflow after push; prior runs
  failed at status discovery, run `29231637753` reached sibling isolation and
  redaction before the stderr-protocol failure, and `0abed5a` is pending
  confirmation
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
- Exact-head Ubuntu package build/install: passed. Run `29231637753` observed
  `Status: warning`, `codex: unhealthy`, `fixture: ok`,
  `claude_code: unavailable`, `opencode: ok`, and `droid: ok`; it then failed
  the redaction probe because a PyGObject deprecation warning preceded the
  expected AT-SPI miss sentinel. The earlier status-discovery failures remain
  recorded on runs `29229103417`, `29229486473`, `29229940050`, and diagnostic
  `29230286887`; no Linux hermetic coverage is claimed until the post-`0abed5a`
  run completes.

## Non-goals / residuals

- Packaged Attempt-history / Capture-renormalize journey (bounded non-goal;
  covered by host/renderer contracts).
- Host-installed/real-machine provider roots, VoiceOver/Narrator speech,
  Developer ID signing/notarization/stapling, Windows packaging, Electron
  retirement, and #17/#38 closure.
