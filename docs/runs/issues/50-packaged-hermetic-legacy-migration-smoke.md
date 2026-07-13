# Issue Session — #50 Packaged Hermetic Legacy Electron-home Import Smoke

## Issue

- Issue: [#50](https://github.com/AustinKelsay/distill/issues/50)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: post-#49 on `feature/distill-clean-rebuild`
- Worker session: Cursor Grok 4.5 bounded Feature Dev slice
- Loop: Matt Pocock skills v1.1 / Plebdev Feature Dev loop v0.4.0
- Status: implementation in worktree; Linux `LPKG-007` pending exact-head Ubuntu
  package/install/smoke promotion. Darwin `PKG-007` is manual-required when
  System Events cannot expose the packaged window.
- Review packet: `docs/runs/reviews/50-packaged-hermetic-legacy-migration-smoke.md`

## Intended Contract

Build a temporary synthetic legacy Electron home using the existing host/CLI
fixture shape (`distill.db`, empty `blobs/`/`exports/`, one Fixture
Source/Capture/Session, and regular-file journal sidecars). Drive the existing
bridge-only Legacy Electron migration panel in the installed Linux package
smoke: type Distill home + legacy source path, activate `Import legacy home`,
and assert safe report fields (`Migration status: success`, `ok: true`,
captures/sessions counts, `reused: false`). Search/detail must find the migrated
session. Before/after SHA-256 of the synthetic legacy home (including
`distill.db-wal` / `distill.db-shm`) must match. The retained Fixture
Detect/Sync/search/detail/Attempt/renormalize/curation/export/restart/artifact/
containment journey remains part of the same installed-host run.

This slice reuses existing Library/CLI/Tauri/React migration policy and callers.
It does not edit Electron product sources, add parser-version preference UI, or
claim signing/notarization, Windows packaging, host-installed providers, or
screen-reader speech.

## Testing Seam

- Primary: installed Ubuntu `.deb` under `dbus-run-session`/Xvfb with AT-SPI and
  `xdotool`, using `linux-package-smoke.mjs` plus
  `packaged-hermetic-legacy-home.mjs`.
- Darwin: `PKG-007` is an explicit manual AX checklist when the packaged window
  is exposed; never silently promote a missing-window run to a pass.
- Existing Library `LMI-*`, CLI migrate, host `host_legacy_import`, and React
  migration panel contracts remain the product-policy coverage; this issue only
  adds packaged caller evidence.
- Forbidden shortcuts: SQL/CAS/storage authority in the renderer, Electron
  `src/**` edits, live user homes, parser-version controls, signing, Windows
  packaging, and host-installed/real-machine provider roots.

## Verification Plan

- `node --check` on smoke/seeder scripts
- `npm --prefix apps/distill-desktop run test:hermetic-fixtures`
- Desktop typecheck, lint, format, renderer tests, and frontend build as needed
- Exact-head Ubuntu package/install/smoke before promoting `LPKG-007`
- Leave `PKG-007` as `manual_required` until Darwin AX exposes the window

## Darwin `PKG-007` manual checklist

When System Events exposes the packaged window, seed a temporary host/CLI-shaped
legacy home beside the chosen Distill home (siblings, not ancestor/alias). Type
both paths, activate `Import legacy home`, and verify `Migration status: success`,
`ok: true`, `reused: false`, and the expected captures/sessions counts. Search for
the migrated session title and open detail. Confirm before/after SHA-256 of the
legacy home including `distill.db-wal`/`distill.db-shm` is unchanged, then retain
the existing Fixture Detect/Sync/search/Attempt/curation/export/restart checks.
Record `manual_required` rather than `passed` if AX cannot expose the window;
this checklist does not claim VoiceOver speech or a live-user legacy home.

## Evidence Symbols

- `apps/distill-desktop/scripts/packaged-hermetic-legacy-home.mjs::seedHermeticLegacyHome`
- `apps/distill-desktop/scripts/linux-package-smoke.mjs::runUiJourney`
- `linux-package-smoke.mjs` evidence field `hermetic_legacy_migration`
- Matrix/evidence IDs: `PKG-007`, `LPKG-007`

## Exact-head evidence

- Not yet promoted. Local hermetic seeder `node --test` and script syntax checks
  pass in the worktree. Promote `LPKG-007` only after exact-head Ubuntu
  package/install/smoke records the migration report, migrated-session
  discoverability, sidecar-inclusive source-home immutability, and the retained
  Fixture journey.

The first exact-head Ubuntu attempt [29248181597](https://github.com/AustinKelsay/distill/actions/runs/29248181597)
failed at `Migration status: idle`: the coordinate-only Import activation was
inert under AT-SPI even though the control was discoverable. The harness now
uses the semantic AT-SPI action helper for `Import legacy home`; this failure is
recorded rather than counted as `LPKG-007` evidence.

## Non-goals / residuals

- Darwin AX automation when the packaged window is not exposed; human
  VoiceOver/Narrator speech; signing/notarization/stapling; Windows packaging;
  host-installed providers; Electron retirement; Rust advisory warning policy;
  and #17/#38 closure remain explicit residuals.
