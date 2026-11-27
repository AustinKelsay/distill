<img src="crates/distill-gui/assets/icon.png" alt="" width="96">

# DISTILL

**D**ata **I**nfrastructure for **S**toring, **T**agging, **I**ndexing & **L**ogging, **L**ocally.

A private, searchable archive of the sessions your local coding agents record.

DISTILL copies every session your agent harnesses keep into one archive you own, in one shape. You, or an agent you trust, can then search it by words or by meaning and read what happened. Secrets are found and replaced with placeholders in what models read. Personal details are found and covered for the owner; a trusted model reads them as written.

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
scripts/install.sh          # installs distill and distill-mcp, and the window: DISTILL.app in ~/Applications on macOS
distill gui                 # or open DISTILL from Spotlight; starts on the Guide, then Setup (harnesses, trusted AI,
                            # models, privacy); Finish installs the timer
```

Without the window: `distill init` writes `~/distill` and a config with every harness found here, and `distill schedule install` installs the timer (launchd on macOS, systemd on Linux). The timer runs `distill tick` every hour by default. The home is `~/distill`, or `DISTILL_HOME`, or `--home`. On Linux the window needs the libraries CI installs (`.github/workflows/ci.yml`); without it, install the CLI and MCP server alone with `cargo install --locked --path crates/distill-cli` and `cargo install --locked --path crates/distill-mcp`.

Models are optional. With none set, DISTILL captures, parses, scans and finds by words. In `config.toml`:

```toml
[steward]
card_model = "…"          # writes a card per session (title, summary, tags, outcome)
embedding_model = "…"     # search by meaning, run in Ollama on this machine
detail_model = "…"        # finds names, addresses, phone numbers and companies; defaults to card_model
sessions_per_run = 25     # sessions the card and detail steps take per tick

