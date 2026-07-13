# Review Packet — #46 Rebuild CI Gate

## Review context

- Issue: [#46](https://github.com/AustinKelsay/distill/issues/46)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point: `2d80280`
- Worker: Cursor Grok 4.5 bounded CI/docs slice
- Loop: Matt Pocock skills v1.1 / Plebdev Feature Dev loop v0.4.0
- Review mode: local CodeRabbit must be attempted; bounded Grok review is the fallback
  if CodeRabbit is unavailable, rate-limited, or stalls (same policy as #40–#45)
- Final review status: `PASS` — independent Grok standards/spec review passed; CI run ID
  not yet recorded (`REPLACE_AFTER_PUSH`)

## Scope reviewed

Allowlisted surfaces only:

- `.github/workflows/rebuild-ci.yml` (new continuous core-gate workflow)
- `docs/gates.md` (authoritative CI mapping for core gates)
- `docs/gaps/current-state-gap-register.md` (GAP-R003 honesty + residuals)
- `docs/testing/contract-scenario-evidence.md` (#46 continuous-gate note)
- `docs/runs/issues/46-rebuild-ci-gate.md`
- `docs/runs/reviews/46-rebuild-ci-gate.md`
- `docs/runs/feature-dev-distill-clean-rebuild.md`

This packet does **not** claim production readiness, packaged real-provider smoke,
VoiceOver/Narrator speech, signing/notarization, Windows packaging, Electron
retirement, or merge/close of #17 / #38.

## Acceptance checklist

- [x] Path-filtered PR to `staging` + `workflow_dispatch`
- [x] Least privilege (`contents: read`) and bounded timeouts
- [x] Ubuntu Rust job: fmt, clippy `-D warnings`, workspace tests, `test-faults`,
  `test-leases` Sync suite
- [x] Ubuntu desktop job: `npm ci` + typecheck/lint/format/test/frontend build
- [x] Stable Node/Rust setup conventions reused
- [x] No real providers, full-scale bench, signing, or Windows jobs in this workflow
- [x] Package smoke and RustSec workflows remain separately authoritative
- [ ] First green Actions run linked (`REPLACE_AFTER_PUSH`)
- [x] CodeRabbit local attempt bounded; it stalled in summarization, so the independent
  Grok review below is the recorded fallback

## Findings

Initial independent Grok review found three low-severity documentation precision issues:

1. The verification note said the workflow matched only `rebuild`/`desktop` launcher
   modes even though `test-faults` is a separate launcher mode. The note now names
   `rebuild` + `faults` + `desktop` explicitly.
2. The feature-dev ledger now includes the lease-enabled Sync suite in its continuous
   gate summary.
3. GAP-R003 now calls CI “implemented” while the authoritative green Actions run is
   pending, instead of calling the gate resolved ahead of evidence.

All three were remediated before this packet was recorded. No workflow or scope findings
remain. Placeholders (`REPLACE_AFTER_*`) must be replaced after push; they are not
invented run IDs.

The first implementation-head Actions run (`29223953816`) also exposed a
pre-existing environment-sensitive Codex detection test: the runner lacked a `codex`
executable, so a configured-root result was reported `unavailable` instead of `ok`.
The workflow now writes a deterministic Rust-test `PATH` to `GITHUB_ENV`, placing a
no-op Codex shim before Cargo, Rustup, and system tools. This is a CI hermeticity fix; it
does not change product detection behavior or provider claims.
The follow-up run (`29224211256`) confirmed the provider fix but caught that the first
PATH form also omitted Rustup's toolchain bin (and therefore `rustfmt`); the workflow now
retains both the Cargo bin and `dirname "$(rustup which rustfmt)"` before the next run,
with the Codex shim retained for the configured-root contract.

## Explicit residuals

- Packaged real-provider machine roots
- Human assistive-technology / screen-reader speech observation
- Developer ID signing / hardened runtime / notarization / stapling
- Windows packaging
- Electron retirement
- Root issue #17 / PR #38 merge or close
- First authoritative rebuild-CI Actions run ID (`REPLACE_AFTER_PUSH`)

## Verification record

Inspection targets:

- Workflow commands match `docs/gates.md` continuous PR list and the
  `scripts/run-library-checks.mjs` `rebuild` + `faults` + `desktop` modes.
- Gap register and contract evidence leave residuals explicit.
- Feature-dev ledger status for #46 must not claim Complete with a green CI run until
  `REPLACE_AFTER_PUSH` is replaced with a real Actions URL.

Independent review result: `STANDARDS_STATUS: pass`, `SPEC_STATUS: pass`, no remaining
findings after the three documentation remediations above. Local CodeRabbit command was
attempted with `coderabbit review --agent --type all --base staging`; it stalled during
summarization and was terminated within the bounded review window, so Grok is the
recorded fallback.
