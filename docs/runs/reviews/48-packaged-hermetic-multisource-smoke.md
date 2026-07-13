# Review Packet — #48 Packaged Hermetic Multi-Source Smoke

## Review context

- Issue: [#48](https://github.com/AustinKelsay/distill/issues/48)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Implementation commits: `dbfe7b0`, `8d84209`, `53c3372`, `cec7876`, `b84bde4`
- Worker: Cursor Grok 4.5 bounded implementation sidecar for Codex
- Loop: Matt Pocock skills v1.1 / Plebdev Feature Dev loop v0.4.0
- Review mode: implementation packet for Codex standards/spec review before commit
- Final review status: PASS — fresh Grok standards/spec rereview; no hard
  findings. CodeRabbit first remediation pass found 0 issues; a later minor
  status-region finding was fixed. A bounded diagnostic CodeRabbit attempt
  stalled in `reviewing` without findings; no unresolved review finding remains.

## Scope reviewed

Packaged macOS/Linux smoke harnesses only: hermetic multi-Source fixture seeding,
Detect Sources sibling-failure isolation/redaction, Start Sync Run before the
retained Fixture search/detail/curation/export/restart/containment journey,
matrix/evidence rows `PKG-004`/`PKG-005`/`LPKG-004`/`LPKG-005`, and canonical
gap/gates/ledger/issue/review docs. No Rust product logic, bridge/UI product
logic, Electron `src/**`, signing config, or Windows packaging edits.

## Findings and remediation

Attempt-history/renormalize is recorded as a bounded packaged non-goal rather
than inventing new packaged controls. Fixture-only `PKG-001..003` /
`LPKG-001..003` rows remain unchanged in intent; their previous package
evidence is historical, while the modified harness still needs the local Darwin
and exact-head Ubuntu runs to confirm the combined journey.
OpenCode uses `{root}/bin/opencode` only — never a host installation.

The first review findings were remediated before this final rereview: the
helper test is excluded from Vitest discovery, Linux redaction is fail-closed,
Detect asserts healthy-sibling statuses, and Sync asserts each source reaches
`completed` rather than relying on aggregate success alone.

The packaged status indicators were then made explicitly observable to AT-SPI;
the main Sync status retained its existing section-owned live region after a
CodeRabbit duplicate-announcement finding. Those changes did not make the
installed-host Linux status assertion green, so the package rows remain
blocked rather than overstated.

## Verification evidence

- `npm --prefix apps/distill-desktop run test:hermetic-fixtures` — passed.
- `node --check` on `macos-package-smoke.mjs`, `linux-package-smoke.mjs`, and
  `packaged-hermetic-multisource.mjs` — passed.
- `python3 -m py_compile apps/distill-desktop/scripts/linux-atspi-find.py` —
  passed.
- Local Darwin unsigned package build/static gate — passed. The AX journey is
  blocked in the current runner with `Distill window did not appear`; explicit
  manual mode records `ui: manual_required` and does not claim hermetic
  coverage.
- Exact-head Ubuntu package build/install passed, but installed-host smoke
  failed at `AT-SPI accessible not found: Status: warning` on
  `29229103417`, `29229486473`, and `29229940050`; diagnostic run
  `29230286887` also failed. The packaged Linux hermetic rows remain blocked.

## Explicit residuals

- Host-installed/real-machine provider roots, VoiceOver/Narrator speech,
  Developer ID signing/notarization/stapling, Windows packaging, Electron
  retirement, packaged Attempt-history/renormalize journey, and #17/#38
  closure.
