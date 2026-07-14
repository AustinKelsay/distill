# Distill 0.2.0-beta.1

This is the first beta release of the Rust-first Distill rebuild.

## Product boundary

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
route. The Linux package smoke is a separate CI workflow and creates that flag
only in a temporary file before packaging.

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

The installed Ubuntu package smoke remains authoritative for the Linux runtime
journey, including hermetic legacy migration (`LPKG-007`). macOS Accessibility
focus and legacy migration checks remain manual when System Events cannot expose
the packaged window.

## Signing and support boundaries

The release workflow is configured to consume Apple signing/notarization secrets
when the repository provides them. Without those secrets, macOS output is an
unsigned developer artifact and must be labeled as such. Windows installers are
build-verified but have no automated UI smoke claim in beta. Screen-reader speech,
live-user-home migration, and host-installed provider behavior remain human or
out-of-scope validation gates.
