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

Each tick then runs `ssh <host> distill export --since <bytes>` for each machine, after the sweep, and the parse step turns what came into sessions. `distill pull` (or `distill pull nas`) runs it at once. The hub refuses a machine that names itself otherwise, or whose archive is not the one it has a copy of.

SSH runs with `BatchMode`, so it never asks for a password: the hub's key must be in the spoke's `authorized_keys`, and the spoke in the hub's `known_hosts` (connect once by hand).

## Limiting the hub's key to the export

On the spoke, the hub's key can run nothing but the export. Prefix its line in `~/.ssh/authorized_keys`:

```text
command="distill export --ssh",no-pty,no-port-forwarding,no-agent-forwarding,no-X11-forwarding ssh-ed25519 AAAA… hub
```

With `--ssh`, the export reads where to start from what the hub asked to run (`SSH_ORIGINAL_COMMAND`), which must be `distill export --since <bytes>`; anything else is refused. Use the full path to `distill` if it isn't on SSH's `PATH`.

## What leaves the spoke

An export is the spoke's raw captures, as they are, to the owner's own hub: not a read, and no reader gets it. It is protected on the wire by SSH alone. The spoke logs each export in `log/exports.jsonl`: when, from where to where in its manifest, and how many captures and bytes went.
