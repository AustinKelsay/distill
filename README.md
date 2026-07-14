# DISTILL

DISTILL is a local-first desktop app for collecting, normalizing, searching,
curating, and exporting local LLM chat history.

## Status

The first beta is `0.2.0-beta.1`. The shipped product is a Rust Library with a
thin Rust CLI and a Tauri 2 + React desktop app. Codex CLI, Claude Code, and
OpenCode are supported capture sources.

The former Electron/TypeScript product source has been retired from this
workspace. Native Rust migration code can still read an old Electron-shaped
Distill home without modifying it; see [the legacy boundary](docs/legacy/electron/README.md).

## What DISTILL does

- discovers supported local chat captures
- preserves raw capture content in DISTILL-owned local storage
- normalizes sessions, messages, and artifacts into local SQLite
- searches and reviews the current session projection
- labels and tags sessions
- exports approved sessions to JSONL
- imports a legacy Electron home through a read-only migration seam

Everything is local-first: source data stays on the machine, and the importer
creates its own durable copy.

## Local setup

```bash
npm ci
npm run check:release
npm run desktop:dev
```

The Rust CLI can exercise the same Library contract without the desktop host:

```bash
cargo run -p distill-cli -- --home /tmp/distill-home --fixture /path/to/fixture --format json
```

By default, Distill stores its local database and files in `~/.distill`. Set
`DISTILL_HOME` to use another location.

## Beta packaging

```bash
npm run desktop:package:macos
npm run desktop:package:linux       # Ubuntu/Linux
npm run release:package:windows    # Windows
```

The release workflow builds macOS, Linux, and Windows artifacts from the
`v0.2.0-beta.1` tag. Release packaging never enables the smoke-test DOM marker;
the signed/notarized macOS path needs Apple credentials. See
[docs/release/first-beta.md](docs/release/first-beta.md) for the exact claims
and remaining manual checks.

## Canonical docs

The authoritative architecture, behavior, and verification docs live under
[docs/README.md](docs/README.md). [IMPLEMENTATION.md](IMPLEMENTATION.md) is a
short current implementation map, not a second specification.
