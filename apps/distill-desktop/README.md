# distill-desktop

`distill-desktop` is the native Rust starter for the Distill desktop rebuild.

The current starter is intentionally desktop-first and engine-first:

- native shell built with `Slint` on the `winit` backend
- defaults to a Rust-owned Distill home and schema
- can open an existing Distill Electron home in explicit compatibility mode
- imports Codex and Claude Code captures into the Rust-owned store when you trigger `Reload` in Rust mode
- renders `Sessions`, `DB`, and `Logs` in an Electron-like shell layout
- keeps all writes out of the Electron data directory in compatibility mode

Planning and parity docs for the rebuild live under `docs/`.

## Current Scope

- macOS and Linux are first-class targets
- the shell defaults to a Rust-owned app home under your local app data directory (typically `~/.local/share/distill-desktop` on Linux or `~/Library/Application Support/distill-desktop` on macOS)
- override the Rust app home with `DISTILL_DESKTOP_HOME=/path/to/home`
- switch to Electron compatibility mode with `DISTILL_SOURCE_MODE=electron_compat` (default: Rust-owned mode)
- override the Electron home with `DISTILL_ELECTRON_HOME=/path/to/.distill-electron` (default: `~/.distill-electron`)
- shell preferences are stored separately from the Electron app data
- connector discovery uses `CODEX_HOME` (default: `~/.codex`) and `CLAUDE_HOME` (default: `~/.claude`)

## Layout

- `AGENTS.md`: desktop-local instructions for future work
- `docs/`: parity gap map, rebuild roadmap, and acceptance plan
- `src/app.rs`: bootstrap and path resolution
- `src/connectors/`: canonical source shapes plus the first Codex connector
- `src/controller.rs`: synchronous UI orchestration, callbacks, and preferences
- `src/data/`: read models over either the Rust-owned store or Electron compatibility mode
- `src/storage/`: schema ownership, migrations, raw capture persistence, and import writes
- `src/view_models.rs`: UI-facing state contracts
- `ui/shell.slint`: Electron-like topbar shell and route host
- `ui/sessions_pane.slint`, `ui/logs_pane.slint`, `ui/db_pane.slint`: route panes and stores
- `ui/settings_modal.slint`: read-only settings overlay
- `ui/components.slint`: shared Slint structs and reusable Electron-style components
- `ui/theme.slint`: shared dark shell palette
- `scripts/`: packaging helpers for macOS and Linux

## Commands

Run the desktop shell:

```bash
cargo run -p distill-desktop
```

In Rust-owned mode, `Reload` performs a native sync against `CODEX_HOME` / `~/.codex` and `CLAUDE_HOME` / `~/.claude`, then refreshes the Electron-like shell views.

Run in Electron compatibility mode against a specific Distill Electron home:

```bash
DISTILL_SOURCE_MODE=electron_compat DISTILL_ELECTRON_HOME="$HOME/.distill-electron" cargo run -p distill-desktop
```

Validate the starter:

```bash
cargo check -p distill-desktop
cargo test -p distill-desktop
```

## Packaging

Stage a macOS `.app` bundle:

```bash
apps/distill-desktop/scripts/build-macos.sh
```

Stage a Linux bundle and tarball:

```bash
apps/distill-desktop/scripts/build-linux.sh
```

Both scripts compile the release binary and write staged artifacts under `apps/distill-desktop/dist/`.
