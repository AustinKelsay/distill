# Issue Session — #49 Packaged Attempt-history + Capture-renormalize Smoke

## Issue

- Issue: [#49](https://github.com/AustinKelsay/distill/issues/49)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: post-#48 on `feature/distill-clean-rebuild`
- Worker session: Cursor Grok 4.5 High bounded audit; Codex integration
- Loop: Matt Pocock skills v1.1 / Plebdev Feature Dev loop v0.4.0
- Status: Linux harness and contract documentation in progress; exact-head
  Ubuntu package evidence is required before closeout. Darwin `PKG-006` may
  remain manual-required when System Events cannot expose the packaged window.
- Review packet: `docs/runs/reviews/49-packaged-attempt-history-renormalize-smoke.md`

## Intended Contract

After the retained packaged Fixture search/detail journey, drive the existing
bridge-only Attempt-history controls. `Load Attempt history` must discover the
selected Session's Capture through Activity and render an immutable Attempt row
with Capture id, outcome, and parser id/version. `Renormalize Capture` must retry
the same Capture through the existing Distill-owned bridge seam, expose the
Attempt/report, and leave the Fixture/provider roots byte-stable. The retained
curation, JSONL export, restart, artifact, and containment assertions remain
part of the same journey.

This slice does not add parser-version preference UI, provider-root controls,
host-installed provider claims, or new product policy. It does not claim
screen-reader speech, signing/notarization, Windows packaging, or Electron
retirement.

## Testing Seam

- Primary: installed Ubuntu `.deb` under `dbus-run-session`/Xvfb with AT-SPI and
  `xdotool`, using the existing `linux-package-smoke.mjs` journey.
- Darwin: the corresponding `PKG-006` row is manual-required if AX cannot see
  the packaged window; no failure is silently promoted to a pass.
- Existing renderer/host/CLI Attempt contracts remain the product-policy
  coverage; this issue only adds packaged caller evidence.
- Forbidden shortcuts: SQL/storage access in the renderer, parser-version
  controls, provider-root rereads, Electron edits, signing, Windows packaging,
  and host-installed/real-machine provider roots.

## Verification Plan

- `node --check apps/distill-desktop/scripts/linux-package-smoke.mjs`
- `npm --prefix apps/distill-desktop run test:hermetic-fixtures`
- Desktop typecheck, lint, format, renderer tests, and frontend build.
- Rust workspace, fault-injection, format, clippy, dependency-tree, and diff
  gates as required by the rebuild workflow.
- Attempt CodeRabbit on unstaged changes; if bounded/rate-limited, record the
  independent Grok standards/spec review fallback.
- Exact-head Ubuntu package/install/smoke and Rust advisory workflows; promote
  `LPKG-006` only after the packaged journey reaches the retry report and the
  retained export/restart/containment checks.

## Darwin `PKG-006` manual checklist

When System Events exposes the packaged window, complete the retained Fixture
search/detail journey, activate `Load Attempt history`, and verify the visible
status includes a discovered Capture id plus the initial `#1` Attempt with
`fixture/1.0.0` and its outcome. Activate `Renormalize Capture` and verify the
same Capture id, `attempt 2`, parser id/version, and outcome appear in the
report/list. Confirm the existing curation/export/restart/Fixture-root checks
still pass. Record `manual_required` rather than `passed` if AX cannot expose
the window; this checklist does not claim VoiceOver speech.

## Evidence Symbols

- `apps/distill-desktop/scripts/linux-package-smoke.mjs::runUiJourney`
- `linux-package-smoke.mjs` evidence field `attempt_history_renormalize`
- Matrix/evidence IDs: `PKG-006`, `LPKG-006`

## Non-goals / residuals

- Darwin AX automation when the packaged window is not exposed; human
  VoiceOver/Narrator speech, signing/notarization/stapling, Windows packaging,
  host-installed providers, Electron retirement, Rust advisory warning policy,
  and #17/#38 closure remain explicit residuals.
