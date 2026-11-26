# Backup adapters

A backup target of any kind but `folder` is reached through an adapter: an executable named
`distill-backup-<kind>` on `PATH` (ADR 0009, issue #177). DISTILL runs it once for each use of
a target, as the owner, with the timer's `PATH` when the tick runs it. They talk in lines, as
git remote helpers do, so DISTILL holds no cloud SDK and no network code for backups.

An adapter moves opaque objects and knows nothing about DISTILL. Each object's name is 64
lowercase hex characters (a keyed hash), and its bytes are ciphertext. An adapter never sees a
key.

## The target

A target is a `[[backup]]` table in `config.toml`. `name` and `kind` are DISTILL's. Every other
key is the adapter's own setting:

```toml
[[backup]]
name = "offsite"
kind = "s3"              # runs distill-backup-s3
bucket = "my-backups"    # the adapter's own settings
```

## The protocol

DISTILL writes one command per line to the adapter's standard input. For each command, the
adapter writes one reply line to its standard output, and for `list` the names after it. Any
reply may be `error <message>` instead. DISTILL then stops using the target and shows the
message. When DISTILL is done it closes the adapter's standard input, and the adapter exits.
The adapter's standard error is left to the owner's terminal or the tick's log.

| Command | Reply | What the adapter does |
| --- | --- | --- |
| `hello 1 <open> <json>` | `ok` | Takes protocol version 1 and the target's table as one line of JSON. `<open>` says what this use may do. `probe`: check that the target is there and takes a write, making nothing that stays. `first`: the target's first use, so the adapter may set up its place (a bucket prefix, a folder). `again`: the target was used before, so a place that is missing is an error (a drive or host that isn't there must not become an empty new target). |
| `list` | `ok <n>`, then `n` lines with one name each | Names every object the target holds. |
| `put <name> <file>` | `ok stored` or `ok present` | Stores the bytes of the file at the path `<file>` under `<name>`, unless the target holds that name already. Then it stores nothing and says `present`. A put never overwrites. |
| `get <name> <file>` | `ok` | Writes the object's bytes into the file at the path `<file>`. |
| `sync` | `ok` | Makes every object the target lists durable. A run sends it before its record and again after it. |
| `has <name>` | `ok yes` or `ok no` | Says whether the target holds `<name>`. |
| `delete <name>` | `ok` | Removes the object `<name>`, if the target holds it. Used only to prune. |

The path `<file>` is the rest of the line. Each file is in a temporary folder only the owner can
use, and DISTILL removes it after the call. A reply DISTILL doesn't expect, or an adapter that
exits before replying, fails the use of the target.

DISTILL doesn't send `has` or `delete` yet: `put` answers whether the name was there, and
`delete` waits for pruning. An adapter should still answer both. It answers a command it doesn't
know with `error`, so a later version can add commands and tell old adapters apart.

The target's table sits in `config.toml` and goes to the adapter whole, so keep a credential out
of it. An adapter takes its credentials from where its tool keeps them (an SSH key, a cloud
CLI's profile, rclone's config), and a restore names the target with `--kind` and `--set`
alone.

## Adapters that ship

`scripts/install.sh` installs these from `adapters/`. Each script's header shows its settings.

- `distill-backup-ssh`: a folder on another machine the owner reaches over SSH (`host`, `dir`).
  It uses the owner's SSH keys and config as they are, never asks for a password, and shares one
  connection across a use.
- `distill-backup-rclone`: any rclone remote (`remote`), which covers S3-compatible stores,
  self-hosted ones included, and rclone's other backends. Set the remote up with `rclone config`.

`crates/distill-core/tests/fixtures/distill-backup-reference` is a reference adapter in `sh`
that keeps objects in a folder. The tests use it.
