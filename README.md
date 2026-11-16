# Distill

Distill keeps every session your local agent harnesses record in one private archive you own, in one consistent shape, so that you or an agent can find the right sessions later. Distilling, training and analysis can then start from a search.

**Distill is being rebuilt.** The spec is [issue #72](https://github.com/AustinKelsay/distill/issues/72), its terms are defined in [`GLOSSARY.md`](GLOSSARY.md), and its decisions are recorded in [`docs/adr/`](docs/adr/). What is built so far is in [`docs/status.md`](docs/status.md). The previous build is tagged `agent-driven-final`; the one before it, `legacy-final`.

```
harnesses ──sweep──▶ raw archive ──parse──▶ events and session facts
(Claude Code,        append-only,           rebuildable from the
 Codex, OpenCode,    hash-verified,         raw archive at any time
 Droid, Pi,          no model
 Cursor, Grok,
 Antigravity,
 Hermes, T3 Code,
 Goose)
```

## What works today

Capture and the parsers, which the rebuild keeps (ADR 0001):

| Command | What it does |
| --- | --- |
| `distill init` | Creates the home (`~/distill`, or `DISTILL_HOME` / `--home`) and a config with every harness found on this machine enabled. |
| `distill harnesses` | Shows each harness and what of it is on this machine. |
| `distill sweep` | Copies new and changed harness files into `raw/`. A file that only grew replaces its old capture; a rewritten file keeps both. Every file under a harness's roots is kept. SQLite files are read through SQLite's backup API; OpenCode sessions, T3 Code threads and Goose and Hermes sessions are exported one per session. Excluded projects are never copied. |
| `distill verify` | Checks every capture in the raw archive against its hash. |
| `distill parse` | Turns captures into `sessions/<harness>/<id>/events.jsonl` (prompts, replies, reasoning, tool calls and results) and `meta.json` (session facts, which hold no session text), and rebuilds the session list. Reports format drift. |
| `distill tick` | What the timer runs: sweep, then parse. Writes one entry to the run log. |
| `distill status` | Sessions, capture health and format drift per harness, and the last sweep. |
| `distill runs` | The run log. |
| `distill providers` | Lists each provider, local or remote, and whether it is trusted. `ollama`, the runtime on this machine, is always trusted; its models are shown as stored here or served from a cloud. `providers add <name> [--trusted]`, `trust <name>` and `untrust <name>` manage remote providers. |
| `distill schedule install` | Runs `distill tick` on a timer (launchd on macOS, systemd on Linux). |
| `distill gui` / `distill-mcp` | The window and the stdio MCP server. Both are shells that expose nothing until their tickets land. |

No command prints session text.

## Building

```sh
cargo build --release
cargo test && cargo test -p distill-gui
```

Use a temporary home (`--home` or `DISTILL_HOME`) for anything you try out. `scripts/install.sh` installs `distill`, `distill-mcp` and `distill-gui` into Cargo's bin directory. The timer no longer needs Full Disk Access on macOS: it reads only the harnesses' own folders (see `docs/status.md`).

## Privacy

Distill has no network code and opens no network listener. The archive keeps every session exactly as the harness wrote it, secrets included, with owner-only permissions. Claude Code deletes transcripts older than 30 days by default; Distill sweeps hourly, so it keeps them.
