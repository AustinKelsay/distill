# Distill Ingest Pipeline Spec

This document is normative.

## Scope

This spec defines the end-to-end behavior for importing local captures into Distill.

## Definitions

### `source kind`

The normalized connector identifier such as `codex`, `claude_code`, or `opencode`.

### `capture`

A successfully snapshotted unit of raw source content that Distill has accepted into canonical raw history.

### `canonical capture`

A `capture` that has recoverable raw content stored in Distill-owned storage and participates in canonical dedupe and replay behavior.

### `CaptureContentRef`

The Distill-owned reference to recoverable raw capture content. See [data-model.md](data-model.md) for the authoritative type definition.

### `session`

The logical conversation identified by `(source kind, external_session_id)` after normalization. If a source lacks a stable external session id, Distill must synthesize one before projection materialization.

### `capture records`

The parsed, source-shaped facts tied to one canonical capture version.

### `projection`

The current materialized session view used by queries, search, curation, and export. A projection includes the current session row, projected messages, and projected artifacts.

### `projected messages`

The ordered message rows in the current session projection.

### `projected artifacts`

The artifact rows associated with the current session projection.

### `session capture count`

The number of canonical captures that have been accepted for a logical session and recorded in the materialized session row.

### `implementation-specific mapping`

Implementation-local documentation that ties the canonical spec to a concrete codebase, such as `IMPLEMENTATION.md`, code comments, or a runtime config schema.

## Capture Status State Machine

The canonical capture state machine is:

```ts
type CaptureStatus = "captured" | "failed_parse" | "normalized";
```

Transitions:

- snapshot success -> `captured`
- `captured` + successful parse and projection replacement -> `normalized`
- `captured` + parse or normalization failure -> `failed_parse`

Rules:

- snapshot failures do not create canonical capture rows
- `normalized` is terminal for a specific capture version
- `failed_parse` is terminal for a specific capture version
- retries create a new canonical capture only when the new snapshot produces a different dedupe identity; otherwise the existing failed row may be updated only if a future implementation branch explicitly allows that behavior

## 1. Discovery

The importer must:

1. enumerate supported source connectors
2. detect each source independently
3. discover candidate captures independently per source
4. continue importing healthy sources when one source fails detection or discovery

Source detection and discovery failures are audit and operational events. They do not mutate existing projections.

## 2. Snapshot

Each discovered capture is snapshotted before any parsing work occurs.

Snapshot responsibilities:

- read or materialize the raw source content
- compute a stable SHA-256 checksum over the raw content
- compute byte size
- preserve source path and source timestamps when available
- write a Distill-owned `CaptureContentRef`

Canonical rule:

- only successfully snapshotted content becomes a canonical `capture`
- every canonical capture must have a mandatory source path; file-backed captures use the local path and virtual captures use a stable virtual source path

If snapshotting fails:

- no canonical capture row is created
- an audit event must record the failure
- source sync status must reflect the failure
- existing session projections remain unchanged

## 3. Raw Persistence

For every successful snapshot:

- raw content must be recoverable from Distill-owned storage
- the chosen `CaptureContentRef` must include checksum and byte size
- blob-backed content must live under Distill-managed storage

Current default storage policy:

- small text payloads may be stored inline
- larger or binary payloads should be stored as blobs

The exact size threshold is implementation-defined and must be documented in the implementation-specific mapping when introduced.

If Distill-owned raw persistence fails after raw content has been read:

- emit `capture_failed`
- leave existing projections unchanged
- continue importing later captures
- do not insert a new canonical capture row unless raw persistence succeeds

## 4. Dedupe

Exact capture dedupe occurs before a new canonical capture row is inserted.

The dedupe key is:

- source kind
- source path
- raw SHA-256

Because canonical captures require a source path, there is no pathless fallback dedupe rule in the current spec.

If an identical capture already exists:

- the importer skips capture insertion
- the importer does not mutate the session projection
- an operational summary may report the capture as skipped

## 5. Parse

Each connector parses the capture into:

- one normalized session payload
- zero or more parsed capture records
- zero or more normalized messages
- zero or more normalized artifacts

Connector output must be source-specific in parsing details and source-agnostic in emitted shapes.

## 6. Projection Update

The current session projection update is atomic.

Canonical transaction boundary:

1. insert parsed capture records for the accepted capture
2. upsert the session row
3. replace projected messages for the session
4. replace projected artifacts for the session
5. mark the capture as `normalized`
6. emit the projection audit event

Projection rule:

