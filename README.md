# Distill

A private, searchable archive of the sessions your local coding agents record.

Distill copies every session your agent harnesses keep into one archive you own, in one shape. You, or an agent you trust, can then search it by words or by meaning and read what happened. Secrets and personal details are found and kept out of what models read.

```
harnesses ──sweep──▶ raw archive ──parse──▶ events and session facts ──▶ findings, word index,
(Claude Code,        append-only,           rebuildable from the        cards, vectors
 Codex, OpenCode,    hash-verified          raw archive at any time
 Droid, Pi,
 Cursor, Grok,
 Antigravity,
 Hermes, T3 Code,
 Goose)
```

Terms are defined in [`GLOSSARY.md`](GLOSSARY.md), decisions in [`docs/adr/`](docs/adr/), and known limits in [`docs/known-gaps.md`](docs/known-gaps.md).

## Set up

```sh
scripts/install.sh          # builds and installs distill, distill-mcp and distill-gui
distill init                # creates ~/distill and a config with every harness found here
distill gui                 # Setup: harnesses, providers, models, privacy; Finish installs the timer
```

`distill schedule install` installs the timer without the window (launchd on macOS, systemd on Linux). It runs `distill tick` every hour by default. The home is `~/distill`, or `DISTILL_HOME`, or `--home`.

Models are optional. With none set, Distill captures, parses, scans and finds by words. In `config.toml`:

```toml
[steward]
card_model = "…"          # writes a card per session (title, summary, tags, outcome)
embedding_model = "…"     # search by meaning, run in Ollama on this machine
detail_model = "…"        # finds names, addresses, phone numbers and companies; defaults to card_model
sessions_per_run = 25     # sessions each model step takes per tick

[steward.generation]      # optional: run cards and details at a trusted remote provider
provider = "…"            # must be a provider marked trusted
base_url = "https://…/v1" # an OpenAI-compatible API
api_key_file = "…"        # first line is the key
```

## Commands

| Command | What it does |
| --- | --- |
| `distill tick` | What the timer runs: sweep, parse, scan, the word index, cards, vectors, personal details. Writes one run log entry. |
| `distill status` | Sessions and capture health per harness, the last sweep, each step's progress, pending reveal requests, the exposure check. |
| `distill search <words>` | Sessions that match by words or by meaning, best first, for a model at a named provider. |
| `distill read <session>` | A session's events for a model at a named provider, with a placeholder for each secret. |
| `distill findings` | The rotation list: each secret by fingerprint, where it appears and when; never a value. |
| `distill reveal` | A model asks for specific secrets; the owner approves in the window; the model collects them once. |
| `distill exposure` | Whether the home is owner-only, unsynced and unlistened; `--fix` repairs file modes. |
| `distill providers` | Providers, which are trusted, and the local runtime's models. |
| `distill reads`, `distill runs` | The read log (every read, refusal and reveal) and the run log. |
| `distill init`, `harnesses`, `sweep`, `parse`, `scan`, `verify`, `schedule`, `gui` | Setup, and each step on its own. |

`distill <command> --help` lists every flag. Session text and secret values leave Distill only through the read path: `read`, `search`, `reveal collect`, the window and the MCP tools.

## The MCP server

`distill-mcp` speaks MCP over stdio, opens no network port, and reads the home from `DISTILL_HOME` or `~/distill`. Its tools are `search_sessions`, `list_sessions`, `read_session`, `list_findings`, `request_reveal` and `collect_reveal`.

Every tool answers as a model at the provider named when the harness registers the server (`--provider` or `DISTILL_PROVIDER`, and `--model` or `DISTILL_MODEL` when known). A trusted provider gets text with a placeholder for each secret, between a notice that it is archived data and not instructions. Any other provider gets a refusal that says how the owner can trust it. Every call is in the read log.

```sh
claude mcp add distill -- distill-mcp --provider <provider>
```

**Name a provider only if every provider that harness can use is trusted.** Distill cannot check which provider a reader really runs at.

## Privacy

The archive keeps every session exactly as the harness wrote it, secrets included. Findings record where each secret or personal detail is, by kind and fingerprint, never the value. Text is masked when it is read: covered chips for the owner in the window, placeholders for models. Detection can miss things, so models at untrusted providers get nothing at all (ADR 0003).

Distill must be no more exposed than the harness folders it copies from (ADR 0005). `distill exposure` checks the first three:

- The home, every file in it, and the fingerprint key are readable by the owner alone.
- No part of Distill opens a network listener.
- The home is not in a synced folder (iCloud Drive, Dropbox, Google Drive, OneDrive).
- Secret values stay out of the index, cards and vectors, and fingerprints cannot be reversed without the key.

Distill makes outbound connections only to the providers you set:

- **Ollama on this machine:** embeddings, and cards and details unless generation is set.
- **The trusted generation provider,** when `[steward.generation]` is set: cards and details (ADR 0008). It reads sessions with secrets as placeholders, but personal details are not masked for a trusted reader.

What Distill does not defend against: anything running as the owner (the archive is not encrypted at rest), someone holding the disk (that is FileVault's job), backups (they carry the archive; keep them encrypted), and an administrator on the machine.

Claude Code deletes transcripts after 30 days by default. Distill sweeps hourly, so it keeps them.

## Building

```sh
cargo test && cargo test -p distill-gui && cargo test -p distill-eval
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
```

Try things against a temporary home (`--home` or `DISTILL_HOME`). `scripts/install.sh`, and Finish in the window's Setup, replace the timer job your real archive runs on.
