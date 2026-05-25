# Distill Electron Architecture Spec

This document is normative.

## Product Scope

Distill Electron is a local-first desktop application for collecting, normalizing, inspecting, curating, and exporting local LLM chat history that already exists on disk or can be captured locally.

The current canonical product loop is:

`discover -> snapshot -> preserve -> normalize -> search -> curate -> export`

SQLite remains a retained architectural constraint for the local data layer.

## Explicit Non-Goals

These items are not part of the current normative architecture:

- cloud sync
- multi-user collaboration
- browser extension capture
- provider OAuth or hosted APIs in the critical path
- embeddings or vector search
- auto-tagging in the critical path
- watched import folders
- a local capture API
- fine-tuning orchestration
- dataset versioning UI

Those may be added later, but only through new canonical specs.

## Five-Layer System Shape

The canonical architecture is split into five layers:

1. `source discovery`
2. `source connectors`
3. `ingest pipeline`
4. `local storage and projection`
5. `query, curation, export, and operations`

Responsibilities:

- Discovery decides which source installations and candidate captures exist.
- Connectors know source-specific file formats and transcript rules.
- The ingest pipeline owns snapshotting, raw preservation, dedupe, parsing, and projection updates.
- Local storage owns append-only raw capture history plus the current materialized session view.
- Query, curation, export, and operations only read or update standardized local entities.

## Core Invariants

- Distill Electron is local-first. Source truth comes from local captures, not remote APIs.
- Connectors are thin. They detect, discover, snapshot, and parse. They do not make storage, search, or curation decisions.
- Canonical raw capture history is append-only.
- Every successfully snapshotted capture must be recoverable from Distill Electron-owned storage.
- Parsed capture records are immutable facts tied to a specific capture version.
- The session view is a materialized projection. It is replace-on-success, never merge-on-failure.
- Search indexes are derived from the current materialized projection.
- Manual tags and manual labels are the only normative curation mechanisms in the current spec.
- Activity auditing and operational logs are separate concerns.
- Jobs are an operational mechanism, not a replacement for the canonical audit trail.

## Tech-Agnostic Terminology

### `DiscoveredSource`

An observed local installation or source root such as Codex, Claude Code, or OpenCode.

### `DiscoveredCapture`

A candidate source input that can be snapshotted and imported.

### `CaptureSnapshot`

The raw bytes or text obtained from a discovered capture at import time, along with checksum and size metadata.

### `CaptureContentRef`

A Distill Electron-owned reference to recoverable raw content. The canonical type is:

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

### `ParsedCaptureRecord`

A raw, provider-shaped fact produced by parsing a specific capture.

### `SessionProjection`

The current materialized session state for `(source_kind, external_session_id)`, including the current session row, ordered messages, and related artifacts.

### `ActivityEvent`

An append-only audit event describing something meaningful that happened in Distill Electron.

### `Job`

An operational unit of work used for source sync or future operational workflows. Jobs are not the canonical session or audit model.
