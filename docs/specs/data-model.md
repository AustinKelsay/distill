# Distill Data Model Spec

This document is normative.

## Overview

The canonical Distill data model has three layers:

1. append-only raw capture history
2. parsed capture facts
3. current materialized session projection

Operational and curation entities sit beside those layers and must not redefine them.

## Closed Enums

### `CaptureStatus`

```ts
type CaptureStatus = "captured" | "failed_parse" | "normalized";
```

Notes:

- Only successfully snapshotted inputs become canonical captures.
- Snapshot failures are audit and operational events, not canonical captures.

### `ActivityEventType`

```ts
type ActivityEventType =
  | "capture_recorded"
  | "capture_failed"
  | "projection_replaced"
  | "tag_added"
  | "tag_removed"
  | "label_toggled"
  | "export_written"
  | "sync_queued"
  | "sync_started"
  | "sync_completed"
  | "sync_failed";
```

### `CurationOrigin`

```ts
type CurationOrigin = "manual" | "auto_rule" | "model";
```

Current normative behavior uses `manual` only.

### `JobType`

```ts
type JobType = "sync_sources";
```

No other job types are normative until a new spec adds them.

### `MessageKind`

```ts
type MessageKind = "text" | "meta";
```

Meaning:

- `text`: user-visible transcript content intended for normal transcript rendering and export
- `meta`: visible but non-primary transcript content such as reasoning summaries, step markers, or structured trace text intentionally surfaced by a connector

## Shared Types

### `CaptureContentRef`

`CaptureContentRef` identifies the Distill-owned raw content for a canonical capture.

```ts
type CaptureContentRef =
  | {
      kind: "inline";
      mediaType: string;
      text: string;
      sha256: string;
      byteSize: number;
    }
  | {
      kind: "blob";
      mediaType: string;
      blobPath: string;
      sha256: string;
      byteSize: number;
    };
```

Rules:

- every canonical capture must resolve to exactly one `CaptureContentRef`
- `kind = "inline"` is for Distill-owned content stored directly in the canonical capture record
- `kind = "blob"` is for Distill-owned content stored in the Distill blob area
- external URLs or source-owned references are not valid `CaptureContentRef` values

## `sources`

Purpose:

- store one row per supported source kind observed by Distill
- preserve installation and local-root metadata

Canonical semantics:

- one logical row per `source_kind`
- mutable metadata is allowed
- source rows are not part of the append-only history

## `captures`

Purpose:

- preserve every successfully snapshotted raw input
- support replay, auditing, and re-normalization

Canonical semantics:

- append-only
- a capture is created only after Distill has raw content it can recover later
- exact duplicate capture snapshots may be skipped before insertion
- the dedupe key is `(source_kind, source_path, raw_sha256)`
- `source_path` is mandatory for canonical captures; file-backed captures use the local path and virtual captures use a stable virtual path such as `opencode://session/<id>`

Required fields:

- source identity
- mandatory source path or virtual capture path
- external session id when known
- source timestamps and size when known
- parser version
- `CaptureStatus`
- `CaptureContentRef`
- checksum and byte size

## `capture_records`

Purpose:

- persist parsed, source-shaped records for a specific capture version

Canonical semantics:

- tied to exactly one capture
- append-only through capture append behavior
- never shared across captures
- immutable after the capture is finalized

Required fields:

- line or ordinal within the source capture
- source record type
- source timestamps when known
- provider identifiers when known
- role and meta markers when known
- raw structured payload
- normalized free-text preview when practical

## `sessions`, `messages`, and `artifacts`

Purpose:

- store the current materialized session projection used by the UI, search, curation, and export flows

Canonical semantics:

- exactly one current session projection per `(source_kind, external_session_id)`
- a new successful import replaces the entire message and artifact projection for that session
- a failed import leaves the previous successful projection intact
- materialized rows are derived state, not immutable raw history

If a source does not provide a stable external session id, Distill must synthesize one before projection materialization. The synthetic session id must be deterministic for the accepted capture and recorded in session metadata as synthetic provenance.

### `sessions`

Required semantics:

- stable identity is `(source_kind, external_session_id)`
- contains the current session metadata for the latest successful projection
- `raw_capture_count` tracks how many canonical captures have been accepted for that session

### `messages`

Required semantics:

- ordered by projection ordinal
- `externalMessageId` is provenance when present, not the only identity rule
- ordinal order is part of the canonical projection contract
- role and `messageKind` determine transcript behavior

Canonical fallback identity for reasoning about duplicates across imports:

- session identity
- role
- text hash
- created timestamp

### `artifacts`

Required semantics:

- represent non-text or structured payloads associated with the current projection
- `message_id` should be set when an artifact belongs to a user-visible materialized message
- `capture_record_id` should be set whenever provenance exists, even if a `message_id` is also present
- both `message_id` and `capture_record_id` may be set when an artifact has both user-visible message association and capture provenance
- either field may be null when not applicable
- may carry `CaptureContentRef`-style blob references for large payloads

## `tags` and `tag_assignments`

Purpose:

- lightweight, reversible descriptors

Canonical semantics:

- tags describe characteristics, categories, or quick-filter terms without changing system behavior
- current normative assignments are session-level and manual
- every assignment must store origin as a `CurationOrigin` value and its assignment timestamp
- tags support grouping, filtering, and human-friendly categorization

Example:

- tag: `research`

## `labels` and `label_assignments`

Purpose:

- stronger curation states used to decide export or review behavior

Canonical semantics:

- labels decide export inclusion and review-routing behavior for a session
- current normative assignments are session-level and manual
- label toggling must be auditable
- `train`, `holdout`, and `exclude` are dataset labels and are mutually exclusive
- `sensitive` and `favorite` are orthogonal labels and may coexist with at most one dataset label
- dataset-label exclusivity must be enforced transactionally when a manual toggle enables a conflicting dataset label
- labels take precedence over tags when export or review behavior would otherwise conflict
- UI surfaces should present labels before tags, and export metadata should list labels before tags