- projected session state is replace-on-success, never merge-on-failure
- `replace projected messages for the session` means full replacement: atomically delete all existing projected message rows for that session and insert the provided ordered set
- `replace projected artifacts for the session` means full replacement: atomically delete all existing projected artifact rows for that session and insert the provided set
- if the provided message or artifact set is empty, the replacement still deletes all previous rows for that projection slice
- replacement must happen in the same transaction as the session upsert and capture normalization state update

## 7. Failure Handling

If parsing or normalization fails after snapshot success:

- the canonical capture remains stored with status `failed_parse`
- partial projection writes must be rolled back
- the previous successful projection remains intact
- audit and operational records must capture the failure

## 8. Re-Import Behavior

Unchanged capture:

- exact duplicate
- skipped
- no projection mutation

Changed capture for an existing session:

- `changed` means the snapshot SHA-256 does not match any previously accepted canonical capture for the same source path
- append a new canonical capture version
- append its capture records
- replace the current session projection atomically
- increment the session capture count

New capture for a new session:

- insert a new canonical capture
- create the first session projection

## 9. Transaction Boundaries

Canonical transaction guidance:

- snapshotting and raw blob writes happen before the normalization transaction
- projection replacement happens inside a single transaction
- audit rows tied to projection success should commit with the projection transaction
- operational job bookkeeping may occur outside the projection transaction

## 10. What The Ingest Pipeline Does Not Do

The ingest pipeline does not:

- perform auto-tagging
- decide export policy
- expose source-specific parsing rules outside connector appendices
- use remote provider APIs as source truth

## Rebuild Ingest Notes

The Rust Library ingest path preserves the Electron-era invariants above and adds explicit Attempt bookkeeping:

1. SourceAdapter `detect` / `discover` / `snapshot` / `parse` run through the production seam (Fixture first).
2. File-backed candidates are rejected with typed `PathOutsideConfiguredRoot` when their canonical path escapes the configured Source root.
3. Capture size is bounded; exceeding the configured limit fails with typed `CaptureTooLarge` before acceptance.
4. Capture acceptance requires Distill-owned recoverable content (inline or CAS blob) and checksum verification; the Capture row and its `capture_recorded` Activity Event commit together.
5. A Normalization Attempt is recorded for parser execution. Capture Facts and Session Projection publication happen only for successful Attempts, in one transaction with FTS and `projection_replaced` Activity.
6. Failed Attempts never mutate the current Session Projection or FTS generation. They retain typed safe diagnostics (`parse_failed` or `projection_failed`) and, when a Session already exists, refresh only the separately named Capture and Attempt counters.
7. Exact duplicate snapshots are inert: no new Capture, Attempt, projection mutation, FTS change, or Activity Event.
8. Changed bytes for the same source path create a new immutable Capture and a new Attempt. A successful Attempt fully replaces the prior projection, including when the new message or artifact set is shorter or empty.
9. The same accepted Capture can be re-normalized from Distill-owned replay bytes by a newer version of the registered Fixture parser without creating a new Capture. Prior Attempts and Capture Facts remain immutable and observable through caller-safe Attempt summaries.
10. Replay and health read Distill-owned content only; deleting the original source must not break replay.
11. Interrupted ingest reopen contracts (issue #21) use the real transaction boundaries:
    - stage write before CAS rename → disposable canonical `staging/{64 lowercase hex}.partial` only; safe open removes it; noncanonical staging names are reported, never silently deleted
    - CAS rename before Capture DB acceptance → unreferenced blob; explicit repair may delete only in-root regular canonical orphans and never follows CAS symlinks or leaves the Distill home
    - Capture insert before `capture_recorded` inside the same SQLite transaction → full rollback (no durable Capture/Activity split)
    - after Capture+`capture_recorded` commit before Attempt → incomplete Capture; explicit repair appends Capture-scoped `capture_failed` with safe interruption context and does not invent a Normalization Attempt; later explicit renormalize may retry; exact duplicates remain inert
    - after pending Attempt before projection publication → pending Attempt remains; last-good projection preserved; repair fails the pending Attempt and recomputes Session counters
    - Fact/projection rows before FTS, after FTS before Attempt success/Activity, and after `projection_replaced` before commit → publication transaction rolls back; pending Attempt remains; last-good projection/FTS preserved
12. Test-only fault injection behind the non-default `test-faults` feature proves those reopen states via a typed cfg-gated fault variant. Production default builds do not expose fault controls or message-prefix fault special cases.

Legacy Electron still uses the `CaptureStatus` state machine documented above. The rebuild gap register tracks the dual-model period until Electron cutover.
