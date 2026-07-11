# Issue Session — #19 Thin Tauri, React, and CLI Callers

## Issue

- Issue: [#19](https://github.com/AustinKelsay/distill/issues/19)
- Fixed point before session: `5655cde`
- Worker session: Grok 4.5 xhigh edit session
- Commit: Pending (do not commit in this worker session)
- Status: Implementation complete; awaiting review commit

## Inputs

- Spec issue: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Ticket: Wire thin Tauri, React, and CLI Fixture callers
- Relevant glossary terms: Source, Capture, Session Identity, Session Projection, Activity Event
- Relevant ADRs: `0001-rust-library-with-tauri-shell`, `0002-captures-attempts-and-projections-are-distinct`, `0003-sqlite-and-content-addressed-files-are-library-internals`
- Prototype answer and source branch, if any: None; the preserved Slint scaffold is research evidence only and is not copied

## Implementation

- Public interface used: existing Rust `Library` methods plus the smallest caller-oriented additions (`SessionIdentity`, `SourceSummary`, `FixtureJourneyPhase`, `FixtureJourneyResult`, `detect_fixture`, `run_fixture_journey`); no SQL/storage exposure
- Behaviors covered: chosen-home Fixture CLI with human/JSON output and exit codes `0`/`1`/`2`; async Tauri command over the same journey; typed progress/result/error boundary; React first-run form and source/sync/session/health result states; no renderer ambient authority; minimal Tauri capability file
- `tdd` used: Yes — real CLI binary, Tauri host boundary, and React typed-bridge fake are the approved seams
- Commands run during implementation:
  - `cargo fmt --all -- --check` — pass
  - `cargo clippy --workspace --all-targets -- -D warnings` — pass
  - `cargo test --workspace` — pass (Library 6, CLI 4, desktop host 4)
  - `cargo build --workspace` — pass
  - `npm run desktop:typecheck|lint|format|test|frontend:build` — pass (5 renderer tests across UI and production bridge)
  - `npm run desktop:build` — pass; Tauri release host built with `--no-bundle`
  - `npm run build` — pass
  - legacy `npm test` — 103 passed on verification host Node `v26.0.0`
- Full suite command: `cargo test --workspace` plus `npm run desktop:test` plus legacy `npm test`
- Node note: the known Electron DB-inspector limitation applies on Node 22 baselines that lack `DatabaseSync.setAuthorizer`. This ticket does not patch that legacy surface; the verification host used Node `v26.0.0`, where the ten inspector tests also pass.

## Review

- Review fixed point: Pending
- Standards findings: Pending
- Spec findings: Pending
- Worthy fixes applied: Integration pass corrected the production Tauri invoke argument casing, narrowed the capability file to event permissions only, made async listener cleanup race-safe, and added production-bridge contracts for both behaviors.
- Findings ignored with reasons: Pending

## Risks

- Tauri desktop compilation may depend on platform system libraries. Packaging and install smoke remain #35/#36; this ticket proves the host boundary and development build only.
- The known Node 22 Electron DB-inspector limitation (`DatabaseSync.setAuthorizer` missing) remains unpatched. This ticket does not change that legacy surface. The verification host for this session ran Node `v26.0.0`, where those ten inspector tests pass.
- Generic Sync Runs, provider adapters, Curation, export, and search UI remain deferred (GAP-R002, GAP-R003).
- The required pre-commit local CodeRabbit attempt was rate-limited for 18 minutes; independent Grok standards/spec review remains required after commit.