Example:

- label: `train`

## `exports`

Purpose:

- bookkeeping for generated export artifacts

Canonical semantics:

- one row per completed export artifact written by Distill
- export rows describe operational output, not raw capture history

## `activity_events`

Purpose:

- canonical append-only audit trail

Canonical semantics:

- append-only
- captures user-visible and pipeline-significant events
- not limited to UI logs
- the audit trail must cover capture lifecycle, projection lifecycle, curation actions, export actions, and sync lifecycle

## `jobs`

Purpose:

- operational work scheduling and reporting

Canonical semantics:

- current normative use is `sync_sources` only
- jobs are allowed to track attempts, status, and scheduling metadata
- jobs do not replace canonical audit history

## `user_preferences`

Purpose:

- persist local UI preferences that are not part of the chat-domain model

Canonical semantics:

- local-only
- mutable
- not part of export contracts

## Append-Only vs Replace-On-Success

Append-only entities:

- `captures`
- `capture_records`
- `activity_events`
- `exports`

Replace-on-success projection entities:

- `sessions`
- `messages`
- `artifacts`

Mutable operational or preference entities:

- `sources`
- `jobs`
- `user_preferences`
- manual curation descriptors and assignments

## Current Implementation Mapping

The current SQLite schema is an implementation artifact in `schema.sql`. It is informative, not authoritative. Any gap between `schema.sql` and this document must be tracked in `docs/gaps/current-state-gap-register.md`.

## Rebuild Model: Captures, Attempts, Facts, Projection

The Rust Library rebuild separates four durable concepts that the legacy Electron `CaptureStatus` state machine collapsed:

1. **Capture** — immutable identity plus Distill-owned, checksum-verified content. Accepted only after verified ownership. Exact dedupe key remains `(source_kind, source_path, sha256)`.
2. **Normalization Attempt** — one parser identity/version execution against a Capture. Attempts record outcome, error classification, metrics, and the successful projection generation when applicable. The same Capture may have many Attempts across parser versions. Failed Attempts keep typed safe diagnostics and never rewrite prior Facts or the current projection.
3. **Capture Fact** — immutable provider-shaped record belonging to a successful Attempt. Facts are never rewritten; a newer parser creates a new Attempt and new Facts. Prior Attempt Fact counts remain observable through caller-safe summaries.
4. **Session Projection** — the latest successful normalized view for `(source_kind, external_session_id)`, including projected messages and artifacts. Replacement is atomic and generation-scoped. A failed Attempt leaves the prior current generation unchanged. Successful replacement is full replace even when the new message or artifact set is shorter or empty.

Rebuild schema entities (fresh Distill home; no legacy schema inclusion):

- `schema_migrations(version, checksum, applied_at)`
- `sources`
- `captures` (immutable content refs: inline or blob)
- `normalization_attempts`
- `capture_facts`
- `sessions` with separately named `accepted_capture_count`, `normalization_attempt_count`, and `successful_projection_generation`
- `projection_messages`, `projection_artifacts`
- FTS5 over the current projection (`projection_fts`)
- `activity_events`

Inline versus blob storage is an internal Library choice. The documented threshold is **64 KiB** (`INLINE_CONTENT_THRESHOLD_BYTES`). Larger Captures are staged, checksummed, atomically renamed into the content-addressed store, then accepted in SQLite.

Health and recovery classification over that durable state:

- migration/schema integrity via checksummed `schema_migrations` plus SQLite quick/integrity/foreign-key checks with stable redacted messages
- referenced Capture content presence, size, and checksum (inline or blob), never following CAS symlinks and never reading outside the Distill home even when `blob_path` is absolute or traverses parents
- exact current `projection_messages` ↔ `projection_fts` agreement across session_id, message_id, title, project_path, role, and text (not count-only)
- canonical disposable staging partials (`{64 lowercase hex}.partial`) and unrecognized staging entries under the Distill `staging/` directory
- unreferenced regular in-root canonical CAS blobs under `blobs/`, with symlinks/malformed tree entries reported as blocking rather than deletion candidates
- incomplete Captures (no Attempt and no Capture-scoped `capture_failed` recovery), pending Attempts, Sessions with broken `current_attempt_id`/generation linkage, and mismatched materialized Session counters
- empty successful projections are valid when linkage invariants hold
- `operations_status` is explicitly `not_applicable` until issue #22

Safe open reconciliation may remove only canonical `{64 lowercase hex}.partial` staging files and must report what it reconciled. Orphan CAS deletion (in-root regular canonical files only) and incomplete durable-state repair require an explicit `repair` call; repair is idempotent, transactional for related SQLite mutations, returns typed named actions/counts, appends `capture_failed` for Attempt-less interrupted Captures without inventing Attempts, recomputes Session counters, rebuilds FTS from Session title/project_path, and never silently deletes referenced content or immutable Captures/Facts.

Public Library read/write extensions for Attempt history and retry:

- `capture_attempts(capture_id)` returns immutable Attempt summaries with parser identity/version, outcome, typed error class/message, optional projection generation, and Fact count
- `renormalize_capture(capture_id)` re-runs the Library-registered Fixture parser against Distill-owned Capture bytes without accepting a new Capture or accepting caller-supplied arbitrary parser ids
- `set_registered_fixture_parser_version(version)` accepts only a strictly newer semantic version and advances only the registered Fixture parser used by ingest and renormalize
- `health()` / `repair(options)` own integrity classification and documented recovery; see architecture and ingest-pipeline rebuild notes
