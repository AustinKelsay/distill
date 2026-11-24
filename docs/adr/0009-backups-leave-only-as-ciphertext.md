# Backups are a second way out of DISTILL, and leave only as ciphertext

The raw archive is one copy on one disk, and nothing can rebuild it. A backup copies it to a backup target DISTILL does not control: a folder on another drive, another machine, an object store. That is a second way out besides the read path (ADR 0002), and it carries everything, secrets included. So everything is encrypted on this machine, in an audited format (age), with a backup key the target never sees, before an adapter is handed it. There is no plaintext adapter, not even for a target the owner trusts. Objects are named by a keyed hash under a key derived from the backup key, not by their SHA-256, so a target cannot test whether it holds a known file. It learns only how many objects there are, their sizes and when they arrive.

A backup is not a read. No reader gets session text from it, because only the owner's key opens what the target holds, so it needs no trusted provider (ADR 0003) and writes nothing to the read log. Each run records itself, per target, in the backup log (`log/backups.jsonl`). Restore is the only way a backup comes back in. This squares with ADR 0005, which declined encryption at rest on this machine and already relied on backups being encrypted: off the machine there are no harness folders to match, so the bar there is that a target learns nothing beyond sizes and counts.

## The backup set

Only what cannot be rebuilt: `raw/` (blobs and manifest), `config.toml` (which holds the trust list and the targets), the fingerprint key, the read log, the backup log, the reveal requests with their approvals and denials, and the owner's tags once they exist. Cards and vectors can be rebuilt but cost model time, so a target may include them (`cards = true`, `vectors = true`); restored vectors are used only if the home's embedding model is the one in their header. Everything else stays out: sessions, findings (the model's personal details included), the word index, `state/` and the run log. `Home` lists the set, and a test fails when a path in the home is neither in it nor marked rebuildable.

A run is incremental by content. It puts the blobs the target lacks, then the manifest lines the target lacks as one new object, then each other file of the set whose content changed. Nothing is diffed, and nothing on the target is locked or rewritten.

## Adapters

A target is one `[[backup]]` table in `config.toml` with a `name`, a `kind` and that kind's own settings. Several run at once, each tracked separately. Every kind meets one write-once contract: `list`, `has`, `put` if absent, `get`, and `delete`, used only to prune. A put never overwrites, so a half-finished run is safe to repeat. The `folder` kind is built in. Any other kind is an executable `distill-backup-<kind>` on `PATH` that speaks a line protocol over stdin and stdout, as git remote helpers do, so core holds no cloud SDK and no network code for backups. An adapter sees opaque names and ciphertext, never a key, and knows nothing about DISTILL. It runs as the owner, so installing one is trusting a program on this machine, which ADR 0005 already puts outside what DISTILL defends.

## Keys and restore

The backup key is separate from the fingerprint key, which travels inside the backup set, and lives owner-only in `keys/`. At setup the owner is shown the backup key to keep a copy somewhere else. If both are lost, no one can open the backups, the owner included. A new key starts each target over; objects written under the old key stay readable with it until pruned.

`distill restore` writes only into an empty home. It fetches and decrypts, checks every blob against its capture hash, writes `raw/` through `archive.rs`, and writes every other file of the backup set as it was stored: `config.toml`, the fingerprint key, the logs, the reveal requests and the owner's tags, plus cards and vectors when the target carries them. The tick rebuilds the rest. Merging a backup into a live home is not offered.

## Consequences

- Session text leaves DISTILL two ways: readable through the read path, and as ciphertext in a backup. There is no third.
- Backups and restores are the owner's: targets are set in `config.toml` or Setup, and MCP only reports backup status.
- DISTILL still opens no network listener. An adapter makes its own outbound connections.
- A target learns the archive's size, its growth, and how often it is backed up.
