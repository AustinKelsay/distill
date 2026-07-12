# Review Packet — #40 Rust Advisory Scan

## Scope

Review of the bounded RustSec CI advisory-scan slice documentation. This packet does
not evaluate or claim production readiness, Windows packaging, VoiceOver/Narrator
output, signing/notarization, Electron deletion, dependency upgrades, or a completed
CI advisory result.

## Honesty review (pre-CI)

- Reviewer: Grok 4.5 xhigh continuation on allowlisted docs/packets only.
- Verdict: `FINDINGS` initially; remediation is recorded below, with CI still pending.
- Findings:
  - Prior ledger wording that treated #40 as “Implemented” / implementation PASS before
    an Actions run was an overclaim; status is corrected to in-progress.
  - Local Darwin output must not be treated as authoritative `cargo-audit` evidence.
  - A non-authoritative warning inventory (gtk-rs GTK3 unmaintained stack,
    `proc-macro-error`, `unic-*`, and `glib` unsound `RUSTSEC-2024-0429`) is recorded in
    the issue packet for triage only.
  - Default gate semantics remain: fail on vulnerability-class advisories; unmaintained /
    unsound warnings alone do not fail, and the 17 warning IDs are explicitly inventoried.
  - A weekly schedule was added so RustSec database changes are observed when the
    lockfile is unchanged.
- No workflow, product-code, or dependency edits are in this continuation’s allowlist.
- Formal independent rereview after commit/CI remains required before a Complete claim.

## Required checks

- `docs/gates.md` security section describes the CI command, pin, Darwin absence,
  warning-vs-vuln rule, and pending CI authority: verified by inspection.
- Issue packet and feature-dev ledger mark #40 as in-progress: verified by inspection.
- Node 26 legacy/docs suite: passed; 104 tests.
- `cargo tree --workspace --locked`: passed.
- `npm audit --audit-level=moderate --ignore-scripts`: passed; 0 vulnerabilities.
- Authoritative `cargo audit` result: **pending** first CI run; no run ID claimed.
- Local Darwin `cargo-audit` 0.22.2: exit 0 with 17 allowed warnings; diagnostic only,
  not authoritative.
- Post-remediation Grok 4.5 xhigh rereview: `FINDINGS` only for the pre-commit ledger
  wording that called local `cargo-audit` absent; that wording is corrected above. The
  policy, workflow pin, weekly refresh, warning inventory, and no-overclaim boundary
  now pass; CI remains the only missing evidence.
- CodeRabbit CLI: rate-limited before analysis; Grok policy rereview was the documented
  fallback.

## Residuals

- First CI advisory scan result is not yet recorded; treat a green (or vulnerability-
  failing) Actions run as the gate evidence, not a local Darwin scan or the recorded
  warning inventory alone.
- Screen-reader, signed-release, Windows, production, and Electron-retirement gates
  remain explicit out-of-scope items from prior cutover residuals.
- Transitive gtk-rs / glib / unic / proc-macro-error warnings may still appear on a
  green vulnerability-clean CI run; that is expected under the current gate rule and is
  not a silent Complete claim by itself.
