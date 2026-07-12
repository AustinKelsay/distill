# Issue Session — #35 macOS Packaging

## Issue

- Issue: [#35](https://github.com/AustinKelsay/distill/issues/35)
- Fixed point before session: `76fb500`
- Implementation commit: pending
- Status: Complete
- Review packet: `docs/runs/reviews/35-macos-packaging.md`

## Intended Contracts

- The Tauri package builds a deterministic macOS `.app` with identifier
  `dev.distill.desktop`, product name `Distill`, macOS 12.0 minimum, checked-in icon
  assets, and the existing `core:event:default` capability only.
- A clean temporary home and Fixture root can be used through the packaged renderer to
  run Fixture sync, search, session detail, one curation mutation, export, quit/relaunch,
  and artifact checks.
- The packaged app writes only the chosen home/export destination and does not mutate the
  Fixture source root.
- Local evidence is honest about signing: the package is built with `--no-sign` and the
  smoke reports ad-hoc/unsigned developer metadata without claiming Developer ID,
  hardened runtime, notarization, or ticket stapling.

## Implementation

- `apps/distill-desktop/src-tauri/tauri.conf.json` enables the `.app` bundle, macOS 12.0
  minimum, and deterministic PNG/ICNS icon inputs.
- `apps/distill-desktop/src-tauri/icons/icon.icns` is generated from the checked-in green
  placeholder mark; no product-brand claim is made.
- Root and workspace scripts expose `desktop:package:macos` and
  `desktop:smoke:macos`; package construction uses the installed npm Tauri CLI because
  the Cargo `tauri` subcommand is unavailable in this environment.
- `apps/distill-desktop/scripts/macos-package-smoke.mjs` performs static bundle/capability
  checks, Accessibility-driven packaged UI actions, chosen-home/Fixture containment,
  export existence, and quit/relaunch persistence checks. `DISTILL_MACOS_ALLOW_MANUAL=1`
  is an explicit fallback that reports `manual_required` and never claims a pass. The
  default gate fails when the packaged journey cannot be automated.

## Evidence

- Package command: `npm run desktop:package:macos` — pass. The bundle is emitted at
  `target/release/bundle/macos/Distill.app`; Vite frontend and Rust release compilation
  both complete.
- Smoke command: `npm run desktop:smoke:macos` — pass on `darwin arm64`.
- Recorded smoke result:
  - bundle identifier `dev.distill.desktop`, name `Distill`, macOS minimum `12.0`, icon
    `icon.icns`;
  - capability source exactly `["core:event:default"]`;
  - signing classification `adhoc`; notarization explicitly not assessed;
  - packaged UI `passed`, restart `passed`;
  - chosen home contains `distill.db` and one non-empty `exports/*.jsonl` artifact;
  - Fixture source file paths and SHA-256 contents are unchanged before/after the
    journey; new files under the smoke temp parent are constrained to the chosen home
    or Fixture root.

## Verification

- `node --check apps/distill-desktop/scripts/macos-package-smoke.mjs` — pass.
- `npm run desktop:package:macos` — pass.
- `npm run desktop:smoke:macos` — pass.
- Full Rust/desktop gates and the implementation commit are recorded in the review
  packet below.
- CodeRabbit CLI found one minor signing-classification issue; successful command
  results now carry `code: 0`, the finding is fixed, and the packaged smoke was rerun.
  Independent Grok 4.5 xhigh review initially found six evidence blockers and two
  non-blocking risks. All actionable findings were applied and the final rereview passed.

## Explicit Non-Claims

This slice does not claim migration, crash recovery, privacy hardening, scale, export
atomicity, Developer ID signing, hardened runtime, notarization, ticket stapling, or
VoiceOver/screen-reader coverage. Linux packaging remains #36.
