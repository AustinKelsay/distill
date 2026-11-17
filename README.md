# Distill

Distill keeps every session your local agent harnesses record in one private archive you own, in one consistent shape, so that you or an agent can find the right sessions later. Distilling, training and analysis can then start from a search.

**Distill was rebuilt** from the spec in [issue #72](https://github.com/AustinKelsay/distill/issues/72); its terms are defined in [`GLOSSARY.md`](GLOSSARY.md), and its decisions are recorded in [`docs/adr/`](docs/adr/). What is built, and its known gaps, are in [`docs/status.md`](docs/status.md). The previous build is tagged `agent-driven-final`; the one before it, `legacy-final`.

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

Capture and the parsers, kept from the previous build (ADR 0001), and everything built on them:

| Command | What it does |
| --- | --- |
| `distill init` | Creates the home (`~/distill`, or `DISTILL_HOME` / `--home`) and a config with every harness found on this machine enabled. |
| `distill harnesses` | Shows each harness and what of it is on this machine. |
| `distill sweep` | Copies new and changed harness files into `raw/`. A file that only grew replaces its old capture; a rewritten file keeps both. Every file under a harness's roots is kept. SQLite files are read through SQLite's backup API; OpenCode sessions, T3 Code threads and Goose and Hermes sessions are exported one per session. Excluded projects are never copied. |
| `distill verify` | Checks every capture in the raw archive against its hash. |
| `distill parse` | Turns captures into `sessions/<harness>/<id>/events.jsonl` (prompts, replies, reasoning, tool calls and results) and `meta.json` (session facts, which hold no session text), and rebuilds the session list. Reports format drift. |
| `distill scan` | Scans every session whose findings are missing or out of date for secrets, and stores `findings.json` beside it: each secret's place, kind and fingerprint (a keyed hash), never its value. Stored text is never changed. Prints counts only. |
| `distill findings [<fingerprint>]` | The rotation list: each secret by fingerprint (its short form), with its kind, how many sessions it appears in, their harnesses, and when it was first and last seen. `--kind personal`, `all` or one kind (`github_token`) selects other kinds; `--harness`, `--since`, `--until` filter; `--sort spread` (the default) or `recent`. Given a fingerprint or its short form, every session it appears in, for every fingerprint that starts with it. Answers from stored findings, never a value. In the read log. |
| `distill tick` | What the timer runs: sweep, parse, scan, the word index, cards, vectors for search by meaning, then personal details found by the local model. The model steps start no session after half the timer's interval and leave what they don't reach for the next tick, so a slow model holds the tick past that by at most one model call (each capped at 10 minutes). Capture (a full rescan, say) and each step's own reading and disk work have no time limit, and a five-minute interval is shorter than one call, so the next sweep can still start late. Writes one entry to the run log. |
| `distill status` | Sessions, capture health and format drift per harness, the last sweep, the progress of the scan, word index, cards, vectors and personal details (and why a model step is skipped), pending reveal requests, and the exposure check. |
| `distill exposure` | The exposure check: whether the home and everything in it, and the fingerprint key, are owner-only, whether the home is in a synced folder, and whether a Distill process listens on a network port, each as pass or fail. Also whether the disk is encrypted, a reminder that backups carry the archive, and the previous build's folders that hold session-derived data, with their modes. Fails when a rule fails. `--fix` takes the group's and others' permissions off an existing home. `distill status` carries the same results. |
| `distill runs` | The run log. |
| `distill providers` | Lists each provider, local or remote, and whether it is trusted. `ollama`, the runtime on this machine, is always trusted; its models are shown as stored here or served from a cloud. `providers add <name> [--trusted]`, `trust <name>` and `untrust <name>` manage remote providers. |
| `distill read <session>` | A session's events as text, for a model at a named provider (`--provider` or `DISTILL_PROVIDER`, `--model` or `DISTILL_MODEL`), in full or `--from`/`--to` an event number, between a fixed notice that it is archived data and not instructions. A model at a trusted provider reads `[secret: <kind> #<fingerprint>]` in place of each secret; a session is scanned first if its findings are missing or out of date. Every other reader is refused, with what the owner can do. |
| `distill reads` | The read log, newest first: every call to the read path, refusals included, with time, reader, session and result; a call that lists sessions is one line with how many it listed. `--reader` narrows it to `owner`, `distill`, a provider or `provider/model`. |
| `distill schedule install` | Runs `distill tick` on a timer (launchd on macOS, systemd on Linux). |
| `distill search <words>...` | Sessions that have every word of the query, or are close to it in meaning, best first in one list, for a model at a named provider (`--provider`, `--model`, as for `distill read`): each one's facts, card (or opening) and matching passage. `--harness`, `--project`, `--used-model`, `--since`, `--until`, `--tag` and `--outcome` narrow it; `--mirrors` and `--steward` include mirrors and the previous steward's sessions, which are left out by default; `--offset` and `--limit` (up to 50) page it. Without an `embedding_model` or a local runtime it finds by words alone and says so. |
| `distill reveal request <session> <fingerprint>... --reason <why>` | A model at a trusted provider asks the owner for the values of specific secrets; prints a request ID, never a value. `distill reveal collect <id>` gives the values once the owner has approved, once, to the model that asked. Requests expire after 24 hours. Approving and denying happen only in the GUI. |
| `distill gui` | The window: Setup, Overview, Sessions (search and filters, an open session with each finding covered until clicked), Findings (the rotation list), Approvals (reveal requests to approve or deny) and the Read log. |
| `distill-mcp` | The stdio MCP server, for an agent in a harness: `list_sessions` (filtered by harness, project, model and date, 50 rows a call), `read_session` (in full or by event range), `search_sessions` (as `distill search`), `list_findings` (the rotation list, or where one fingerprint appears, never a value), and `request_reveal` and `collect_reveal` (as `distill reveal`). See [The MCP server](#the-mcp-server). |

Only `distill read`, `distill search`, the GUI and the MCP server's tools return session text, and only through the read path, the one function in core that returns it.

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

The archive keeps every session exactly as the harness wrote it, secrets included; findings mark where the secrets are without copying them. Claude Code deletes transcripts older than 30 days by default; Distill sweeps hourly, so it keeps them.

### The security bar

Distill must be no more exposed than the harness folders it copies from (ADR 0005). It meets that bar this way, and `distill exposure` checks the first three:

- The home and every file and folder in it are readable by the owner alone, including folders created on the way to a file, and so is the fingerprint key.
- No part of Distill opens a network listener. Its only connections go to the local runtime on this machine.
- The home should not sit in a synced folder (iCloud Drive, `~/Library/CloudStorage`, Dropbox, Google Drive, OneDrive), where the archive would be uploaded.
- Secret values stay out of everything but the raw archive and the sessions' events, fingerprints cannot be reversed without the key, and models read placeholders.

What Distill does not defend against:

- Anything running as the owner. A process with the owner's permissions can read the archive directly, as it can read the harness folders. The archive is not encrypted at rest.
- Someone holding the disk. That is the disk encryption's job (FileVault on macOS).
- Backups. They carry the archive, every secret in it included: keep them encrypted.
- An administrator, or root, on the machine.
