# Issue Session — #18 Library Fixture Tracer

This issue packet is historical evidence. Its old `src/**` fixture paths were
removed before beta; native coverage now lives under `crates/` and
`apps/distill-desktop/`.

## Issue

- Issue: [#18](https://github.com/AustinKelsay/distill/issues/18)
- Fixed point before session: `b471a77`
- Worker session: Grok 4.5 xhigh edit session
- Commit: `a13bf74`, review fixes `b87f5cb`
- Status: Complete — issue closed

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

- Review fixed point: initial `b471a77...a13bf74`; corrected `b471a77...b87f5cb`
- Standards findings: Changes requested — incomplete governed gap fields; two undocumented public methods; Fixture-coupled parser identity in generic ingest; magic pre-accept Capture id; duplicate size-limit enforcement
- Spec findings: Changes requested — Capture Fact provenance not asserted at the public seam; formatting helper mutated instead of checking
- Worthy fixes applied: All findings. Pre-commit CodeRabbit fixes validated helper modes and synthetic identity. Formal review fixes completed gap metadata, synchronized API docs, moved parser identity into `DiscoveredSource`, added a typed staged-integrity error, removed duplicate limit enforcement, verified adapter snapshot metadata before dedupe, asserted Capture Fact provenance, and made formatting check-only.
- Findings ignored with reasons: None
- Focused re-review: `STANDARDS_STATUS: pass`; `SPEC_STATUS: pass`

## Risks

- The staging baseline is TypeScript/Electron. This ticket adds the native Library beside it without copying the legacy schema or disrupting legacy tests; Electron retirement is reserved for the cutover ticket.
- Provider adapters, Sync Runs, Curation, Export, retry, and Tauri/React callers remain deferred (GAP-R002, GAP-R003).
- Capture acceptance and blob staging are not yet covered by fault-injection reopen contracts (reserved for later health/fault tickets).
- The legacy full-suite command requires a Node runtime that exposes `node:sqlite` `DatabaseSync.setAuthorizer`; the current Node `v22.22.3` baseline does not. This ticket does not change the legacy inspector.
