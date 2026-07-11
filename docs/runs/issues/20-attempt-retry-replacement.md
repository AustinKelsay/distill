# Issue Session — #20 Attempt Retry, Replay, and Projection Replacement

## Issue

- Issue: [#20](https://github.com/AustinKelsay/distill/issues/20)
- Fixed point before session: `4564d28`
- Worker session: Grok 4.5 xhigh edit session
- Commit: Pending
- Status: Implementation complete — awaiting review/commit

## Inputs

- Spec issue: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Ticket: Add attempt retry, replay, and atomic projection replacement
- Relevant glossary terms: Capture, Normalization Attempt, Capture Fact, Session Identity, Session Projection, Activity Event
- Relevant ADRs: `0002-captures-attempts-and-projections-are-distinct`, `0003-sqlite-and-content-addressed-files-are-library-internals`
- Prototype answer and source branch, if any: None

## Implementation

- Public interface used: Rust `Library` with real temporary home, SQLite, CAS, and Fixture adapter/replay paths
- Behaviors covered: duplicate inertness; changed immutable Capture; parse/projection failure history, safe diagnostics, and last-good preservation; same-Capture strictly-newer-parser retry from Distill-owned bytes; immutable prior successful Attempts/Facts; replace-even-when-shorter and empty; honest counters surfaced through Library, CLI, and desktop result models
- `tdd` used: Yes — one Library public-seam contract per red/green cycle in `crates/distill-library/tests/library_attempt_retry.rs`; caller assertions only for named count presentation
- Commands run during implementation:
  - `cargo fmt --all` / `cargo fmt --all -- --check` — pass
  - `cargo clippy --workspace --all-targets -- -D warnings` — pass
  - `cargo test --workspace` — pass (Library attempt-retry 8, Library tracer 6, CLI 4, host 4)
  - `cargo build --workspace` — pass
  - `npm run desktop:typecheck` / `desktop:test` / `desktop:frontend:build` — pass (renderer 5)
  - `npm run desktop:build` (`tauri build --no-bundle`) — pass
  - `npm test` on Node `v26.0.0` — 103 pass; `src/**` preserved
  - `coderabbit review --agent --light --type uncommitted` — attempted; service rate-limited with a two-minute retry window
- Full suite command: `cargo test --workspace` + `npm run desktop:test` + `npm test`

## Review

- Review fixed point: Pending
- Standards findings: Pending
- Spec findings: Pending
- Worthy fixes applied: Pending
- Findings ignored with reasons: Pending

## Risks

- Fault injection and crash-point reopen repair remain #21; this ticket covers ordinary typed failures and transaction rollback only.
- Generic Sync Run concurrency/cancellation remains #22.
- Renormalize currently supports the registered Fixture parser only; provider SourceAdapters arrive in later tickets.
