# Distill Docs

This directory is the canonical documentation package for Distill.

## Normative vs Non-Normative

Normative documents define the architecture, contracts, and engineering rules that new work must follow.

Authoritative normative documents:

1. `docs/specs/architecture.md`
2. `docs/specs/data-model.md`
3. `docs/specs/ingest-pipeline.md`
4. `docs/specs/connectors.md`
5. `docs/specs/search-curation-export.md`
6. `docs/specs/activity-and-ops.md`
7. `docs/specs/legacy-migration.md`
8. `docs/governance/spec-governance.md`
9. `docs/testing/contract-test-matrix.md`
10. `docs/gaps/current-state-gap-register.md`
11. `docs/roadmap/spec-alignment-plan.md`

Non-normative documents explain the current implementation, preserve research notes, or provide navigation:

- `README.md`
- `PLAN.md`
- `IMPLEMENTATION.md`
- `DISCOVERY.md`
- `schema.sql`
- `src/**`

`schema.sql` and `src/**` are implementation artifacts, not the canonical domain specification. When they diverge from the canonical docs, the docs and the gap register win.

## How To Read The Docs

Read the documents in this order:

1. `docs/specs/architecture.md`
2. `docs/specs/data-model.md`
3. `docs/specs/ingest-pipeline.md`
4. `docs/specs/connectors.md`
5. `docs/specs/search-curation-export.md`
6. `docs/specs/activity-and-ops.md`
7. `docs/specs/legacy-migration.md`
8. `docs/gaps/current-state-gap-register.md`
9. `docs/testing/contract-test-matrix.md`
10. `docs/roadmap/spec-alignment-plan.md`
11. `docs/governance/spec-governance.md`

That order moves from system intent to entity semantics to pipeline behavior to current drift, tests, delivery order, and process.

## Source Of Truth Files

- System model: `docs/specs/architecture.md`
- Canonical entities and invariants: `docs/specs/data-model.md`
- Import and re-import behavior: `docs/specs/ingest-pipeline.md`
- Connector boundary: `docs/specs/connectors.md`
- Search, curation, and export behavior: `docs/specs/search-curation-export.md`
- Audit and operational behavior: `docs/specs/activity-and-ops.md`
- Legacy Electron migration behavior: `docs/specs/legacy-migration.md`
- Known implementation drift: `docs/gaps/current-state-gap-register.md`
- Required contract tests: `docs/testing/contract-test-matrix.md`
- Verification gates: `docs/gates.md`
- Delivery sequence: `docs/roadmap/spec-alignment-plan.md`
- Contribution rules: `docs/governance/spec-governance.md`

## Updating Docs And Tests

Every code change that alters behavior covered by the canonical specs must update the matching doc and the test matrix in the same change.

Required update flow:

1. Update the relevant file under `docs/specs/`.
2. If the implementation still diverges, update `docs/gaps/current-state-gap-register.md`.
3. Update `docs/testing/contract-test-matrix.md` when acceptance criteria or fixtures change.
4. Update the root docs only if the current-state summary or doc map changes.
5. Add or update executable tests when a contract moves from planned to enforced.

If a behavior matters but is not yet specified, add it to the canonical docs before implementing it.
