# Issue Session — #50 Packaged Hermetic Legacy Electron-home Import Smoke

This issue packet preserves pre-beta legacy-home evidence. Old root `src/**`
paths are historical; the shipped migration seam is Rust-owned and the smoke
harness is under `apps/distill-desktop/scripts/`.

## Issue

- Issue: [#50](https://github.com/AustinKelsay/distill/issues/50)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: post-#49 on `feature/distill-clean-rebuild`
- Worker session: Cursor Grok 4.5 bounded Feature Dev slice
- Loop: Matt Pocock skills v1.1 / Plebdev Feature Dev loop v0.4.0
- Status: implementation complete and promoted by exact-head Ubuntu package
  evidence. The smoke includes the WebKitGTK `Return`-key compatibility path,
  native form fallback, and an opt-in package-native DOM activation route for
  the installed Linux smoke. Linux `LPKG-007` is passed at exact head
  `0cedf9c`; Darwin `PKG-007` remains manual-required when System Events cannot
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

- Promoted at exact head `0cedf9c9c268bf3f53eb5f7ded82b4678376cc08` by the
  Ubuntu package/install/smoke run
  [29290567000](https://github.com/AustinKelsay/distill/actions/runs/29290567000),
  package-smoke job `86952998066`. The installed `.deb` completed the full
  hermetic journey and emitted `Migration report: ok=true reused=false
  captures=1 sessions=1`; the migrated session was searchable and selectable,
  and the source-home SHA-256 values (including `distill.db-wal` and
  `distill.db-shm`) were unchanged. Detect/Sync, Fixture Attempt history and
  same-Capture renormalize, curation, export, restart, artifact, and
  containment checks also passed in the same run.
- The exact-head rebuild CI run
  [29290567121](https://github.com/AustinKelsay/distill/actions/runs/29290567121)
  and Rust advisory run
  [29290566994](https://github.com/AustinKelsay/distill/actions/runs/29290566994)
  are green for the same head. The package evidence is installed-host Ubuntu
  evidence over temporary hermetic roots; it does not claim a live user home,
  host-installed providers, screen-reader speech, or a signed release.
- Earlier runs are retained below as diagnosis history rather than current
  status. They stopped at the WebKitGTK/Xvfb synthetic activation boundary and
  were superseded by the successful exact-head run.
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
  fallback, with renderer regression coverage. The successful exact-head run
  below proves that this compatibility path is sufficient in the installed
  Ubuntu package.
- The packaged harness now exercises four bounded keyboard transports—focused
  and window-targeted `Return`/`Enter`—before the existing pointer and AT-SPI
  fallbacks. This broadens only automation transport coverage; it does not add
  a storage shortcut or change migration policy.
- The Linux smoke package writes a temporary
  `apps/distill-desktop/.env.production.local` containing
  `VITE_DISTILL_SMOKE_DOM_ACTIVATE=1` only in the package workflow, then removes
  it after packaging. Tauri's `beforeBuildCommand` is explicitly rooted at the
  renderer workspace so that file is loaded by Vite. After the smoke scrolls
  the migration panel into view through the focused input helper, the renderer
  exposes an accessible smoke-only marker in the migration button name and
  live region, waits for the real migration input value, and invokes the
  existing form through native button activation and a bounded DOM submit-event
  fallback inside the packaged WebView. If WebKitGTK still leaves the status
  idle, the smoke-only route calls the existing renderer `onImportLegacy`
  handler through a ref holding the current state; this remains bridge-only and
  package-flagged. Normal builds never enable it and the shipped CSP is
  unchanged. The exact-head package run records the resulting report, session,
  and source-hash contract below.
- Exact-head run [29272756471](https://github.com/AustinKelsay/distill/actions/runs/29272756471)
  included the Vite-built renderer route but still stopped at
  `Migration status: idle`; the marker/native-click revision must be validated
  by the next exact-head run.
- Exact-head run [29274953615](https://github.com/AustinKelsay/distill/actions/runs/29274953615)
  exercised the temporary env-file workflow but still stopped before marker
  exposure; the Tauri hook working directory was not yet rooted at the renderer
  workspace. Exact-head run [29280938632](https://github.com/AustinKelsay/distill/actions/runs/29280938632)
  then proved the rooted hook and button marker were packaged, but every
  bounded native/AT-SPI activation transport still left `Migration status:
  idle`; no migration report or source-home hash evidence was emitted. The
  follow-up renderer-handler fallback described above was then promoted by
  [29290567000](https://github.com/AustinKelsay/distill/actions/runs/29290567000).

### Diagnosis boundary

The earlier failure was isolated to synthetic activation of the installed
WebKitGTK renderer under Xvfb/AT-SPI: shell/package setup, the seeder, Rust
Library and CLI migration, Tauri host, React handler/state tests, and the
retained package journey all passed. The final package-native, smoke-only
renderer route made the existing bridge handler observable without adding
renderer storage authority or changing the shipped CSP. The successful run
promotes only the Linux hermetic contract; it is not evidence for live-user
homes, host-installed providers, screen-reader speech, or signed release
packaging.

## Non-goals / residuals

- Darwin AX automation when the packaged window is not exposed; human
  VoiceOver/Narrator speech; signing/notarization/stapling; Windows packaging;
  host-installed providers; Electron retirement; Rust advisory warning policy;
  and #17/#38 closure remain explicit residuals.
