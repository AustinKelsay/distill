# Several machines: one hub, its spokes

A **hub** holds the archive, the index, the cards and the read path. A **spoke** only captures, and the hub pulls its new captures over SSH (ADR 0010). Neither side opens a listener, and the spoke never holds a credential for the hub.

## A spoke

On the machine that runs agents:

```sh
scripts/install.sh --spoke          # `distill` alone: no window, no MCP server
distill init --spoke nas            # this machine's name: lowercase letters, digits and -
distill schedule install            # its timer only sweeps
```

The spoke's sessions are keyed `nas:<harness>/<id>` on the hub. A machine keeps its name.

## The hub

In the hub's `config.toml`, one table per spoke:

```toml
[[machines]]
name = "nas"            # the name in the spoke's [spoke]
host = "me@nas"         # what `ssh` reaches it by: a host, user@host, or a Host in ~/.ssh/config
# command = "/home/me/.cargo/bin/distill"   # when `distill` isn't on the PATH SSH gives it
```

Or add it in the window, under Setup's Machines step, with its name and SSH host. There, Check connects as a pull starts and reads only the export's header: it says whether the spoke answers as that name and has captures to pull, and takes nothing. It hangs up on the export, which the spoke logs, usually as cut short.

Each tick then runs `ssh <host> distill export --since <bytes>` for each machine, after the sweep, and the parse step turns what came into sessions. `<bytes>` is the length of the hub's copy of that spoke's manifest, so the copy is its own cursor: a restored or rebuilt hub goes on from where its copy ends. `distill pull` (or `distill pull nas`) runs it at once. The hub refuses a machine that names itself otherwise, or whose archive is not the one it has a copy of.

SSH runs with `BatchMode`, so it never asks for a password: the hub's key must be in the spoke's `authorized_keys`, and the spoke in the hub's `known_hosts` (connect once by hand).

## Limiting the hub's key to the export

On the spoke, the hub's key can run nothing but the export. Prefix its line in `~/.ssh/authorized_keys`:

```text
command="distill export --ssh",restrict ssh-ed25519 AAAA… hub
```

`restrict` turns off a terminal, forwarding and `~/.ssh/rc`, whose output would break the export. With `--ssh`, the export takes where to start from what the hub asked to run (`SSH_ORIGINAL_COMMAND`), which must end `export --since <bytes>`; whatever else it asks for, only the export runs. Use the full path to `distill` if it isn't on SSH's `PATH`.

## What leaves the spoke

An export is the spoke's raw captures, as they are, to the owner's own hub: not a read, and no reader gets it. It is protected on the wire by SSH alone. The spoke logs each export in `log/exports.jsonl`: when, from where to where in its manifest, and how many captures and bytes went.

## Checking on them

`distill search` and `distill tags` take `--machine <name>` to keep to one spoke's sessions, or `--machine hub` for the hub's own (a spoke named `hub` can't be picked this way); the MCP tools take `machine` the same way.

On the hub, `distill status` has a line for each spoke: its health, its sessions and when a pull last took from it, then its harnesses as the hub's own are shown. A spoke whose newest pull failed says why. When a spoke in `[[machines]]` hasn't been pulled from for twice the timer's interval, status warns. On a spoke, the exports count among what left it, as `the hub, over ssh`.
