# Distill Implementation Map

This file is informative. It describes the current TypeScript/Electron implementation and how it maps to the canonical specs under `docs/`.

Start with the canonical architecture here:

- [docs/specs/architecture.md](docs/specs/architecture.md)

## Rebuild Library (informative)

The clean-rebuild Rust Library lives beside the Electron baseline:

- workspace: root `Cargo.toml`
- Library crate: `crates/distill-library`
- CLI crate: `crates/distill-cli` (`distill` binary)
- Desktop app: `apps/distill-desktop` (Tauri 2 host + React/Vite renderer)
- Library contract tests: `crates/distill-library/tests/library_fixture_tracer.rs`
- CLI contract tests: `crates/distill-cli/tests/cli_fixture_journey.rs`
- Host contract tests: `apps/distill-desktop/src-tauri/tests/host_fixture_journey.rs`
- Renderer contract tests: `apps/distill-desktop/src/App.test.tsx`

Useful commands (Rust toolchain and npm workspaces):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
npm run desktop:typecheck
npm run desktop:lint
npm run desktop:format
npm run desktop:test
npm run desktop:frontend:build
```

CLI Fixture journey:

```bash
cargo run -p distill-cli -- --home /tmp/distill-home --fixture /path/to/fixture --format json
```

Documented CLI exit codes: `0` success, `1` Library/runtime failure, `2` usage/validation.

Canonical rebuild behavior remains under `docs/specs/` and the gap register.

## Current Runtime

- desktop shell: Electron
- application code: TypeScript
- local data layer: SQLite

## Current Module Map

### Connectors

- `src/connectors/index.ts`
- `src/connectors/codex/*`
- `src/connectors/claude_code/*`
- `src/connectors/opencode/*`

These modules implement the current source-specific logic for detect, discover, snapshot, and parse behavior.

Canonical reference:

- [docs/specs/connectors.md](docs/specs/connectors.md)

### Import And Storage

- `src/distill/import.ts`
- `src/distill/db.ts`
- `schema.sql`

These modules implement the current importer, database helpers, and SQLite schema.

Canonical references:

- [docs/specs/data-model.md](docs/specs/data-model.md)
- [docs/specs/ingest-pipeline.md](docs/specs/ingest-pipeline.md)

### Query, Curation, And Export

- `src/distill/query.ts`
- `src/distill/curation.ts`
- `src/distill/export.ts`

Canonical reference:

- [docs/specs/search-curation-export.md](docs/specs/search-curation-export.md)

### Operations

- `src/distill/jobs.ts`
- `src/distill/logs.ts`
- `src/electron/main.ts`
- `src/electron/preload.ts`
- `src/renderer/app.ts`

Canonical reference:

- [docs/specs/activity-and-ops.md](docs/specs/activity-and-ops.md)

### Tests

- `src/test/*.test.ts`

Current tests primarily validate current implementation behavior. The canonical contract-test plan lives in [docs/testing/contract-test-matrix.md](docs/testing/contract-test-matrix.md).

## Current Behavior Notes

The current codebase already implements the core local-first MVP loop defined in the canonical specs.

Current implementation highlights:

- recoverable raw capture storage for file-backed and virtual captures
- inline raw payloads of `64 KiB` or less are stored directly in the capture record, and larger payloads are stored in the Distill blob area
- append-only capture history plus replace-on-success session projection writes
- canonical activity auditing for capture, projection, curation, export, and sync lifecycle
- direct artifact-to-message linkage alongside capture-record provenance

See [docs/gaps/current-state-gap-register.md](docs/gaps/current-state-gap-register.md) for historical resolution notes and any newly tracked drift.

## How This File Should Be Used

Use this file when you need to find the current code quickly.

Do not use this file as the authority for target behavior. The canonical behavior specs live under `docs/specs/`.