[steward.generation]      # optional: run cards and details at a trusted remote provider
provider = "…"            # must be trusted: distill providers add <name> --trusted
base_url = "https://…/v1" # OpenAI-compatible: lists models at /models, takes JSON mode at /chat/completions
api_key_file = "/…/key"   # an absolute path; the first line is the key
```

Tags are shown and filtered in one standard form ("Rust Lang" and "rust_lang" are both `rust-lang`), and aliases you set merge the rest; a card keeps what its model wrote:

```toml
[tags.aliases]
"rust-lang" = "rust"
"js" = "javascript"
```

You can also add a tag to one session or hide one on it, in the window or with `distill tag <session> --add ops --hide misc`; your tags win over the model's and outlive a new card. To filter by several tags at once, repeat `--tag` (`distill search deploy --tag rust --tag cli`), press several chips in the window, or write them with spaces between; a session must have all of them.

With `[steward.generation]` set, `card_model` and `detail_model` name models that provider serves. Setup's Models step writes this section when you run models at a trusted service.

Backups (ADR 0009) go to `[[backup]]` targets, encrypted on this machine first. Run `distill backup init` once to make the key; keep a copy of the key it prints somewhere other than this machine.

```toml
[[backup]]
name = "drive"
kind = "folder"             # built in; the folder must exist (an unplugged drive is refused)
path = "/Volumes/Backup"
cards = false               # also carry cards, which cost model time to rebuild
vectors = false             # also carry vectors, which cost embedding time to rebuild
```

Any other kind runs an adapter, `distill-backup-<kind>` on `PATH` (`docs/backup-adapters.md`). Two are installed with DISTILL: `ssh`, to a folder on another machine you reach over SSH (`host`, `dir`), and `rclone`, to any rclone remote, such as an S3-compatible store (`remote`). Each script's header shows its settings.

Agents on several machines (ADR 0010, `docs/machines.md`): on each other machine, `scripts/install.sh --spoke`, `distill init --spoke <name>` and `distill schedule install` set up a spoke, which only captures; give the hub's SSH key access to it as `docs/machines.md` shows. Add it to the machine that keeps the archive, the hub, under Setup's Machines step or as a `[[machines]]` table (`name`, and the `host` SSH reaches it by), and the hub's tick pulls its new captures over SSH. Its sessions are keyed `<name>:<harness>/<id>`; the window's overview shows each spoke's last pull, and `search` and `tags` take `--machine`.

## Commands

| Command | What it does |
| --- | --- |
| `distill tick` | What the timer runs: sweep, parse, scan, the word index, cards, vectors, personal details. Writes one run log entry. |
| `distill backup init` / `new-key` / `run` / `status` / `verify` | Make the backup key (printing it once), replace it with a new one (every target starts over), back up to every `[[backup]]` target now, show each target's last whole run and the captures it is behind, or check that each holds its newest run (`--full` decrypts every object). After a target's first run, the tick keeps it current. |
| `distill restore --folder PATH` | Bring a folder target's newest run back into an empty home, with the backup key from `--key-file` or a pipe. The next tick rebuilds the rest. For an adapter target: `--kind KIND --set key=value ...`, its settings as in its `[[backup]]` table. |
| `distill status` | Sessions and capture health per harness, the last sweep, each step's progress, pending reveal requests, the exposure check. |
| `distill search <words>` | Sessions that match by words or by meaning, best first, for a model at a named provider. |
| `distill tags` | The tags on the sessions' cards, most used first, with the usual filters, for a model at a named provider. |
| `distill tag <session>` | Add a tag to a session (`--add`) or hide one (`--hide`), as its owner. |
| `distill read <session>` | A session's events for a model at a named provider, with a placeholder for each secret. |
| `distill findings` | The rotation list: each secret by fingerprint, where it appears and when; never a value. |
| `distill reveal` | A model asks for specific secrets; the owner approves in the window; the model collects them once. |
| `distill exposure` | Whether the home is owner-only, outside known synced folders and unlistened; `--fix` repairs file modes. |
| `distill providers` | Providers, which are trusted, and the local runtime's models; `add <name> --trusted`, `trust` and `untrust` manage remote ones. |
| `distill pull [machine]` | On a hub, pull each `[[machines]]` spoke's new captures over SSH now; the tick does it on every run. |
| `distill export --since BYTES` | On a spoke, what the hub runs over SSH: its raw captures from that byte of its manifest on. `--ssh` takes it from a forced command (`docs/machines.md`). |
| `distill reads`, `distill runs` | The read log (every read, refusal and reveal) and the run log. |
| `distill init`, `harnesses`, `sweep`, `parse`, `scan`, `verify`, `schedule`, `gui` | Setup, and each step on its own. |

`distill <command> --help` lists every flag. `search`, `tags` and `read` answer only a reader named with `--provider` (or `DISTILL_PROVIDER`). Session text and secret values leave DISTILL only through the read path: `read`, `search`, `tags`, `reveal collect`, the window, the MCP tools, and the tick's model steps. Two other ways out are not reads, and no reader gets text from them: a backup carries the archive encrypted on this machine (ADR 0009), and a spoke's export carries its raw captures to its hub over SSH (ADR 0010).

## The MCP server

`distill-mcp` speaks MCP over stdio, opens no network port, and reads the home from `DISTILL_HOME` or `~/distill`. Its tools are `search_sessions`, `list_sessions`, `list_tags`, `read_session`, `list_findings`, `request_reveal` and `collect_reveal`.

Every tool answers as a model at the provider named when the harness registers the server (`--provider` or `DISTILL_PROVIDER`, and `--model` or `DISTILL_MODEL` when known). A trusted provider gets text with a placeholder for each secret, between a notice that it is archived data and not instructions; only `collect_reveal` returns values, and only ones the owner approved. Any other provider gets a refusal that says how the owner can trust it. Every read, refusal and reveal is in the read log.

```sh
claude mcp add distill -- distill-mcp --provider <provider>
```

**Name a provider only if every provider that harness can use is trusted.** DISTILL cannot check which provider a reader really runs at.

## Privacy

The archive stores captures without redaction, secrets included: a file as the harness wrote it, a database as a SQLite backup, or one session exported from a database. Findings record where each secret or personal detail is, by kind and fingerprint, never the value. Findings are applied when text is read: the owner sees both covered in the window; a model gets a placeholder for each secret and personal details as written. Detection can miss things, so models at untrusted providers get nothing at all (ADR 0003).

DISTILL must be no more exposed than the harness folders it copies from (ADR 0005). `distill exposure` checks the first three:

- The home, every file in it, and the fingerprint key are readable by the owner alone.
- No part of DISTILL opens a network listener.
- The home is not in a known synced folder (iCloud Drive, Dropbox, Google Drive, OneDrive).
- Detected secret values stay out of the index, cards and vectors, and fingerprints cannot be reversed without the key.

DISTILL makes outbound connections only to what you set:

- **Ollama on this machine:** embeddings, and cards and details unless generation is set.
- **The trusted generation provider,** when `[steward.generation]` is set: cards and details (ADR 0008). It reads sessions with secrets as placeholders, but personal details are not masked for a trusted reader.
- **Your backup targets,** for `[[backup]]` entries that are adapters: ciphertext only, under keyed names (ADR 0009).
- **Your spokes, over SSH,** on a hub with `[[machines]]`: each pull runs the export there and takes its new raw captures (ADR 0010).

On several machines, trust is still one tier, checked on the hub's read path: a spoke's read path serves no reader, and it runs no model. Run `distill exposure` on each spoke too; a spoke has no fingerprint key. Limit the hub's key to the export on the spoke (`docs/machines.md`); its private key stays in the hub's `~/.ssh`, where SSH refuses to use one other accounts can read. Each pull is recorded in the hub's run log, and each export in the spoke's `log/exports.jsonl`. Captures cross the network protected by SSH alone, and the spoke keeps every capture it makes, readable to whoever holds its disk.

What DISTILL does not defend against: anything running as the owner (the archive is not encrypted at rest), someone holding the disk (that is disk encryption's job: FileVault, or dm-crypt on Linux), copies of the home other than DISTILL's own encrypted backups, such as a disk backup (they carry the archive readable; keep them encrypted), and an administrator on the machine.

Claude Code deletes transcripts after 30 days by default. DISTILL sweeps hourly, so it keeps them.

## Building

```sh
cargo test && cargo test -p distill-gui && cargo test -p distill-eval
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all --check
```

Try things against a temporary home (`--home` or `DISTILL_HOME`). `scripts/install.sh` replaces the binaries your real archive's timer runs (and DISTILL.app on macOS); `distill schedule install` and Finish in the window's Setup replace the timer job itself, whatever home you pass.
