# Distill Desktop Agent Instructions

## First Rule
- Current Rust code is not product truth; treat it as a scaffold.
- Read `docs/README.md`, `docs/plans/parity-gap-map.md`, `docs/roadmap/rebuild-roadmap.md`, and `docs/testing/parity-acceptance-matrix.md` before parity work.
- Use the Electron canonical docs under `../distill-electron/docs/` as the shipped-behavior baseline.

## Rules
- Keep rebuild planning and implementation self-contained under `apps/distill-desktop`.
- Prefer engine parity over UI polish in early phases.
- If Rust docs and Rust code differ, the docs define the intended target.

## Change Requirements
- Update the relevant file under `apps/distill-desktop/docs/` for behavior changes.
- Add or update executable tests for claimed contracts.
- Do not redefine Electron behavior locally without calling out the divergence.
- Unless instructed otherwise, attempt to run the CodeRabbit CLI on unstaged changes before committing and pushing.

## Engineering Principles
**1. Think Before Coding**: State assumptions, surface uncertainty, and present tradeoffs.
**2. Simplicity First**: Minimum code required. No speculative features or unnecessary abstractions.
**3. Surgical Changes**: Touch only what is necessary. Match existing style.
**4. Goal-Driven Execution**: Define success via verifiable tests/checks.
