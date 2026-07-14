# Distill Claude Instructions

This file mirrors `AGENTS.md` for Claude-family agents and other tools that preferentially read `CLAUDE.md`.

## Agent skills

### Issue tracker

Issues and specs are tracked in GitHub Issues. See `docs/agents/issue-tracker.md`.

### Triage labels

The repo uses the canonical Matt Pocock triage-role labels. See `docs/agents/triage-labels.md`.

### Domain docs

Distill uses a single product-domain context with root `CONTEXT.md` and system decisions under `docs/adr/`. See `docs/agents/domain.md`.

## Read This Documentation In Order Before Making Behavior Decisions

1. `docs/README.md`
2. `docs/specs/architecture.md`
3. `docs/specs/data-model.md`
4. `docs/specs/ingest-pipeline.md`
5. `docs/specs/connectors.md`
6. `docs/specs/search-curation-export.md`
7. `docs/specs/activity-and-ops.md`
8. `docs/gaps/current-state-gap-register.md`
9. `docs/testing/contract-test-matrix.md`
10. `docs/roadmap/spec-alignment-plan.md`
11. `docs/governance/spec-governance.md`

## Canonical Rule

The `docs/` tree is the authoritative source of target behavior.

Do not treat these as canonical:

- `README.md`
- `PLAN.md`
- `IMPLEMENTATION.md`
- `DISCOVERY.md`
- `docs/legacy/electron/README.md`
- historical `schema.sql` and `src/**` paths recorded in Git history

The legacy boundary is useful for migration provenance only. Active
implementation lives under `crates/` and `apps/distill-desktop/`.

## If You Are About To Touch These Areas

### Connectors

Read:

- `docs/specs/connectors.md`
- `docs/specs/data-model.md`
- `docs/testing/contract-test-matrix.md`

### Import, storage, captures, projection

Read:

- `docs/specs/data-model.md`
- `docs/specs/ingest-pipeline.md`
- `docs/gaps/current-state-gap-register.md`

### Search, tags, labels, export

Read:

- `docs/specs/search-curation-export.md`
- `docs/specs/activity-and-ops.md`
- `docs/testing/contract-test-matrix.md`

### Audit, jobs, logs

Read:

- `docs/specs/activity-and-ops.md`
- `docs/specs/data-model.md`
- `docs/gaps/current-state-gap-register.md`

## If Code And Docs Differ

- canonical docs win
- then check the gap register
- if the divergence is missing from the gap register, add or update documentation in the same change

## Required Change Hygiene

For any behavior-changing change:

1. update the matching canonical spec under `docs/specs/`
2. update the gap register if implementation still diverges
3. update the contract-test matrix if acceptance intent changed
4. add or update executable tests when the contract is intended to be enforced now

If you are unsure which file is authoritative, start at `docs/README.md` and follow the stated order there.
