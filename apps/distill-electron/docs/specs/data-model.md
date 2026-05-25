# Distill Electron Data Model Spec

This document is normative.

## Overview

The canonical Distill Electron data model has three layers:

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

`CaptureContentRef` identifies the Distill Electron-owned raw content for a canonical capture.

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
- `kind = "inline"` is for Distill Electron-owned content stored directly in the canonical capture record
- `kind = "blob"` is for Distill Electron-owned content stored in the Distill Electron blob area
- external URLs or source-owned references are not valid `CaptureContentRef` values

## `sources`

Purpose:

- store one row per supported source kind observed by Distill Electron
- preserve installation and local-root metadata

Canonical semantics:

- one logical row per `source_kind`
- mutable metadata is allowed
- source rows are not part of the append-only history
- current normative source kinds are `codex`, `claude_code`, `opencode`, and `droid`

## `captures`

Purpose:

- preserve every successfully snapshotted raw input
- support replay, auditing, and re-normalization

Canonical semantics:

- append-only
- a capture is created only after Distill Electron has raw content it can recover later
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

If a source does not provide a stable external session id, Distill Electron must synthesize one before projection materialization. The synthetic session id must be deterministic for the accepted capture and recorded in session metadata as synthetic provenance.

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

- one row per completed export artifact written by Distill Electron
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
