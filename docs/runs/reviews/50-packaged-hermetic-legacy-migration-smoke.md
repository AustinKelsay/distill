# Review Packet — #50 Packaged Hermetic Legacy Electron-home Import Smoke

## Scope reviewed

Bounded Feature Dev slice for packaged hermetic legacy Electron-home import
smoke (Linux-first). Harness/docs only: temporary synthetic legacy home seeder,
installed Linux AT-SPI journey extension, Darwin manual checklist, and
matrix/evidence/gap/cutover/ledger honesty. No Electron product edits, no new
migration mapping policy, no parser-version UI, no signing/Windows/host-provider/
screen-reader claims.

## Review outcome

- Independent Grok standards/spec review: `PASS_WITH_FINDINGS`; the original
  harness had no product-code blocker. A follow-up local remediation accepts
  both DOM `Enter` and WebKitGTK's `Return` key name, and wraps the controls in
  a native form-submit fallback, with a renderer regression test. The final
  package-native fallback was verified at exact head by Ubuntu run
  [29290567000](https://github.com/AustinKelsay/distill/actions/runs/29290567000),
  including migration report/session/hash contracts and Fixture Attempt
  identities `Capture 2` / `Attempt 2` / retry `Attempt 7`.
- PKG-007 evidence now names only the Darwin smoke/manual checklist; the Linux
  row names the hermetic seeder because Linux actually invokes it.
- `docs/specs/legacy-migration.md` points at packaged `PKG-007`/`LPKG-007`
  evidence without changing the Library migration policy.
- CodeRabbit reached summarization but hung in the bounded attempt; prior
  completed zero-finding review history and this independent Grok review are
  the recorded fallback.
- Exact-head attempts [29248181597](https://github.com/AustinKelsay/distill/actions/runs/29248181597),
  [29248740562](https://github.com/AustinKelsay/distill/actions/runs/29248740562),
  and [29249235142](https://github.com/AustinKelsay/distill/actions/runs/29249235142)
  failed before migration report assertions with `Migration status: idle`.
  Follow-up probes established that AT-SPI cannot expose the HTML input value,
  while input focus, React readiness, and button focus do pass in
  [29249994169](https://github.com/AustinKelsay/distill/actions/runs/29249994169),
  [29253668633](https://github.com/AustinKelsay/distill/actions/runs/29253668633),
  [29254323223](https://github.com/AustinKelsay/distill/actions/runs/29254323223),
  and [29255028326](https://github.com/AustinKelsay/distill/actions/runs/29255028326).
  The latest focused-input Return/clearmodifiers attempt
  [29260415053](https://github.com/AustinKelsay/distill/actions/runs/29260415053)
  still leaves status idle; a direct-window click regression in
  [29259709770](https://github.com/AustinKelsay/distill/actions/runs/29259709770)
  was reverted. The post-closeout exact-head package run
  [29261962986](https://github.com/AustinKelsay/distill/actions/runs/29261962986)
  reproduced the same idle boundary after build/install. The later docs-only
  rerun [29262524827](https://github.com/AustinKelsay/distill/actions/runs/29262524827)
  reproduced it again. Those attempts are retained as diagnosis history. The
  final exact-head package run
  [29290567000](https://github.com/AustinKelsay/distill/actions/runs/29290567000)
  passed `LPKG-007` at head `0cedf9c9c268bf3f53eb5f7ded82b4678376cc08`; rebuild
  CI [29290567121](https://github.com/AustinKelsay/distill/actions/runs/29290567121)
  and Rust advisory [29290566994](https://github.com/AustinKelsay/distill/actions/runs/29290566994)
  are green on the same head.

## Files

- `apps/distill-desktop/scripts/packaged-hermetic-legacy-home.mjs`
- `apps/distill-desktop/scripts/packaged-hermetic-legacy-home.node-test.mjs`
- `apps/distill-desktop/scripts/linux-package-smoke.mjs`
- `apps/distill-desktop/scripts/macos-package-smoke.mjs` (comments/non-claims)
- `apps/distill-desktop/package.json` (`test:hermetic-fixtures`)
- `docs/testing/contract-test-matrix.md`
- `docs/testing/contract-scenario-evidence.md`
- `docs/gaps/current-state-gap-register.md`
- `docs/runs/issues/37-matrix-cutover.md`
- `docs/runs/issues/50-packaged-hermetic-legacy-migration-smoke.md`
- `docs/runs/reviews/50-packaged-hermetic-legacy-migration-smoke.md`
- `docs/runs/feature-dev-distill-clean-rebuild.md`

## Standards / Spec

- Reuses existing bridge-only migration panel and Library import seam.
- Destination/source homes are siblings under the smoke temp base.
- `LPKG-007` is `passed` at exact head through the opt-in Vite-built renderer
  DOM activation path. The package workflow creates a temporary
  `.env.production.local` so Tauri's frontend build receives the smoke flag;
  `PKG-007` remains `manual_required`.
- Packaged Linux “no migration claim” wording is retired after promotion;
  cutover/gap text continues to separate hermetic Ubuntu evidence from live-user
  homes, host providers, screen-reader speech, and signed releases.
- Empty planted `distill.db-wal`/`distill.db-shm` companions cover sidecar hash
  paths after Python sqlite checkpoints live WAL on close; live-WAL immutability
  remains Library `LMI-001`.

## Local checks recorded

- `node --check` on seeder + linux smoke scripts
- `npm --prefix apps/distill-desktop run test:hermetic-fixtures` (2 suites pass)

## Residual risks

- AT-SPI timing/journey length after inserting migration before Detect/Sync is
  covered by the successful Ubuntu run; keep the explicit waits and markers
  stable if the harness changes.
- Fixture Capture/Attempt IDs shift to 2 / 2 / 7 after migration-first
  sequencing; Ubuntu run `29290567000` confirmed those identities.
- The bounded Grok diagnostic attributes the prior boundary to WebKitGTK/Xvfb
  event delivery, not to a failed Rust/CLI/host migration contract. The harness
  keeps the four bounded keyboard transports and adds a smoke-only Vite-built
  renderer marker/button-activation/submit fallback route. Run
  [29274953615](https://github.com/AustinKelsay/distill/actions/runs/29274953615)
  failed before marker exposure because the hook was not rooted at the renderer
  workspace; run
  [29280938632](https://github.com/AustinKelsay/distill/actions/runs/29280938632)
  proved the rooted temporary-env package and button marker; the final
  renderer-handler fallback is covered by the successful run
  [29290567000](https://github.com/AustinKelsay/distill/actions/runs/29290567000).
