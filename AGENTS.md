# Distill Agent Instructions

This file is authoritative for generic coding agents working in this repository.

## First Rule

Do not infer target behavior from historical Electron paths, `schema.sql`, or
root markdown files before reading the canonical docs package.

The canonical source of truth lives under `docs/`.

Start here:

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

## How To Use The Docs

- `docs/specs/*.md` define canonical architecture and behavior.
- `docs/gaps/current-state-gap-register.md` defines acknowledged drift between canonical specs and current code.
- `docs/testing/contract-test-matrix.md` defines required contract coverage and acceptance intent.
- `docs/roadmap/spec-alignment-plan.md` defines the intended branch sequence and delivery ordering.
- `docs/governance/spec-governance.md` defines the repo’s change-control rules.

## What Is Not Canonical

These are informative, not authoritative:

- `README.md`
- `PLAN.md`
- `IMPLEMENTATION.md`
- `DISCOVERY.md`
- `docs/legacy/electron/README.md`
- historical `schema.sql` and `src/**` paths recorded in Git history

Use the legacy boundary to understand migration provenance, but not to redefine
the spec. Active implementation lives under `crates/` and `apps/distill-desktop/`.

## When Docs And Code Differ

Follow this rule:

- canonical docs win
- if the code does not match the docs, check `docs/gaps/current-state-gap-register.md`
- if the divergence is not listed there, treat it as missing documentation and update the docs or gap register with the code change

## Required References By Task Type

### Import / storage / replay work

Read:

- `docs/specs/data-model.md`
- `docs/specs/ingest-pipeline.md`
- `docs/gaps/current-state-gap-register.md`

### Connector work

Read:

- `docs/specs/connectors.md`
- `docs/specs/data-model.md`
- `docs/testing/contract-test-matrix.md`

### Search / curation / export work

Read:

- `docs/specs/search-curation-export.md`
- `docs/specs/data-model.md`
- `docs/specs/activity-and-ops.md`

### Audit / jobs / logs work

Read:

- `docs/specs/activity-and-ops.md`
- `docs/specs/data-model.md`
- `docs/gaps/current-state-gap-register.md`

## Change Rules

For any behavior-changing change:

1. update the relevant canonical doc under `docs/specs/`
2. update `docs/gaps/current-state-gap-register.md` if implementation still diverges
3. update `docs/testing/contract-test-matrix.md` if acceptance criteria or fixtures changed
4. update executable tests when the contract is meant to be enforced now

If you add a new connector or change canonical behavior, do not patch code first and explain later. Update the spec surface in the same change.
