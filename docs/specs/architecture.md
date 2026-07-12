# Distill Architecture Spec

This document is normative.

## Product Scope

Distill is a local-first desktop application for collecting, normalizing, inspecting, curating, and exporting local LLM chat history that already exists on disk or can be captured locally.

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

- Distill is local-first. Source truth comes from local captures, not remote APIs.
- Connectors are thin. They detect, discover, snapshot, and parse. They do not make storage, search, or curation decisions.
- Canonical raw capture history is append-only.
- Every successfully snapshotted capture must be recoverable from Distill-owned storage.
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

A Distill-owned reference to recoverable raw content. The canonical type is:

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

An append-only audit event describing something meaningful that happened in Distill.

### `Job`

An operational unit of work used for source sync or future operational workflows. Jobs are not the canonical session or audit model.

## Rebuild Library Shape

The clean rebuild centers product behavior in one deep Rust `Library` crate (`crates/distill-library`). Desktop (Tauri), CLI, and contract tests are equal callers of that public seam. SQLite, FTS5, content-addressed files, migrations, and recovery protocols remain Library internals (see ADRs `0001`–`0003`).

Public Library methods for the Fixture tracer and thin callers:

- `Library::open(home)` — create or open a Distill home, apply ordered checksummed migrations, enforce restrictive Unix modes (`0o700` directories, `0o600` files), and perform safe open reconciliation that removes only canonical `{64 lowercase hex}.partial` staging files while reporting what was reconciled
- `Library::open_with_limits(home, max_capture_bytes)` — open with an explicit testable Capture acceptance limit
- `detect_fixture(fixture_root)` — return a caller-facing `SourceSummary` through the production Fixture SourceAdapter detect path
- `ingest_fixture(fixture_root)` — run the production `SourceAdapter` seam with the Fixture adapter only; the ingest report includes distinct `SessionIdentity` values projected during the run
- `set_registered_fixture_parser_version(version)` — advance the Library-owned Fixture parser to a strictly newer semantic version; callers cannot replace its parser identity
- `renormalize_capture(capture_id)` — re-run the registered Fixture parser against checksum-verified Distill-owned bytes without creating a new Capture
- `capture_attempts(capture_id)` — return immutable caller-safe Attempt summaries, including parser version, outcome, projection generation, diagnostics, and Fact count
- `run_fixture_journey(fixture_root, on_progress)` — first-run helper that detects, ingests, loads the first projected Session, and returns health as a `FixtureJourneyResult` with typed progress phases
- `session_slice` / bounded `search` — read a bounded slice of the current Session Projection and FTS index; cursor paging lands in issue #23
- `replay_capture(capture_id)` — return Distill-owned Capture bytes after checksum verification
- `health()` — report schema/migration integrity (including SQLite quick/integrity/foreign-key checks), referenced inline/blob size+checksum without following CAS symlinks or leaving the Distill home, exact projection↔FTS agreement across session_id/message_id/title/project_path/role/text, staging partials, unreferenced CAS blobs, incomplete Captures/Attempts/current projections (empty successful projections are valid), mismatched Session counters, and Sync Run operations status (`ok` / `active` / `failed`, including stale leases) as typed `HealthIssue` values with redacted summaries
- `list_sources` / `set_source_preference` — persist enabled/disabled and optional canonical configured-root overrides per Source without exposing adapter or storage internals
- `detect_sources` — return one independent typed detection result per requested Source (executable `None` for Fixture; effective data root; typed health/status); one failing Source never aborts siblings
- `start_sync` / `request_sync_cancel` / `sync_status` — durable Sync Run bookkeeping with typed progress, safe cancellation checkpoints before each Source and Capture Candidate (cancel requested at CandidateStarted still finishes that candidate; lease ownership is re-asserted after the progress callback before candidate work), `sync_already_running` / `sync_no_enabled_sources` / unknown-kind rejection with no side effects, owner/lease/heartbeat stale detection on open using system UTC only (no public injectable clock), background lease heartbeat for long candidates, and typed `sync_lease_lost` when ownership is lost
- `repair(options)` — explicit idempotent transactional repair for documented repairable states (orphan CAS deletion of in-root regular canonical blobs only, incomplete-state resolution via failed pending Attempts plus `capture_failed` recovery Activity without inventing Attempts, Session counter recompute, FTS rebuild from Session title/project_path, staging cleanup of canonical `{64 hex}.partial` only); never silently deletes referenced content or immutable Captures/Facts
- `recent_activity(limit)` — return a bounded first Activity slice for the tracer; cursor paging and operations views land in issue #30

Thin callers:

- `crates/distill-cli` — Fixture journey plus owning `health`, `repair`, `sources list|set`, and `sync start|status|cancel` commands; exit `0` success, `1` Library/runtime failure, `2` usage/validation
- `apps/distill-desktop` — Tauri 2 host runs journey/health/repair/sync off the UI thread via `spawn_blocking`, validates inputs, emits typed Fixture and Sync progress, and returns typed results to a sandboxed React renderer; repair requires explicit confirmation; renderer remains bridge-only

Test-only fault injection lives behind the non-default `test-faults` Cargo feature on `distill-library`. It is absent from production default API/behavior (including any message-prefix fault special cases) and interrupts real ingest boundaries (stage write, CAS rename, Capture/`capture_recorded` tx, post-accept Attempt, and mid-projection publication points). Mid-SQLite-transaction faults observe rollback rather than inventing impossible partial rows.

The legacy Electron application under `src/**` remains available until migration and cutover. It is not a dependency of the Rust Library.

## SourceAdapter Seam

Connectors use an internal Library `SourceAdapter` with exactly four operations: `detect`, `discover`, `snapshot`, and `parse`. The trait and provider-shaped values are not part of the public caller interface. Adapters never touch SQLite, Curation, search, exports, or Activity persistence. The Fixture adapter detects only an explicitly supplied root and is the first production adapter proven through the Library seam.
