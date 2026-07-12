# Review Packet — #35 macOS Packaging

## Review Scope

- Issue: #35
- Slice: Tauri macOS `.app` packaging and packaged source-to-export smoke
- Baseline: `76fb500`
- Implementation: pending (working tree; update after commit)

## Review Findings And Remediation

The independent Grok 4.5 xhigh review initially returned FAIL with six evidence
blockers: the built plist minimum OS was not asserted, signing classification was not
constrained to the documented local ad-hoc/unsigned gate, search was optional,
`distill.db` was not required, write containment was tautological, and the gap language
was broader than the packaged harness. It also identified two non-blocking risks: the
favorite-only curation mutation could publish an empty `train` export, and Fixture
immutability compared only paths.

The smoke now asserts `LSMinimumSystemVersion=12.0`, fails if codesign reports a
non-local signing class, requires a matching post-search row, requires `distill.db` and
a non-empty JSONL export, clicks the `train` curation label, compares Fixture SHA-256
contents, and rejects new files under the temp parent outside the chosen home/Fixture
roots. GAP-R007 wording is narrowed to the macOS journey actually automated; dialog
focus/cancellation-focus and screen-reader evidence remain open human/Linux gates.

Final independent Grok 4.5 xhigh rereview: PASS with no blockers after the remediation
pass. The only remaining observations were non-blocking evidence-strengthening ideas
(for example, parsing the exported record, which this slice now does).

## Review Checklist

- [x] Bundle identifier, product name, minimum macOS, icon, and capability source match
  canonical docs.
- [x] The smoke uses a clean temporary home/Fixture root and drives the packaged UI
  through sync, search, detail, curation, export, quit/relaunch, and artifact checks.
- [x] The smoke proves chosen-home containment and Fixture immutability without broad
  privacy or encryption claims.
- [x] Signing/notarization language is accurate for `--no-sign` local evidence.
- [x] Linux and assistive-technology work remain explicitly open.

## Verification Record

- `cargo fmt --all -- --check` — pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — pass.
- `cargo test --workspace` — pass.
- `cargo test -p distill-library --features test-faults` — pass.
- `cargo build -p distill-desktop --release` — pass.
- `npm run desktop:typecheck` — pass.
- `npm run desktop:lint` — pass.
- `npm run desktop:format` — pass.
- `npm run desktop:test` — 39 tests pass.
- `npm run desktop:frontend:build` — pass.
- `npm --prefix apps/distill-desktop run a11y:smoke` — 12 tests pass.
- `npm run desktop:package:macos` — pass; local `--no-sign` `.app` emitted.
- `npm run desktop:smoke:macos` — pass on `darwin arm64`; UI and restart passed,
  `distill.db` and non-empty curated train JSONL verified.
- CodeRabbit CLI found one minor signing-classification issue; adding `code: 0` to
  successful command results fixed it, and the focused node/smoke checks passed.
