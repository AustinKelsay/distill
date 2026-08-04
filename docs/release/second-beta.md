# Distill 0.2.0-beta.2

This is the second beta release of the Rust-first Distill rebuild, following
`v0.2.0-beta.1`.

## Scope delta over beta.1

- Add Pi as a first-class file-backed Source adapter (#56), bringing the v1 Source
  set to six: Fixture, Codex, Claude Code, OpenCode, Droid, and Pi. Pi is
  detected under a configured sessions root and requires the `pi` executable on PATH
  (detection reports `unavailable` / `executable_not_found` without leaking
  provider text); sync/parse replay is file-backed and never invokes a provider
  subprocess. Discovery resolves session identity from the `session` header line first,
  with deterministic filename-stem and synthetic fallbacks.
- Enforce rebuild gates on main (#55): `.github/workflows/rebuild-ci.yml` runs the
  core Rust and desktop rebuild commands on qualifying pull requests into `main`.
- Windows MSI beta version mapping already carried by the tag (`0.2.0-2` → WiX
  `0.2.0.2`), matching the beta.1 packaging convention.

## Product boundary (unchanged from beta.1)

The beta ships the Rust Library, thin Rust CLI, Tauri 2 desktop host, and
React/TypeScript/Vite renderer. The old Electron/TypeScript product source is
retired and is not bundled or required. Legacy Electron homes remain supported as
read-only migration input through the native migration seam.

## Release artifacts

The tag workflow builds:

- macOS `.app` archive on `macos-14`
- Linux `.deb` and AppImage on Ubuntu 24.04
- Windows NSIS and MSI installers on `windows-latest`

Release builds never enable the smoke-only `VITE_DISTILL_SMOKE_DOM_ACTIVATE`
route. The Linux package smoke is a separate CI workflow and creates that flag only
in a temporary file before packaging.

## Verification

Before tagging, run:

```bash
npm ci
npm run release:check
npm run check:docs
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run desktop:typecheck
npm run desktop:lint
npm run desktop:format
npm run desktop:test
npm run desktop:frontend:build
```

## Signing and support boundaries (unchanged from beta.1)

The first-beta workflow intentionally builds an unsigned/ad-hoc macOS artifact;
Developer ID signing, hardened runtime, notarization, and ticket stapling are a
follow-up release gate rather than a beta claim. Windows installers are
build-verified but have no automated UI smoke claim in beta. The Windows MSI uses
the platform-only numeric Tauri version `0.2.0-2`, which WiX maps to package
version `0.2.0.2`; the release tag, release metadata, and artifact names
remain the canonical `0.2.0-beta.2` beta version. This mapping is kept in
`tauri.windows.beta.conf.json` and enforced by `npm run release:check`.
Screen-reader speech, live-user-home migration, and host-installed provider
behavior remain human or out-of-scope validation gates.
