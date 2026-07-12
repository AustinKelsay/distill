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

## Sync / Source settings gates (#22)

```bash
cargo test -p distill-library --test library_ops_sync --features test-leases
cargo test -p distill-cli --test cli_fixture_journey
cargo test -p distill-desktop --test host_fixture_journey
npm run desktop:test
```

Provider subprocess duration bounds and large-stdin cleanup are covered on macOS/Linux. Output-byte caps are covered on all platforms via a deterministic helper without spawning. Heartbeat accuracy uses the non-default `test-leases` feature only.

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
