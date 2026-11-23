# Known gaps

What DISTILL does not do, or does only partly. Update this page when a change adds or removes a limit.

## Capture

- With any privacy exclusion set, a harness whose sessions record no working directory is not archived at all: Antigravity, Hermes's session files, and Cursor (its stores and transcripts). Neither is a T3 Code thread whose worktree and project root are unknown, nor new native references for it.
- Cursor's IDE chat store (`state.vscdb`) is not read: in the chats checked for #108, each chat with messages also had an agent transcript, which DISTILL archives, but the transcripts keep no tool results or reasoning; only the store has them (`docs/cursor-ide-chats.md`). A chat kept only in the store is missed. Cursor's SDK agent stores (`projects/*/sdk-agent-store`, how T3 Code runs Cursor) encrypt their blobs, so they are archived but not parsed; their agents are covered by Cursor's transcripts and the T3 threads that ran them.
- OpenCode's spilled tool outputs are looked for only in `tool-output/` beside the database DISTILL reads. With `OPENCODE_DB` set elsewhere they are not captured.
- Format drift inside a record type a parser knows (a renamed field) is not counted; only a capture that then parses to nothing shows it.
- A config that lists every harness DISTILL knew when it was written gets new harnesses switched on; one that lists only some gets nothing added.
- Each new capture costs two directory fsyncs (about 18 ms), so a first sweep of a large machine takes a few minutes.

## Findings

- Names, postal addresses, phone numbers and companies are found only by the model pass, only in what the owner and the agent wrote, and only once it has run. A detail cut across two parts of a long session can be missed. Personal details are not masked for a trusted reader.
- Findings go out of date when the fingerprint key changes: a lost key means every session is scanned again, and old fingerprints match nothing new.
- A model reply that is not the expected JSON fails under the strict parse; that session is retried after the rest.

## Search

- Words longer than 64 characters are not indexed, and there is no phrase or prefix search.
- Words inside encoded data are not indexed: a run of at least 100 bytes of base64's characters (letters, digits, `+`, `/`, `=`, `_`, `-`) with a digit in it (#154). On the owner's archive that was half of the tool output and 97% of the distinct words. A token that long without a `.`, a space or another separator, such as a long hex string, is not found either.
- Search by meaning embeds only what the owner wrote and the card summary. Mirrors and sessions that worked inside the home (the steward mark) get no vectors, so `--mirrors` and `--steward` find them by words only. No prefix such as `search_query:` is added for embedding models that expect one. A search by meaning reads the whole vectors file and embeds the query once in Ollama, unless the detector finds a secret in it or a piece of it (split at spaces, `=`, `:`, `,`, `;`, quotes and round brackets) 8 or more bytes long has a short fingerprint already in the word index. A secret both checks miss is embedded as written: a shorter piece, one joined to other text by any other character (`.`, `?`, `/`, `-`, square brackets…), or one not yet indexed. So are personal details.
- A card written by a tick can be embedded in the same tick (within the vectors step's own limit) but joins the word index at the next one, because the index step runs before the card step.
- Measured on the owner's archive (#117): asked the way the owner would say it, 16 of 24 sessions the owner started were in the top five. That set is the yardstick for any search change.

## Safety checks

- The exposure check sees listeners only through `lsof`, and only DISTILL's own processes running at the time; without `lsof` that rule is unchecked. On Linux the disk counts as encrypted only on a dm-crypt device. The timer's unit files sit outside the home with the usual modes; they hold paths, not session data.
- The pruning check reads the owner's own harness settings and Claude Code's managed settings, not project-level settings, MDM profiles or server-managed settings. Antigravity's trimming by size and in-place rewrites have no setting to read (`docs/harness-pruning.md`).
- Whether a model is stored on this machine is read from `OLLAMA_MODELS`, else `~/.ollama/models`; an Ollama server started with another model folder cannot be checked. With `[steward.generation]` set, a card or detail model counts as there when the provider's `/models` lists it.

## Backups

- Backups so far: `distill backup init` makes the key and prints its recovery key once; `distill backup run` copies to each `[[backup]]` target. Only the `folder` kind exists. `distill backup verify` checks that a target holds its newest run, by name or, with `--full`, by fetching and decrypting every object. The window's Setup has a Backups step that adds folder targets (by hand in config.toml otherwise; the CLI has no `backup add`), tests each, and makes the backup key or shows its recovery key again; the Overview has a Backups section with Back up now and Verify (by name only; `--full` is CLI only). A test writes one small file at the target and removes it. `distill backup status` and the status report say where each target stands.
- The tick backs up only to targets that have had a whole run by hand (`distill backup run`), so it never starts a target over or makes its folder. It has no time limit of its own: it sends what changed since the last run, which is usually little. A target that isn't there, such as an unplugged drive, fails the tick's backup step every tick until it's back.
- MCP doesn't report backups: they are the owner's, and a client sees only sessions.
- The backup key can't be rotated yet (#178): `distill backup init` makes it once, and a new key would start every target over.
- Every file of the backup set but `raw/` and the two logs is stored whole whenever it changes. The read log, the backup log and the manifest are stored as what each run appended, in parts that merge as a binary counter carries: a record lists at most log2(runs) + 1 parts of each, and each appended byte is stored again at most that many times. A log that no longer starts with what was stored is stored again from its start.
- A folder target's first run can't tell an empty folder from a drive's mount point with the drive away (on Linux; on macOS the mount point goes with the drive), so it takes the folder; after its first successful run, a target without its `distill-backup/` folder is refused.
- A target learns how many objects it holds, their sizes and when they arrive (ADR 0009). Nothing on a target is ever deleted, so it only grows.
- A blob lost from this machine's archive before any backup took it can't be sent: the run stores the rest, then fails, counting it as missing.
- `distill restore` brings back only the newest run, only from a folder target, and only into an empty home whose parent folder exists; it never merges into a live one. It brings back the backup set alone: the next tick rebuilds sessions, findings and the index, and makes cards (unless the target carries them) and personal details again at model cost. Harness files DISTILL never captured, and anything after the newest run, are not in a backup. The restored backup log lacks the newest run's own line.
- A restore stops when the target lacks a blob the manifest names, unless `--allow-missing` is given. It takes the recovery key from `--key-file` or a pipe, never typed at the terminal, where it would show on screen.

## The window

- Findings are covered with a chip naming their kind rather than blurred (ADR 0007). The list's chips are not clickable; a finding is shown only in an open session. A covered secret copies as its chip, a shown one as its value.
- The session view has no search inside a session and no jump to the passage a search matched; it reads the whole session at once. A session opened from Findings or Approvals is headed by its key, not its card.
- Approvals shows only pending requests; answered, collected and expired ones are in the read log. A request's expiry is recorded when the next call touches requests.
- Setup cannot move the home, and offers `sessions_per_run` as 25, 50, 100 or 200 (another value set by hand is kept until one is picked). It offers the models stored on this machine, or, when cards and detection run at a trusted service (`[steward.generation]`), the models that service lists when checked (plus the one already set). It takes the service's key as a file path, not the key itself.
- A previous build's `recipes/`, `exports/` and `steward/` folders in the home are reported by the exposure check and otherwise ignored, never deleted.
