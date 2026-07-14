# Issue Session — #31 Legacy Electron Home Migration

## Issue

- Issue: [#31](https://github.com/AustinKelsay/distill/issues/31)
- Fixed point before session: `2692cac`
- Status: Complete
- Implementation commit: `f420b37`
- Review packet: `docs/runs/reviews/31-electron-migration.md`

## Intended Contracts

- `Library::import_legacy_electron_home` imports a legacy Electron home through a private read-only SQLite snapshot; the source directory, database, WAL/SHM sidecars, blobs, and exports remain byte-for-byte unchanged.
- Same, alias, ancestor/descendant, traversal, symlink, missing, and unsafe source/destination relationships are rejected or safely skipped with stable redacted reasons.
- Representative Sources, checksum-verified Captures, synthetic legacy Attempts, Capture Facts, generation-1 Session Projections/FTS, tags, labels, Activity-compatible events, and train/holdout export metadata map into the native model. Artifact links that the minimal target read model cannot preserve are explicit `artifact_links_unmapped` losses.
- Fingerprint markers make repeated imports idempotent. Import-owned CAS and export files are cleaned on rollback and newly-created skipped CAS is reclaimed without deleting pre-existing files.
- CLI `migrate`/`import-legacy`, Tauri host, and React first-run migration panel expose the same typed redacted report and explicit lifecycle states.

## Evidence

- `library_legacy_import` has 8 passing contracts covering representative mapping, replay checksum, search, curation, Activity/export redaction and bytes, WAL immutability, path rejection, marker idempotency, marker-less attempt/fact reuse, unsupported-source skip/count, pre-existing CAS preservation, and interrupted CAS cleanup.
- `cli_fixture_journey` covers JSON/human output, the `import-legacy` alias, usage/runtime exit codes, and legacy DB immutability. `host_legacy_import` covers typed Tauri validation and execution. `App.test.tsx` covers migration success/warning, error, and explicit cancellation states.
- Verification: `cargo fmt --all -- --check`; denied-warning workspace Clippy; `cargo test --workspace`; `cargo test -p distill-library --features test-faults`; desktop Vitest (26 passing), typecheck, lint, format check, Vite build, and optimized Tauri build.
- Independent Grok 4.5 xhigh rereview: PASS with no blocker or material correctness finding.
- CodeRabbit CLI: rate-limited (`waitTime=3 minutes`) before analysis; no actionable CodeRabbit findings were available in this run.
