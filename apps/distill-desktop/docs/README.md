# Distill Desktop Docs

This directory is the planning and rebuild package for `apps/distill-desktop`.

It exists to answer one question clearly:

How does the Rust app get from a read-only scaffold to a real replacement for Distill Electron?

## Scope

These docs are authoritative for the Rust rebuild direction inside `apps/distill-desktop`.

They do not replace the Electron canonical docs as the definition of current product behavior. Instead:

- Electron docs define the behavior we are rebuilding toward
- desktop docs define the Rust-side roadmap, sequencing, and acceptance plan

## Read Order

Read these files in order:

1. `docs/plans/parity-gap-map.md`
2. `docs/roadmap/rebuild-roadmap.md`
3. `docs/testing/parity-acceptance-matrix.md`

Then read the Electron baseline docs when implementing a specific capability:

1. `../distill-electron/docs/specs/architecture.md`
2. `../distill-electron/docs/specs/data-model.md`
3. `../distill-electron/docs/specs/ingest-pipeline.md`
4. `../distill-electron/docs/specs/connectors.md`
5. `../distill-electron/docs/specs/search-curation-export.md`
6. `../distill-electron/docs/specs/activity-and-ops.md`
7. `../distill-electron/docs/testing/contract-test-matrix.md`

## Current Reality

The current Rust app is now a mixed scaffold plus first engine slice:

- it defaults to a Rust-owned Distill home and schema
- it keeps Electron compatibility mode explicitly read-only
- it can detect, discover, snapshot, parse, and import Codex and Claude Code captures into the Rust-owned store
- it renders `Sessions`, `DB`, and `Logs` over either backend through an Electron-like shell structure
- it exposes sources, settings, export, and curation surfaces in the UI, but unsupported actions remain disabled
- it still does not implement OpenCode, curation writes, export, or full search parity

That is meaningful progress beyond a viewer, but it is still far from product parity.

## Source Of Truth By Concern

- parity assessment: `docs/plans/parity-gap-map.md`
- staged implementation plan: `docs/roadmap/rebuild-roadmap.md`
- acceptance and test intent: `docs/testing/parity-acceptance-matrix.md`

## Manual QA Checklist

When the desktop shell changes, capture these views side-by-side against Electron:

- `Sessions` with real imported data
- `Sessions` empty/onboarding state
- `Sessions` with the sources panel open
- settings overlay open
- `Logs` route with at least one expanded card
- `DB` route on the `Browse` tab
- `DB` route on the `Query` tab

## Working Rule

Keep rebuild planning and implementation inside `apps/distill-desktop` unless the user explicitly asks for coordinated Electron changes.
