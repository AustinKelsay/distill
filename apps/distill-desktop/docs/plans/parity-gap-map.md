# Distill Desktop Parity Gap Map

This document maps the current Rust scaffold against the Electron canonical product behavior.

## Baseline

Electron remains the product baseline as defined by:

- `../../distill-electron/docs/specs/architecture.md`
- `../../distill-electron/docs/specs/data-model.md`
- `../../distill-electron/docs/specs/ingest-pipeline.md`
- `../../distill-electron/docs/specs/connectors.md`
- `../../distill-electron/docs/specs/search-curation-export.md`
- `../../distill-electron/docs/specs/activity-and-ops.md`
- `../../distill-electron/docs/testing/contract-test-matrix.md`

## Current Rust Coverage

The Rust app currently implements a scaffold plus one real engine slice:

- native desktop shell with `Slint` and `winit`
- Rust-owned schema initialization plus explicit Electron compatibility mode
- session list and detail rendering
- logs rendering from existing jobs and exports
- DB browsing and guarded read-only SQL
- shell preference persistence
- Codex and Claude Code detect/discover/snapshot/parse in Rust
- Rust-owned raw capture persistence with inline/blob storage
- canonical capture insertion and projection replacement for Codex and Claude Code
- sync job and activity rows for the Rust-owned multi-connector import path

The Rust app now owns part of canonical Distill behavior, but only for a single source path and only for import/query foundations.

## Gap Summary By Product Layer

## 1. Source Discovery

Electron baseline:

- detect supported source installations
- discover candidate captures independently per source
- continue across partial source failures

Rust status:

- partial

Required for parity:

- keep the current `codex` and `claude_code` discovery paths stable
- add `opencode`
- broaden source health/status reporting in the UI

## 2. Connectors

Electron baseline:

- four operations per connector: `detect`, `discoverCaptures`, `snapshotCapture`, `parseCapture`
- source-specific parsing rules with canonical shared outputs

Rust status:

- partial

Required for parity:

- keep the current Rust connector trait aligned with Electron semantics
- add `opencode`
- broaden fixture-backed contract coverage from Codex and Claude Code to all sources

## 3. Snapshot And Raw Capture Ownership

Electron baseline:

- Distill-owned raw capture persistence
- checksum and byte-size tracking
- recoverable inline/blob storage
- canonical dedupe key on `(source_kind, source_path, raw_sha256)`

Rust status:

- partial

Required for parity:

- keep the current inline/blob capture persistence stable across multiple sources
- add explicit replay helpers and snapshot-failure coverage
- extend raw persistence to OpenCode virtual captures

## 4. Ingest Pipeline And Projection Replacement

Electron baseline:

- append-only capture history
- append-only capture records
- replace-on-success session projection
- rollback-on-failure semantics
- deterministic session identity fallback when sources lack stable ids

Rust status:

- partial

Required for parity:

- keep the current shared ingest runner stable across Codex and Claude Code
- add artifact-heavy paths and deterministic synthetic-id coverage
- add OpenCode and virtual-capture support
- keep replace-on-success and rollback-on-failure semantics enforced

## 5. Canonical Storage Model

Electron baseline:

- `sources`
- `captures`
- `capture_records`
- `sessions`
- `messages`
- `artifacts`
- `tags` and `tag_assignments`
- `labels` and `label_assignments`
- `exports`
- `activity_events`
- `jobs`

Rust status:

- owns the mirrored canonical schema and migrations in Rust mode
- writes captures, capture_records, sessions, messages, artifacts, jobs, and activity rows for Codex and Claude Code imports
- still relies on compatibility mode for Electron-era read-only access

Required for parity:

- Rust schema ownership and migrations
- typed storage layer around the canonical entities
- compatibility path for opening Electron-era homes during migration

## 6. Search And Query

Electron baseline:

- SQLite FTS over the current materialized projection
- punctuation-safe query normalization
- query read models derived from current projection and manual curation state

Rust status:

- simple in-memory read-model filtering
- no FTS

Required for parity:

- FTS tables or equivalent SQLite-backed search indexes
- canonical token normalization behavior
- session list/detail queries sourced from the current projection only

## 7. Manual Curation

Electron baseline:

- manual session-level tags
- manual session-level labels
- dataset-label exclusivity
- workflow state derivation
- auditable curation changes

Rust status:

- displays existing labels and tags only
- no write paths

Required for parity:

- tag add/remove flows
- label toggle flows
- transactional exclusivity enforcement for dataset labels
- activity event emission for every curation change

## 8. Export

Electron baseline:

- standard dataset export for `train` and `holdout`
- export eligibility rules
- turn-pair derivation
- export bookkeeping

Rust status:

- missing

Required for parity:

- export planning and execution paths
- JSONL or equivalent export writers
- export row persistence
- export detail in logs and activity history

## 9. Activity And Operations

Electron baseline:

- append-only `activity_events`
- sync lifecycle audit
- operational `jobs`
- logs derived from jobs and exports

Rust status:

- reads jobs and exports as logs
- creates sync job rows and core import audit rows for Rust-owned Codex and Claude Code syncs
- does not yet cover curation or export audit

Required for parity:

- activity event writer
- sync job queue and state machine
- logs/query layer over Rust-owned jobs and exports

## 10. Desktop Product Shell

Electron baseline:

- full product workflow: discover, import, review, curate, export, inspect ops

Rust status:

- Electron-like shell chrome and route layout
- sources panel, settings overlay, export stub, and curation stub surfaces
- a real multi-source engine action behind `Reload` in Rust mode

Required for parity:

- source management and sync actions
- curation actions
- export actions
- failure and empty-state UX for real operations
- turn the current UI stubs into working product flows backed by Rust

## What Counts As Parity

`distill-desktop` reaches parity only when it can replace Electron for the canonical local-first workflow:

`discover -> snapshot -> preserve -> normalize -> search -> curate -> export`

Read-only inspection alone does not count as parity.

## Immediate Conclusion

The shell is now much closer to Electron structurally, but parity is still blocked by engine gaps:

1. canonical Rust storage ownership
2. connectors and ingest
3. search and curation writes
4. export and operations
5. convert the current Electron-like UI stubs into real product actions
