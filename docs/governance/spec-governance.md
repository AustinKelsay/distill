# Distill Spec Governance

This document is normative.

## Authority Order

When documents disagree, use this order:

1. files under `docs/specs/`
2. `docs/governance/spec-governance.md`
3. `docs/testing/contract-test-matrix.md`
4. `docs/gaps/current-state-gap-register.md`
5. `docs/roadmap/spec-alignment-plan.md`
6. root markdown files
7. active implementation code
8. historical artifacts such as `schema.sql` and retired source paths
9. discovery notes

## PR Checklist

Every behavior-changing PR must answer all of these:

- Which canonical spec file did this change touch?
- Does the current implementation now match the canonical spec?
- If not, was `docs/gaps/current-state-gap-register.md` updated?
- Which gap IDs are affected by this change? If none, say `no gaps` explicitly.
- Does `docs/testing/contract-test-matrix.md` need new or changed scenarios?
- Are executable tests required now, or intentionally deferred to a listed follow-up branch?
- Did any root doc need a current-state summary update?

## When Docs Must Change

Docs must change in the same PR when any of the following changes:

- entity semantics
- import ordering or transaction boundaries
- search behavior
- curation behavior
- export contract
- audit behavior
- connector source truth
- acceptable failure handling
- branch sequencing or milestone acceptance criteria

## How To Record Gaps

Use `docs/gaps/current-state-gap-register.md` when:

- the canonical spec is ahead of the implementation
- the current implementation intentionally diverges
- a cleanup branch has been planned but not executed yet

Every gap entry must include:

- gap id
- canonical rule
- current behavior
- impacted files or modules
- severity
- target branch
- concrete acceptance criteria

## How To Add New Source Connectors

Before adding a new connector:

1. update `docs/specs/connectors.md`
2. update any shared-shape rules in `docs/specs/data-model.md`
3. add scenarios to `docs/testing/contract-test-matrix.md`
4. add a gap entry if the implementation will land in stages
5. only then add or modify implementation code

## Review Standard

The repository standard is not “the code seems reasonable.”

The standard is:

- the code matches the canonical docs, or
- the gap is explicit, named, test-planned, and branch-tracked
