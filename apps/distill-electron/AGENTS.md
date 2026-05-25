# Distill Electron Agent Instructions

## First Rule
- Canonical product behavior lives under `docs/`, not in `src/**`, `schema.sql`, or root markdown files.
- Read `docs/README.md`, the spec files under `docs/specs/`, `docs/gaps/current-state-gap-register.md`, `docs/testing/contract-test-matrix.md`, `docs/roadmap/spec-alignment-plan.md`, and `docs/governance/spec-governance.md` before behavior work.

## Canonical Use
- `docs/specs/*.md` define architecture and behavior.
- `docs/gaps/current-state-gap-register.md` records accepted drift.
- `docs/testing/contract-test-matrix.md` defines contract coverage.
- Files such as `README.md`, `PLAN.md`, `IMPLEMENTATION.md`, `DISCOVERY.md`, `schema.sql`, and current `src/**` code are informative only.

## Change Requirements
- Update the relevant canonical spec for behavior changes.
- Update the gap register if code still diverges.
- Update the contract matrix and executable tests when the contract changes.
- Unless instructed otherwise, attempt to run the CodeRabbit CLI on unstaged changes before committing and pushing.

## Engineering Principles
**1. Think Before Coding**: State assumptions, surface uncertainty, and present tradeoffs.
**2. Simplicity First**: Minimum code required. No speculative features or unnecessary abstractions.
**3. Surgical Changes**: Touch only what is necessary. Match existing style.
**4. Goal-Driven Execution**: Define success via verifiable tests/checks.
