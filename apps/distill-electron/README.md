# DISTILL ELECTRON
Data Infrastructure for Storing, Tagging, Indexing, and Labeling Locally

DISTILL ELECTRON is a local-first desktop app for collecting, normalizing, searching, curating, and exporting local LLM chat history.

The point of DISTILL ELECTRON is simple: if you already have chat history on disk from tools like Codex CLI, Claude Code, or OpenCode, DISTILL ELECTRON gives you one local place to pull that data together, inspect it, organize it, and turn approved sessions into exportable datasets.

## Status

DISTILL ELECTRON is still in alpha and still being built out.

The core flow exists today, but the product is early. Expect active changes to the UI, workflows, and supported capabilities.

## Supported Sources Right Now

- Codex CLI
- Claude Code
- OpenCode

## What DISTILL ELECTRON Does

- discovers supported local chat captures
- snapshots and preserves raw capture content in DISTILL ELECTRON-owned local storage
- normalizes sessions, messages, and artifacts into a local SQLite database
- lets you search and review the current session projection
- lets you manually label and tag sessions
- exports approved sessions to JSONL

Everything is local-first. DISTILL ELECTRON reads local source data, stores its own local copy, and works from there.

## DISTILL ELECTRON Flow

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

If you just want to get DISTILL ELECTRON running locally right now:

```bash
npm install
npm run doctor
npm run import
npm start
```

What those commands do:

- `npm install` installs the app dependencies.
- `npm run doctor` checks whether supported local sources are installed and detectable.
- `npm run import` imports any discovered local chat history into DISTILL ELECTRON.
- `npm start` builds and opens the Electron app.

By default, DISTILL ELECTRON stores its local database and files in `~/.distill-electron`. That directory is created automatically on first run.

If you want to use a custom local data directory:

```bash
export DISTILL_ELECTRON_HOME=/path/to/custom/.distill-electron
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
