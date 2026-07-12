# Legacy Electron Home Migration

The native Rust `Library` owns the one-way, read-only import seam from a legacy Electron Distill home. The source is evidence, never a writable migration database: the importer must leave every source file byte-for-byte unchanged, including SQLite WAL sidecars and Distill-owned `blobs/` and `exports/` files.

## Public seam and callers

`Library::import_legacy_electron_home(source_home)` returns a typed `LegacyImportReport`. The thin CLI exposes `migrate` (with `import-legacy` as an alias), and the Tauri/React first-run surface calls the same method through a validated host request. Callers receive only redacted report fields and stable error classes.

## Snapshot and path safety

The source and destination homes must both be existing, non-symlink directories. Same paths, filesystem aliases, and ancestor/descendant relationships are rejected. Source content paths reject parent traversal, symlinks, non-regular files, and canonical paths outside the source home.

The importer fingerprints the database, any `distill.db-wal`/`distill.db-shm` sidecars, and safe in-home `blobs/`/`exports/` content. It copies the SQLite database and sidecars into a private destination staging snapshot and opens only that snapshot read-only with `query_only`. If the source fingerprint changes across snapshot creation, the import fails safely and can be retried after the Electron app is closed. The live source is never opened by SQLite.

## Mapping

The importer maps known Sources, immutable Captures and recoverable inline/CAS content, synthetic `legacy-electron-import` Normalization Attempts, Capture Facts, generation-1 Session Projections/FTS, tags, labels, session assignments, Activity-compatible events, and train/holdout export metadata with checksummed Library-owned copies. Existing native rows are never mutated; exact Capture keys and idempotent assignment keys are reused.

Legacy artifact message/fact links that cannot be represented by the minimal target read model are intentionally left nullable and are recorded as a documented fidelity loss. Export metadata is redacted before persistence. Activity payloads and report skip entries remove paths, SQL, command/output streams, provider/raw payload bodies, and path-like strings.

## Idempotency and interruption

The source fingerprint is a durable marker key in migration `0005`. Repeating an unchanged import returns the prior report with `reused_prior_import=true` and creates no new rows. Destination CAS and export files are created atomically; if destination SQL fails, only files created by that import and still unreferenced are removed. A successful import also removes any newly-created CAS file that was skipped because its Source kind was unsupported.

Report counters count rows created by this import (not descriptors merely reused). Skip categories include unsupported Sources/events/datasets, missing or unsafe content, checksum mismatches, missing sessions, unsafe export outputs, and documented projection-fidelity losses. No report field contains a source path, SQL statement, or raw payload.

## Contract evidence

The executable contract suite is `crates/distill-library/tests/library_legacy_import.rs`; CLI and Tauri host seams have corresponding tests. It covers representative mapping/search/curation/activity/export behavior, WAL-home immutability, path relationship rejection, same-fingerprint idempotency, redaction, and interrupted CAS cleanup.
