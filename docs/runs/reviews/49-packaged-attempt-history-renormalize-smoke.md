# Review Packet — #49 Packaged Attempt-history + Capture-renormalize Smoke

## Review context

- Issue: [#49](https://github.com/AustinKelsay/distill/issues/49)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point: post-#48 on `feature/distill-clean-rebuild`
- Worker: Cursor Grok 4.5 High bounded audit; Codex integration
- Loop: Matt Pocock skills v1.1 / Plebdev Feature Dev loop v0.4.0
- Review mode: independent Grok standards/spec review, followed by Codex
  remediation and exact-head CI
- Review status: remediation in progress; final status is pending until the
  packaged Ubuntu journey and advisory/rebuild checks complete.

## Scope reviewed

Linux installed-package smoke only: Activity-discovered Capture id, initial
Attempt row, same-Capture renormalize report/list, retained Fixture
curation/export/restart/containment, and the `PKG-006` Darwin manual fallback.
No parser-version preference UI, provider-root reread, host-installed provider,
signing, Windows packaging, or Electron changes.

## Initial findings and remediation

The independent Grok review initially returned `STANDARDS_STATUS: fail` and
`SPEC_STATUS: fail` for four bounded findings:

1. The Linux smoke asserted parser text and `attempt 2` but not the discovered
   Capture id, initial `#1` outcome, or Capture/outcome on the retry report.
2. This review packet was referenced before it existed.
3. macOS still described Attempt-history/renormalize as a non-goal despite the
   new `PKG-006` manual-required row.
4. The cutover sentence sounded green while `LPKG-006` was still pending exact-
   head Ubuntu evidence.

Codex remediation adds AT-SPI waits for `Capture 1`, `#1`, `succeeded`, the
same Capture id on the retry report, `attempt 2`, and `#2`; adds the explicit
Darwin manual checklist; retires macOS non-goal wording; and softens the
cutover language until exact-head evidence is green.

The first exact-head Ubuntu package run then failed before the new slice at
`invalid_configured_root`: the corrected Codex input had not settled before
Sync. A follow-up experiment showed re-running Detect is not valid for this
hermetic root because Detect correctly requires a host executable. The smoke
now types the corrected Codex root with a slower AT-SPI/`xdotool` delay, blurs
the input, and waits one second before Sync, making the draft-settling boundary
explicit without changing Detect semantics.

## Verification evidence

- `node --check apps/distill-desktop/scripts/linux-package-smoke.mjs` — passed.
- `npm --prefix apps/distill-desktop run test:hermetic-fixtures` — passed.
- Desktop renderer tests: 48 passed; typecheck, lint, format, and frontend
  build passed.
- CodeRabbit was attempted on unstaged changes, reached summarization, and was
  bounded/terminated without findings; a post-remediation retry was rate-limited
  (`waitTime: 51 seconds`), and the final script-only retry completed with 0
  findings. Grok's rereview returned `STANDARDS_STATUS: pass` and
  `SPEC_STATUS: pass`.
- Exact-head Ubuntu package/install/smoke and Rust advisory/rebuild checks are
  pending and required to promote `LPKG-006`.

## Explicit residuals

Darwin `PKG-006` remains manual-required when System Events cannot expose the
packaged window; VoiceOver/Narrator speech, Developer ID signing/notarization,
Windows packaging, host-installed providers, Electron retirement, advisory
warning policy, and #17/#38 closure remain human or out-of-scope residuals.
