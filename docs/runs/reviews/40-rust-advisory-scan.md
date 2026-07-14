# Review Packet — #40 Rust Advisory Scan

## Scope

Review of the bounded RustSec CI advisory-scan slice documentation. This packet does
not evaluate or claim production readiness, Windows packaging, VoiceOver/Narrator
output, signing/notarization, Electron deletion, or dependency upgrades.

## Honesty review and post-CI closure

- Reviewer: Grok 4.5 xhigh continuation on allowlisted docs/packets only.
- Verdict: `PASS` after policy/ledger remediation and green CI.
- Findings:
  - Prior ledger wording that treated #40 as “Implemented” / implementation PASS before
    an Actions run was an overclaim; the final packet now records the green run.
  - Local Darwin output must not be treated as authoritative `cargo-audit` evidence.
  - A non-authoritative warning inventory (gtk-rs GTK3 unmaintained stack,
    `proc-macro-error`, `unic-*`, and `glib` unsound `RUSTSEC-2024-0429`) is recorded in
    the issue packet for triage only.
  - Default gate semantics remain: fail on vulnerability-class advisories; unmaintained /
    unsound warnings alone do not fail, and the 17 warning IDs are explicitly inventoried.
  - A weekly schedule was added so RustSec database changes are observed when the
    lockfile is unchanged.
- No workflow, product-code, or dependency edits are in this continuation’s allowlist.
- Formal independent rereview after the final docs update remains recorded below.

## Required checks

- `docs/gates.md` security section describes the CI command, pin, Darwin limitation,
  warning-vs-vuln rule, and final CI authority: verified by inspection.
- Issue packet and feature-dev ledger mark #40 complete only after the green CI run:
  verified by inspection.
- Node 26 legacy/docs suite: passed; 104 tests.
- `cargo tree --workspace --locked`: passed.
- `npm audit --audit-level=moderate --ignore-scripts`: passed; 0 vulnerabilities.
- Authoritative `cargo audit` result: **passed** in workflow `29213826861`; 17 allowed
  warnings were emitted and no vulnerability-class advisory failed the job.
- Local Darwin `cargo-audit` 0.22.2: exit 0 with 17 allowed warnings; diagnostic only,
  not authoritative.
- Post-remediation Grok 4.5 xhigh rereview: `FINDINGS` only for stale pre-CI wording in
  this packet; that wording is corrected above. The policy, workflow pin, weekly refresh,
  warning inventory, final CI run, and no-overclaim boundary now pass.
- Final post-CI Grok 4.5 xhigh rereview: `PASS`; no remediation remains.
- CodeRabbit CLI: initial attempts were rate-limited before analysis; a docs-only pass
  completed with 0 findings, and a later wording-only retry was rate-limited. Grok policy
  rereview was also completed.

## Residuals

- The RustSec workflow is now the authoritative gate evidence; local output and the
  warning inventory remain diagnostic context, not a clean-advisory claim.
- Screen-reader, signed-release, Windows, production, and Electron-retirement gates
  remain explicit out-of-scope items from prior cutover residuals.
- Transitive gtk-rs / glib / unic / proc-macro-error warnings may still appear on a
  green vulnerability-clean CI run; that is expected under the current gate rule and is
  not a silent Complete claim by itself.
