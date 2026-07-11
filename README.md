# DISTILL

DISTILL is a local-first desktop app for collecting, normalizing, searching, curating, and exporting local LLM chat history.

The point of DISTILL is simple: if you already have chat history on disk from tools like Codex CLI, Claude Code, or OpenCode, DISTILL gives you one local place to pull that data together, inspect it, organize it, and turn approved sessions into exportable datasets.

## Status

DISTILL is still in alpha and still being built out.

The core flow exists today, but the product is early. Expect active changes to the UI, workflows, and supported capabilities.

## Supported Sources Right Now

- Codex CLI
- Claude Code
- OpenCode

## What DISTILL Does

- discovers supported local chat captures
- snapshots and preserves raw capture content in DISTILL-owned local storage
- normalizes sessions, messages, and artifacts into a local SQLite database
- lets you search and review the current session projection
- lets you manually label and tag sessions
- exports approved sessions to JSONL

Everything is local-first. DISTILL reads local source data, stores its own local copy, and works from there.

## DISTILL Flow

```text
Local source data
(Codex / Claude Code / OpenCode)
            |
            v
      Discover captures
            |
            v
  Snapshot + preserve raw content
            |
            v
   Normalize into local SQLite
            |
            v
      Search and review
            |
            v
   Curate with labels / tags
            |
            v
    Export approved JSONL
```

## Local Setup

If you just want to get DISTILL running locally right now:

```bash
npm install
npm run doctor
npm run import
npm start
```

What those commands do:

- `npm install` installs the app dependencies.
- `npm run doctor` checks whether supported local sources are installed and detectable.
- `npm run import` imports any discovered local chat history into DISTILL.
- `npm start` builds and opens the Electron app.

By default, DISTILL stores its local database and files in `~/.distill`. That directory is created automatically on first run.

If you want to use a custom local data directory:

```bash
export DISTILL_HOME=/path/to/custom/.distill
```

If you want to export labeled data:

```bash
npm run export -- train
```

or:

```bash
npm run export -- holdout
```

## Canonical Docs

This root README is intentionally simple.

The authoritative architecture and product docs live under [docs/README.md](docs/README.md).

## Rebuild Callers (informative)

Native rebuild crates live beside the Electron baseline:

- Library: `crates/distill-library`
- CLI: `crates/distill-cli` (`distill --home … --fixture …`)
- Desktop: `apps/distill-desktop` (Tauri 2 + React first-run Fixture UI)

See [IMPLEMENTATION.md](IMPLEMENTATION.md) for rebuild commands. Packaging remains deferred.
