# Issue Session — #50 Packaged Hermetic Legacy Electron-home Import Smoke

## Issue

- Issue: [#50](https://github.com/AustinKelsay/distill/issues/50)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: post-#49 on `feature/distill-clean-rebuild`
- Worker session: Cursor Grok 4.5 bounded Feature Dev slice
- Loop: Matt Pocock skills v1.1 / Plebdev Feature Dev loop v0.4.0
- Status: implementation complete locally, including a WebKitGTK `Return`-key
  compatibility path, native form fallback, and an opt-in package-native DOM
  native DOM activation route for the installed Linux smoke; Linux `LPKG-007`
  remains pending exact-head promotion until this route records the full
  contract. Darwin `PKG-007` remains manual-required when System Events cannot
  expose the packaged window.
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

- Not promoted. Local hermetic seeder `node --test`, script syntax checks, and
  the Library/CLI/host/renderer migration contracts pass. No Ubuntu run has
  produced the packaged migration report, migrated-session discoverability, or
  sidecar-inclusive source-home hash evidence required for `LPKG-007`.
- Runs [29248181597](https://github.com/AustinKelsay/distill/actions/runs/29248181597),
  [29248740562](https://github.com/AustinKelsay/distill/actions/runs/29248740562),
  and [29249235142](https://github.com/AustinKelsay/distill/actions/runs/29249235142)
  all stopped at `Migration status: idle` after the packaged Import control was
  discoverable but did not enter the bridge action.
- Run [29249994169](https://github.com/AustinKelsay/distill/actions/runs/29249994169)
  showed that AT-SPI does not expose the HTML input value as accessible text.
  Run [29253668633](https://github.com/AustinKelsay/distill/actions/runs/29253668633)
  then proved the migration input could receive focus, and
  [29254323223](https://github.com/AustinKelsay/distill/actions/runs/29254323223)
  proved the React `Import legacy home (ready)` state. Button focus also passed
  in [29255028326](https://github.com/AustinKelsay/distill/actions/runs/29255028326),
  but keyboard activation still left status idle.
- The latest focused-input Return plus `--clearmodifiers` attempt,
  [29260415053](https://github.com/AustinKelsay/distill/actions/runs/29260415053),
  still stopped at `Migration status: idle`. A direct-window click experiment in
  [29259709770](https://github.com/AustinKelsay/distill/actions/runs/29259709770)
  regressed the existing repair-dialog journey and was reverted.
- The post-closeout exact-head package run
  [29261962986](https://github.com/AustinKelsay/distill/actions/runs/29261962986)
  reproduced the same `Migration status: idle` failure after package build and
  install; no packaged migration report or source-home hash evidence was emitted.
- The later docs-only rerun
  [29262524827](https://github.com/AustinKelsay/distill/actions/runs/29262524827)
  reproduced that same boundary after build and install.
- A local follow-up accepts both DOM `Enter` and WebKitGTK's `Return` key name
  on the migration source field, and wraps the controls in a native form submit
  fallback, with renderer regression coverage. The next exact-head package run
  must determine whether the key-name/event-delivery boundary was the cause;
  `LPKG-007` is not promoted by the local test alone.
- The packaged harness now exercises four bounded keyboard transports—focused
  and window-targeted `Return`/`Enter`—before the existing pointer and AT-SPI
  fallbacks. This broadens only automation transport coverage; it does not add
  a storage shortcut or change migration policy.
- The Linux smoke package writes a temporary
  `apps/distill-desktop/.env.production.local` containing
  `VITE_DISTILL_SMOKE_DOM_ACTIVATE=1` only in the package workflow, then removes
  it after packaging. The renderer exposes an accessible smoke-only marker,
  waits for the real migration input value, and invokes the existing form
  through native button activation with a bounded DOM submit-event fallback
  inside the packaged WebView; normal builds never enable it and the shipped
  CSP is unchanged. Exact-head CI must still prove the resulting report,
  session, and source-hash contract before `LPKG-007` is promoted.
- Exact-head run [29272756471](https://github.com/AustinKelsay/distill/actions/runs/29272756471)
  included the Vite-built renderer route but still stopped at
  `Migration status: idle`; the marker/native-click revision must be validated
  by the next exact-head run.

### Diagnosis boundary

The failure is isolated to synthetic activation of the installed WebKitGTK
renderer under Xvfb/AT-SPI: shell/package setup, the seeder, Rust Library and
CLI migration, Tauri host, React handler/state tests, and the retained package
journey all pass. This is not evidence that product migration is broken. Do not
promote `LPKG-007` or retire the packaged Linux migration residual until a real
desktop/AT-SPI instrumentation path or package-native browser automation records
the full migration contract.

## Non-goals / residuals

- Darwin AX automation when the packaged window is not exposed; human
  VoiceOver/Narrator speech; signing/notarization/stapling; Windows packaging;
  host-installed providers; Electron retirement; Rust advisory warning policy;
  and #17/#38 closure remain explicit residuals.
