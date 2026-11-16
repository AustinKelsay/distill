# Distill agent guide

- Workspace: `crates/distill-core` (all behavior), `distill-cli` (`distill` binary), `distill-mcp` (stdio MCP server), `distill-gui` (Rune window; the only crate that depends on Rune). The GUI and CLI hold no logic of their own.
- Rebuild in progress (2026-11-15): the spec is issue #72 with tickets #73 to #91; terms are in `GLOSSARY.md`, decisions in `docs/adr/`. Where they disagree with `docs/plan.md`, they win.
- The owner's live archive is `~/distill`, and a timer runs the installed binary against it. Run every build, test and GUI session against a temporary home. Installing (`scripts/install.sh`, `cargo install`) waits for #91, which the owner does. A test that reaches the timer installer stubs `launchctl` (and `systemctl` on Linux), because installing replaces the live `dev.distill.tick` job whatever `HOME` is.
- Design: `docs/plan.md`. Built vs. planned: `docs/status.md`. Update `docs/status.md` when a milestone moves.
- Commands: `cargo test` (default members, no GUI), `cargo test -p distill-gui`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`.
- The raw archive (`raw/`) is append-only. Only `archive.rs` writes it. Everything under `sessions/`, `exports/` and `state/` must stay rebuildable from `raw/`.
- Parsers live in `crates/distill-core/src/normalize/<source>.rs`. When a parser's output changes, bump its `parser_version` in `normalize/mod.rs` and add a test with a small synthetic sample. Never commit real session text, paths, or secrets as fixtures.
- Rune is pinned to one `dev` rev in the root `Cargo.toml` (dependency and `[patch.crates-io]`); bump them together.

## Agent skills

### Issue tracker

Issues and specs live in this repo's GitHub Issues (`gh` CLI). See `docs/agents/issue-tracker.md`.

### Triage labels

The five default labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single context: `GLOSSARY.md` at the root, decisions in `docs/adr/`. See `docs/agents/domain.md`.
