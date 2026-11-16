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
| `distill scan` | Scans every session whose findings are missing or out of date for secrets, and stores `findings.json` beside it: each secret's place, kind and fingerprint (a keyed hash), never its value. Stored text is never changed. Prints counts only. |
| `distill tick` | What the timer runs: sweep, parse, then scan. Writes one entry to the run log. |
| `distill status` | Sessions, capture health and format drift per harness, the last sweep, and how many sessions are scanned. |
| `distill runs` | The run log. |
| `distill providers` | Lists each provider, local or remote, and whether it is trusted. `ollama`, the runtime on this machine, is always trusted; its models are shown as stored here or served from a cloud. `providers add <name> [--trusted]`, `trust <name>` and `untrust <name>` manage remote providers. |
| `distill read <session>` | A session's events as text, for a model at a named provider (`--provider` or `DISTILL_PROVIDER`, `--model` or `DISTILL_MODEL`), in full or `--from`/`--to` an event number, between a fixed notice that it is archived data and not instructions. A model at a trusted provider reads `[secret: <kind> #<fingerprint>]` in place of each secret; a session is scanned first if its findings are missing or out of date. Every other reader is refused, with what the owner can do. |
| `distill reads` | The read log, newest first: every call to the read path, refusals included, with time, reader, session and result; a call that lists sessions is one line with how many it listed. `--reader` narrows it to `owner`, `distill`, a provider or `provider/model`. |
| `distill schedule install` | Runs `distill tick` on a timer (launchd on macOS, systemd on Linux). |
| `distill gui` | The window. A shell that shows nothing until its ticket lands. |
| `distill-mcp` | The stdio MCP server, for an agent in a harness: `list_sessions` (filtered by harness, project, model and date, 50 rows a call) and `read_session` (in full or by event range). See [The MCP server](#the-mcp-server). |

Only `distill read` and the MCP server's tools return session text, and only through the read path, the one function in core that returns it.

## The MCP server

`distill-mcp` speaks MCP over stdio and opens no network listener. Every tool answers as one reader: a model at the provider named where the harness registers the server, with `--provider` (or `DISTILL_PROVIDER`) and, when known, the model with `--model` (or `DISTILL_MODEL`). It gets exactly what the read path gives that provider: text between a notice that it is archived data and not instructions, every call in the read log (a list call as one line with a count), and, for a provider that is unnamed or not trusted, a refusal that says how the owner can trust it, with no session text, facts or counts.

**Name a provider only if every provider used in that harness is trusted.** Distill cannot check which provider a reader really runs at, so a harness that can switch between providers is registered under one only when all of them are trusted.

A harness whose model runs in Ollama on this machine, such as Goose or OpenCode set up for a local model:

```json
{
  "mcpServers": {
    "distill": {
      "command": "distill-mcp",
      "args": ["--provider", "ollama", "--model", "qwen3:8b"]
    }
  }
}
```

A harness whose model runs at a remote provider, once the owner has trusted that provider (`distill providers add anthropic --trusted`), here Claude Code:

```sh
claude mcp add distill -- distill-mcp --provider anthropic
```

The same with the environment instead of flags:

```json
{
  "mcpServers": {
    "distill": {
      "command": "distill-mcp",
      "env": { "DISTILL_PROVIDER": "anthropic" }
    }
  }
}
```

The server reads the home from `DISTILL_HOME`, else `~/distill`. A model at a trusted provider, local or remote, is served openings and text with a placeholder in place of each secret.

## Building

```sh
cargo build --release
cargo test && cargo test -p distill-gui
```

Use a temporary home (`--home` or `DISTILL_HOME`) for anything you try out. `scripts/install.sh` installs `distill`, `distill-mcp` and `distill-gui` into Cargo's bin directory. The timer no longer needs Full Disk Access on macOS: it reads only the harnesses' own folders (see `docs/status.md`).

## Privacy

Distill has no network code and opens no network listener. The archive keeps every session exactly as the harness wrote it, secrets included, with owner-only permissions; findings mark where the secrets are without copying them. Claude Code deletes transcripts older than 30 days by default; Distill sweeps hourly, so it keeps them.
