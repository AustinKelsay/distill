# Issue Session — #40 Rust Advisory Scan

## Issue

- Issue: [#40](https://github.com/AustinKelsay/distill/issues/40)
- Fixed point before session: `05d9294`
- Implementation: uncommitted on this branch (do not claim a commit SHA until committed)
- Status: In progress — allowlisted docs/packets updated; CI advisory evidence not yet recorded
- Review packet: `docs/runs/reviews/40-rust-advisory-scan.md`

## Intended Contract

- A reproducible GitHub Actions job scans the checked-in `Cargo.lock` / workspace with
  a pinned `cargo-audit` release and fails on vulnerability-class advisories. The
  default threshold is explicit: green does not mean advisory-clean when the scanner
  reports unmaintained or unsound warnings.
- No product code changes and no dependency upgrades are required unless CI reports a
  directly actionable vulnerability advisory that must be remediated separately.
- Local Darwin `cargo-audit` availability remains an explicit limitation; CI is
  authoritative for the Rust advisory-database gate. The local 0.22.2 probe exits 0
  with 17 allowed warnings, inventoried below; this is diagnostic only, not CI evidence.
- Scope stays bounded: no production deployment, Windows packaging, VoiceOver/Narrator,
  signing/notarization, or Electron deletion.

## Implementation (docs-only continuation)

- This continuation edits only `docs/gates.md`, the feature ledger, this issue packet,
  and the review packet. It does not modify the workflow, product code, or dependencies.
- `.github/workflows/rust-audit.yml` (already present for this slice) runs
  `cargo audit --file Cargo.lock` on Ubuntu 24.04 with pinned `cargo-audit` `0.22.2`,
  on Cargo/workspace changes and on a weekly schedule.
- `docs/gates.md` records the CI command, pin, Darwin limitation, warning-vs-vuln
  failure rule, and that CI evidence is still pending.

## Recorded `cargo audit` warning inventory (non-authoritative)

Command under discussion:

```bash
cargo audit --file Cargo.lock
```

Local Darwin note: `cargo-audit` 0.22.2 is available in this session, but the inventory
below is still recorded for honesty and triage; it is **not** a CI result and must not
be cited as gate evidence until an Ubuntu Actions run is linked.

| Crate              | Version | Kind         | ID                | Title                                                                        |
| ------------------ | ------- | ------------ | ----------------- | ---------------------------------------------------------------------------- |
| atk                | 0.18.2  | unmaintained | RUSTSEC-2024-0413 | gtk-rs GTK3 bindings - no longer maintained                                  |
| atk-sys            | 0.18.2  | unmaintained | RUSTSEC-2024-0416 | gtk-rs GTK3 bindings - no longer maintained                                  |
| gdk                | 0.18.2  | unmaintained | RUSTSEC-2024-0412 | gtk-rs GTK3 bindings - no longer maintained                                  |
| gdk-sys            | 0.18.2  | unmaintained | RUSTSEC-2024-0418 | gtk-rs GTK3 bindings - no longer maintained                                  |
| gdkwayland-sys     | 0.18.2  | unmaintained | RUSTSEC-2024-0411 | gtk-rs GTK3 bindings - no longer maintained                                  |
| gdkx11             | 0.18.2  | unmaintained | RUSTSEC-2024-0417 | gtk-rs GTK3 bindings - no longer maintained                                  |
| gdkx11-sys         | 0.18.2  | unmaintained | RUSTSEC-2024-0414 | gtk-rs GTK3 bindings - no longer maintained                                  |
| gtk                | 0.18.2  | unmaintained | RUSTSEC-2024-0415 | gtk-rs GTK3 bindings - no longer maintained                                  |
| gtk-sys            | 0.18.2  | unmaintained | RUSTSEC-2024-0420 | gtk-rs GTK3 bindings - no longer maintained                                  |
| gtk3-macros        | 0.18.2  | unmaintained | RUSTSEC-2024-0419 | gtk-rs GTK3 bindings - no longer maintained                                  |
| proc-macro-error   | 1.0.4   | unmaintained | RUSTSEC-2024-0370 | proc-macro-error is unmaintained                                             |
| unic-char-property | 0.9.0   | unmaintained | RUSTSEC-2025-0081 | `unic-char-property` is unmaintained                                         |
| unic-char-range    | 0.9.0   | unmaintained | RUSTSEC-2025-0075 | `unic-char-range` is unmaintained                                            |
| unic-common        | 0.9.0   | unmaintained | RUSTSEC-2025-0080 | `unic-common` is unmaintained                                                |
| unic-ucd-ident     | 0.9.0   | unmaintained | RUSTSEC-2025-0100 | `unic-ucd-ident` is unmaintained                                             |
| unic-ucd-version   | 0.9.0   | unmaintained | RUSTSEC-2025-0098 | `unic-ucd-version` is unmaintained                                           |
| glib               | 0.18.5  | unsound      | RUSTSEC-2024-0429 | Unsoundness in `Iterator` / `DoubleEndedIterator` for `glib::VariantStrIter` |

No vulnerability-class advisory is asserted here. Default `cargo audit` fails on
actionable vulnerabilities; unmaintained/unsound warnings alone do not fail the gate.
CI must still confirm the live advisory-database result.

## Verification

- Docs/packets for #40 updated to in-progress with no CI overclaim.
- `PATH=/opt/homebrew/Cellar/node/26.0.0/bin:$PATH npm test` — passed; 104 tests.
- `cargo tree --workspace --locked` — passed.
- `npm audit --audit-level=moderate --ignore-scripts` — passed; 0 vulnerabilities.
- Local Darwin `cargo-audit --file Cargo.lock` 0.22.2 — exit 0 with 17 allowed warnings;
  diagnostic only, not authoritative gate evidence.
- CodeRabbit CLI — rate-limited before analysis; Grok policy rereview findings were
  applied, with no remaining implementation remediation before CI.
- GitHub Actions run for `.github/workflows/rust-audit.yml` — pending; no run ID recorded.
- Product code / dependency upgrades — not modified in this continuation.

## Remaining Scope

- First Ubuntu workflow run (green or advisory-failing) must be recorded with a run URL
  before #40 can move to Complete.
- If CI reports actionable vulnerability advisories, remediate in a follow-up that may
  upgrade dependencies; this slice only adds the scan gate.
- Production release, Windows packaging, screen-reader claims, signing/notarization, and
  Electron deletion remain outside this slice.
