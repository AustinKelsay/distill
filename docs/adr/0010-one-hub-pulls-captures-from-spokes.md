# One hub pulls captures from spokes over SSH, into an archive per machine

An owner who runs agents on several machines has a partial archive on each, or none (#135). DISTILL takes them into one: a single **hub** holds the archive, the index, the cards and the read path, and every other machine is a **spoke** that only captures. Search, reading, reveals and the read log work on the hub as they do now, across every machine. A federation, where every machine is a full DISTILL and a search fans out, was the other shape. It would put a read path, models and a trust list on every machine and make a search depend on every machine being up, so it is not built.

## The hub pulls

The hub's tick pulls from each spoke over SSH: it runs `ssh <host> distill export --since <cursor>` and takes the blobs and manifest records the spoke has added since the last pull. Neither side opens a listener. The SSH key lives on the hub, and the spoke's `authorized_keys` can limit it to that one command. A spoke never holds a credential for the hub. Blobs are named by their SHA-256, so the hub checks each blob it receives and skips any it has, which makes a pull safe to repeat after one stops partway. The cursor is kept in `state/` on the hub, so it can be rebuilt. Pulling is a tick step inside the tick's time limit.

## An archive per machine

The hub keeps each spoke's captures in a raw archive of their own, `machines/<machine>/raw/`, in the same append-only format as `raw/`. `archive.rs` writes it, as it writes `raw/`. The hub's own captures stay in `raw/`. Where a capture is stored says which machine it came from, so neither manifest format changes (ADR 0001), and no capture of one machine is mistaken for another's even when both have the same origin path. Everything under `sessions/`, `index/` and `state/` is still rebuilt from the raw archives, now including each session's machine. Every raw archive is in the backup set (ADR 0009).

A spoke's home names its machine once, in its config: a name the owner picks, which the hub uses as `<machine>`. A name is lowercase letters, digits and `-`, unique among the hub's machines, and not the hub's own. The hub refuses a spoke whose name is already in use before it takes any capture from it. Session identity becomes the machine, the harness and the session id. The hub's own sessions keep their keys (`<harness>/<id>`), so the read log, tags and reveal requests that name them stay valid. A spoke's sessions are keyed `<machine>:<harness>/<id>`. A machine keeps its name: renaming one would change its sessions' keys, so it is not offered.

## What a spoke does

A spoke runs only the capture half: the sweep and the archive, from a timer of its own. It parses nothing, runs no model, serves no reader (no MCP server, no read path) and holds no index. It keeps its raw archive after the hub has it, since that archive is append-only and is the spoke's own copy. Pruning a spoke after the hub confirms a pull can come later. Every export is recorded on the spoke, so its owner can see what the hub took and when.

## Models run on the hub

Cards, personal details and vectors run only on the hub, under the hub's runtime and trust list (ADR 0003, ADR 0008). A spoke has no trust list, because nothing reads there.

## Consequences

- A spoke's export is a third way session text leaves a DISTILL home, beside the read path (ADR 0002) and a backup (ADR 0009). It goes only to the owner's own hub, as raw captures over SSH. It is not a read: no reader gets it, and on the hub it leaves only through the read path or a backup, as before. The spoke records each export. On the wire it is protected by SSH alone, and on a spoke by that machine's disk. This replaces ADR 0009's "there is no third". AGENTS.md, the glossary's read path, and the test that keeps faces on the read path change when the export ships (#138), since they describe what exists.
- A spoke that is off or unreachable is behind until the next pull, and status says so.
- A spoke's disk keeps every capture it ever made, as the hub's does.
- Only the hub has a window worth opening. A spoke is set up from the command line.
- Machine identity costs version bumps in the session facts, the index and the cards, not migrations, since all of them are rebuildable.
