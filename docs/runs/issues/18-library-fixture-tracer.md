# Issue Session — #18 Library Fixture Tracer

## Issue

- Issue: [#18](https://github.com/AustinKelsay/distill/issues/18)
- Fixed point before session: `b471a77`
- Worker session: Grok 4.5 xhigh edit session
- Commit: Pending (do not commit in this worker pass)
- Status: Implementation complete; independent verification and review pending

## Inputs

- Spec issue: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Ticket: Prove the Library-only Fixture tracer
- Relevant glossary terms: Source, Capture Candidate, Capture, Normalization Attempt, Capture Fact, Session Identity, Session Projection, Transcript Message, Artifact, Activity Event
- Relevant ADRs: `0001-rust-library-with-tauri-shell`, `0002-captures-attempts-and-projections-are-distinct`, `0003-sqlite-and-content-addressed-files-are-library-internals`
- Prototype answer and source branch, if any: None

## Implementation

- Public interface used: Rust `Library` against a real temporary Distill home with real SQLite, content-addressed files, and the production `Fixture` SourceAdapter seam
- Behaviors covered: fresh home/migrations; Fixture detect/discover/snapshot/parse; verified Capture acceptance; Normalization Attempt and Capture Facts; atomic Session Projection and FTS; Activity; query; replay after source deletion; reopen; health; baseline path/root and size rejection; restrictive Unix modes
- `tdd` used: Yes — Library-seam integration tests in `crates/distill-library/tests/library_fixture_tracer.rs`
- Commands run during implementation:
  - `cargo fmt --all` — pass
  - `cargo clippy -p distill-library --all-targets -- -D warnings` — pass
  - `cargo test -p distill-library` — 5 passed (Fixture journey, synthetic identity, two configured-root escape cases, capture-size limit)
  - `npm run build` — pass
  - legacy tests excluding `db_inspector.test.js` — 93 passed
  - `npm test` — 93 passed, 10 unchanged DB-inspector tests fail because Node `v22.22.3` does not provide `DatabaseSync.setAuthorizer`; `src/**` and Node package files are unchanged from fixed point `b471a77`
- Library gate command: `node scripts/run-library-checks.mjs` (defaults to validated `library` mode); explicit `all` also runs the legacy npm suite and therefore reports the documented Node 22 inspector incompatibility

## Review

- Review fixed point: Pending
- Standards findings: Pending
- Spec findings: Pending
- Worthy fixes applied: Pre-commit local CodeRabbit findings applied: validated helper modes with Library-only default, and removed the Fixture manifest-ID fallback so omitted identities use deterministic synthetic provenance; the latter was reproduced red before the adapter fix and green afterward.
- Findings ignored with reasons: Pending

## Risks

- The staging baseline is TypeScript/Electron. This ticket adds the native Library beside it without copying the legacy schema or disrupting legacy tests; Electron retirement is reserved for the cutover ticket.
- Provider adapters, Sync Runs, Curation, Export, retry, and Tauri/React callers remain deferred (GAP-R002, GAP-R003).
- Capture acceptance and blob staging are not yet covered by fault-injection reopen contracts (reserved for later health/fault tickets).
- The legacy full-suite command requires a Node runtime that exposes `node:sqlite` `DatabaseSync.setAuthorizer`; the current Node `v22.22.3` baseline does not. This ticket does not change the legacy inspector.
