# Distill Verification Gates

Canonical rebuild verification commands for the Rust Library, thin callers, desktop host, and legacy Electron baseline.

Run from the repository root with a modern Node toolchain on `PATH` when desktop or legacy npm suites are included.

## Core Rust gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

## Feature-enabled fault contracts (#21)

Fault injection is behind the non-default `test-faults` Cargo feature and is absent from production default builds:

```bash
cargo test -p distill-library --features test-faults
```

Launcher mode:

```bash
node scripts/run-library-checks.mjs faults
```

## Desktop gates

```bash
npm run desktop:typecheck
npm run desktop:test
npm run desktop:frontend:build
cargo build -p distill-desktop
# release host proof without packaging:
cargo tauri build --no-bundle
# or npm run desktop:build when configured
```

## Legacy Electron baseline

```bash
npm test
```

Preferred Node for the documented legacy suite: Node 26 (`/opt/homebrew/Cellar/node/26.0.0/bin` on this machine). Node 22 may hit the known inspector incompatibility.

## Combined launcher

```bash
node scripts/run-library-checks.mjs rebuild   # fmt + clippy + cargo test
node scripts/run-library-checks.mjs library   # same as rebuild
node scripts/run-library-checks.mjs faults    # test-faults suite
node scripts/run-library-checks.mjs desktop   # typecheck/lint/format/test/frontend build
node scripts/run-library-checks.mjs npm       # legacy npm test
node scripts/run-library-checks.mjs all       # full package
```
