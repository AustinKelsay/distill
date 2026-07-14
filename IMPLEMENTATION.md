# Distill implementation map

This file is informative. Target behavior is defined by the canonical docs in
[`docs/README.md`](docs/README.md).

## Shipped beta

- Library: `crates/distill-library` owns storage, parsing, sync, curation,
  export, and the read-only legacy-home migration seam.
- CLI: `crates/distill-cli` is a thin caller over the Library.
- Desktop host: `apps/distill-desktop/src-tauri` is a Tauri 2 host with a
  restricted typed bridge.
- Renderer: `apps/distill-desktop/src` is a React/Vite UI with no filesystem,
  process, shell, SQL, or Node authority.
- Release metadata: the root workspace, desktop package, and Tauri config all
  use `0.2.0-beta.1`.

The old Electron/TypeScript product implementation was removed before beta.
Only its compatibility contract remains: Rust reads Electron-shaped homes from
a private snapshot and leaves the source byte-for-byte unchanged. The boundary
and historical references are recorded in
[`docs/legacy/electron/README.md`](docs/legacy/electron/README.md).

## Useful checks

```bash
npm ci
npm run check:docs
npm run release:check
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
npm run desktop:typecheck
npm run desktop:lint
npm run desktop:format
npm run desktop:test
npm run desktop:frontend:build
```

The combined launcher is available at
`node scripts/run-library-checks.mjs all`.

## Caller contract tests

- Library: `crates/distill-library/tests/library_fixture_tracer.rs`
- CLI: `crates/distill-cli/tests/cli_fixture_journey.rs`
- Host: `apps/distill-desktop/src-tauri/tests/host_fixture_journey.rs`
- Renderer: `apps/distill-desktop/src/App.test.tsx`

The contract matrix and per-scenario evidence registry under
`docs/testing/` are the acceptance record. Historical Electron rows are
explicitly marked `legacy-baseline`; they are provenance, not beta coverage.
