# Review Packet — #50 Packaged Hermetic Legacy Electron-home Import Smoke

## Scope reviewed

Bounded Feature Dev slice for packaged hermetic legacy Electron-home import
smoke (Linux-first). Harness/docs only: temporary synthetic legacy home seeder,
installed Linux AT-SPI journey extension, Darwin manual checklist, and
matrix/evidence/gap/cutover/ledger honesty. No Electron product edits, no new
migration mapping policy, no parser-version UI, no signing/Windows/host-provider/
screen-reader claims.

## Review outcome

- Independent Grok standards/spec review: `PASS_WITH_FINDINGS`; no blocker for
  the in-worktree harness. Promotion remains gated on exact-head Ubuntu proof
  of migration report/session/hash contracts and Fixture Attempt identities
  `Capture 2` / `Attempt 2` / retry `Attempt 7`.
- PKG-007 evidence now names only the Darwin smoke/manual checklist; the Linux
  row names the hermetic seeder because Linux actually invokes it.
- `docs/specs/legacy-migration.md` points at packaged `PKG-007`/`LPKG-007`
  evidence without changing the Library migration policy.
- CodeRabbit reached summarization but hung in the bounded attempt; prior
  completed zero-finding review history and this independent Grok review are
  the recorded fallback.

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
- `LPKG-007` left `pending` until exact-head Ubuntu green; `PKG-007` is
  `manual_required`.
- Packaged Linux “no migration claim” wording retired only after promotion;
  cutover/gap text state that honestly.
- Empty planted `distill.db-wal`/`distill.db-shm` companions cover sidecar hash
  paths after Python sqlite checkpoints live WAL on close; live-WAL immutability
  remains Library `LMI-001`.

## Local checks recorded

- `node --check` on seeder + linux smoke scripts
- `npm --prefix apps/distill-desktop run test:hermetic-fixtures` (2 suites pass)

## Residual risks

- AT-SPI timing/journey length after inserting migration before Detect/Sync.
- Fixture Capture/Attempt IDs shift to 2 / 2 / 7 after migration-first sequencing;
  Ubuntu must confirm those identities.
- Do not promote `LPKG-007` or retire cutover residual language without exact-head
  package evidence.
