# Distill agent guide

- Workspace: `crates/distill-core` (all behavior), `distill-cli` (`distill` binary), `distill-mcp` (stdio MCP server), `distill-gui` (Rune window; the only crate that depends on Rune). The GUI and CLI hold no logic of their own.
- Design: `docs/plan.md`. Built vs. planned: `docs/status.md`. Update `docs/status.md` when a milestone moves.
- Commands: `cargo test` (default members, no GUI), `cargo test -p distill-gui`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`.
- The raw archive (`raw/`) is append-only. Only `archive.rs` writes it. Everything under `sessions/`, `exports/` and `state/` must stay rebuildable from `raw/`.
- Parsers live in `crates/distill-core/src/normalize/<source>.rs`. When a parser's output changes, bump its `parser_version` in `normalize/mod.rs` and add a test with a small synthetic sample. Never commit real session text, paths, or secrets as fixtures.
- Rune is pinned to one `dev` rev in the root `Cargo.toml` (dependency and `[patch.crates-io]`); bump them together.
