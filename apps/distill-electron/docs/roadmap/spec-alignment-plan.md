# Distill Electron Spec Alignment Plan

This document is normative for the sequencing of the spec realignment program.

## Goal

Make Distill Electron’s documentation truthful, tech-agnostic, and hard to drift from, then use that spec package to drive future implementation branches.

## Current Status

As of 2026-03-31, the branch sequence below is implemented in the current tree and should be read as the historical spec-alignment program that produced the current baseline.

Use the gap register for any newly discovered drift. Add new roadmap entries only when future work changes canonical behavior or opens a new staged-alignment branch.

## Branch Sequence

### 1. `docs/spec-foundation`

Scope:

- create the `docs/` tree
- add the docs index and governance file
- rewrite root docs into concise entrypoints

Depends on:

- nothing

Acceptance criteria:

- root docs no longer overclaim behavior
- authority order is explicit
- every root doc points to canonical docs under `docs/`

### 2. `docs/canonical-domain-spec`

Scope:

- architecture spec
- data model spec
- ingest pipeline spec

Depends on:

- `docs/spec-foundation`

Acceptance criteria:

- raw capture and projection semantics are explicit
- append-only vs replace-on-success rules are unambiguous
- failure handling is defined without hand-waving

### 3. `docs/query-curation-ops-spec`

Scope:

- connector spec
- search/curation/export spec
- activity and operations spec

Depends on:

- `docs/canonical-domain-spec`

Acceptance criteria:

- search scope is explicit
- manual curation is clearly normative
- jobs, logs, and activity are clearly separated

### 4. `docs/gap-register-and-roadmap`

Scope:

- gap register
- multi-branch roadmap

Depends on:

- `docs/query-curation-ops-spec`

Acceptance criteria:

- every material divergence has a gap id
- every gap maps to a target branch
- branch dependencies are explicit

### 5. `docs/test-matrix`

Scope:

- contract-test matrix
- governance checklist update for spec/test coupling

Depends on:

- `docs/gap-register-and-roadmap`

Acceptance criteria:

- every critical invariant has at least one named test scenario
- fixture expectations are concrete
- branch mapping is complete

### 5A. `test/connector-contract-hardening`

Scope:

- shared ingest fixture corpus under `src/test/fixtures/ingest/`
- typed fixture/install helper under `src/test/support/ingest_fixtures.ts`
- executable connector contract suite in `src/test/connector_contract.test.ts`
- refactor core parse and import tests to reuse the shared fixture corpus where it reduces duplication

Depends on:

- `docs/test-matrix`

Acceptance criteria:

- `CC-001`, `CC-002`, and `CC-003` are executable and passing against the shared fixture corpus
- the fixture manifest covers Codex live plus archived duplicate, Claude mixed structured blocks, OpenCode visible meta export, parse-failure, snapshot-failure, and large blob-backed capture cases
- import and parse tests reuse the shared fixture corpus for at least raw persistence, projection replacement, and parse-failure rollback coverage

### 6. `test/raw-capture-contracts`

Scope:

- executable tests for raw capture persistence
- executable tests for projection rollback and replacement

Depends on:

- `docs/test-matrix`

Acceptance criteria:

- tests fail when the implementation violates the raw-capture and projection specs
- tests become the gate for future storage changes

### 7. `impl/raw-capture-persistence`

Scope:

- Distill Electron-owned raw capture storage
- consistent behavior for file-backed and virtual captures

Depends on:

- `test/raw-capture-contracts`

Acceptance criteria:

- replay and re-normalization can start from Distill Electron-owned data
- capture history becomes independently recoverable

### 8. `impl/activity-and-curation-audit`

Scope:

- missing activity coverage
- clear separation between audit and operational logs

Depends on:

- `docs/test-matrix`

Acceptance criteria:

- activity coverage matches the canonical event taxonomy
- curation actions become auditable

### 9. `impl/query-and-search-alignment`

Scope:

- search behavior tightened to the canonical spec
- tests for projection-sensitive search behavior

Depends on:

- `docs/test-matrix`

Acceptance criteria:

- search results come from the current projection only
- re-import behavior does not leak stale projection rows into search

### 10. `impl/projection-cleanup`

Scope:

- explicit projection semantics in code
- consistent message and artifact linkage

Depends on:

- `impl/raw-capture-persistence`
- `impl/query-and-search-alignment`

Acceptance criteria:

- artifact provenance and message linkage are consistent
- projection rules are directly reflected in code and tests

## Milestones

### Milestone A: Canonical Docs In Place

Complete branches 1 through 5.

Outcome:

- the repo has a stable documentation authority model
- implementation drift is explicit
- test intent is decision complete

### Milestone B: Storage And Audit Alignment

Complete branches 6 through 8.

Outcome:

- raw capture and audit behavior match the canonical specs

### Milestone C: Projection And Query Alignment

Complete branches 9 and 10.

Outcome:

- search and projection behavior are consistent, explicit, and test-gated

## Recommended Review Order

Review in this order:

1. `docs/README.md`
2. `docs/specs/architecture.md`
3. `docs/specs/data-model.md`
4. `docs/specs/ingest-pipeline.md`
5. `docs/specs/connectors.md`
6. `docs/specs/search-curation-export.md`
7. `docs/specs/activity-and-ops.md`
8. `docs/gaps/current-state-gap-register.md`
9. `docs/testing/contract-test-matrix.md`
10. `docs/governance/spec-governance.md`
11. this roadmap
