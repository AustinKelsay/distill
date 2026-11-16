# Distill agent guide

- Distill is being rebuilt as a private, searchable session archive. The spec is issue #72, with tickets #73 to #91; terms are in `GLOSSARY.md` and decisions in `docs/adr/`. Read them before changing behavior. Built vs. planned: `docs/status.md`; update it when a ticket lands.
- Workspace: `crates/distill-core` (all behavior), `distill-cli` (`distill` binary), `distill-mcp` (stdio MCP server), `distill-gui` (Rune window; the only crate that depends on Rune). The faces hold no logic. The CLI has one module per command (`distill-cli/src/commands/`) and the MCP server one per tool (`distill-mcp/src/tools/`), so tickets built in parallel don't edit the same file.
- What core keeps (ADR 0001): capture (`sweep.rs`, `archive.rs`, `harness.rs`, `schedule.rs`) and the parsers (`normalize/`). The tick runs the steps declared in `tick::STEPS`. Status is one report in `status.rs`. `Home` has a path helper for every entry of the spec's on-disk layout; use it rather than joining paths. Code parked for later tickets lives in `src/parked/` with no callers.
- The owner's live archive is `~/distill`, and a timer runs the installed binary against it. Run every build, test and GUI session against a temporary home. Installing (`scripts/install.sh`, `cargo install`) waits for #91, which the owner does. A test that reaches the timer installer stubs `launchctl` (and `systemctl` on Linux), because installing replaces the live `dev.distill.tick` job whatever `HOME` is.
- Commands: `cargo test` (default members, no GUI), `cargo test -p distill-gui`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo fmt --all --check`.
- The raw archive (`raw/`) is append-only and its on-disk format never changes. Only `archive.rs` writes it. Everything under `sessions/`, `index/` and `state/` must stay rebuildable from `raw/`.
- Parsers live in `crates/distill-core/src/normalize/<harness>.rs`. When a parser's output changes, bump its `parser_version` in `normalize/mod.rs` and add a test with a small synthetic sample. When what the parse step derives beyond the parsers changes, bump `META_VERSION`. Never commit real session text, paths, or secrets as fixtures.
- Rune is pinned to one `dev` rev in the root `Cargo.toml` (dependency and `[patch.crates-io]`); bump them together.

## Agent skills

### Issue tracker

Issues and specs live in this repo's GitHub Issues (`gh` CLI). See `docs/agents/issue-tracker.md`.

### Triage labels

The five default labels: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single context: `GLOSSARY.md` at the root, decisions in `docs/adr/`. See `docs/agents/domain.md`.
