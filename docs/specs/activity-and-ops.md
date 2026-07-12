# Distill Activity And Operations Spec

This document is normative.

## Overview

Distill has two related but separate operational views:

- `activity_events`: canonical append-only audit history
- operational logs and jobs: runtime status surfaces for the app

The audit trail is authoritative. Logs are a convenience surface.

## Activity Event Taxonomy

The canonical activity event taxonomy is:

- `capture_recorded`
- `capture_failed`
- `projection_replaced`
- `tag_added`
- `tag_removed`
- `label_toggled`
- `export_written`
- `sync_queued`
- `sync_started`
- `sync_completed`
- `sync_failed`

Every event should include:

- event type
- object type for the primary domain object
- object id when applicable
- session id when applicable
- timestamp
- structured payload for provenance and metrics

Canonical object-type guidance:

- capture lifecycle events use `object_type = "capture"`
- projection events use `object_type = "session"`
- curation events use `object_type = "session"`
- export events use `object_type = "export"`
- sync lifecycle events use `object_type = "sync_job"`

`structured payload` means a JSON object with enough fields to explain provenance and metrics for the event. Required payload content is event-specific, but sync events should include reason and aggregate counts when available, and failure events should include structured error context when available.

## Sync Job Model

Current normative job type:

- `sync_sources`

Rebuild Sync Runs are durable operational bookkeeping distinct from Activity. Status is the closed set `queued`, `running`, `completed`, `warning`, `failed`, `cancelled`. A partial unique index (or equivalent transaction-safe invariant) permits only one queued/running Sync Run per Distill home. Queue and canonical sync Activity transitions commit together where required. A second start returns typed `sync_already_running` and creates no Sync Run or Activity side effects. Unknown Source kinds and selections that resolve to zero enabled Sources return typed safe errors (`invalid_argument` / `sync_no_enabled_sources`) with zero Sync Run and Activity side effects.

Sync jobs may track:

- queued/running/completed/warning/failed/cancelled status
- warning status for non-fatal sync completions with warning details
- attempts
- owner/lease/heartbeat metadata for stale detection
- aggregated import metrics
- last error

Jobs are allowed to summarize work, but they are not the canonical audit model.

Warning-only sync state is operational only:

- it means the sync completed without a fatal job failure
- warning details and aggregate metrics remain visible in jobs/logs
- warning-only sync state must not by itself imply a canonical `sync_failed` audit event for the overall sync run
- warning Sync Runs still emit canonical `sync_completed` with warning metrics in the payload
- one successful Source/candidate plus one failed sibling terminates as `warning`; all-failing no-progress execution terminates as `failed`

Cancellation uses the closed Activity taxonomy:

- operational Sync Run status becomes `cancelled`
- the canonical Activity Event is `sync_failed` with structured `reason: "cancelled"` (there is no `sync_cancelled` event type)
- never treat operational logs as audit truth
- `request_sync_cancel` on a terminal run is explicitly idempotent (no status mutation)
- `sync_status(None)` with no runs returns typed `not_found`

Active Sync Runs carry an owner id plus heartbeat/lease. Production uses system UTC only; there is no public injectable Library clock. The deterministic stale threshold is 60 seconds without heartbeat renewal. A background lease heartbeat on a separate connection renews ownership for the run lifetime so long candidates are not falsely marked stale; it stops promptly on terminalization and never renews a terminal or ownership-lost run. Lease refresh and terminal updates require `id + owner_id + active status`; zero changed rows surface typed `sync_lease_lost` so a worker whose lease was failed on reopen cannot overwrite status/Activity or continue candidate work. On Library open, only stale active runs are idempotently failed with one `sync_failed` Activity each. Stale repair and normal owner terminalization are mutually exclusive. Health `operations_status` is `ok`, `active`, or `failed` (stale/inconsistent).

Every post-queue Sync execution path reaches exactly one terminal durable state and matching Activity, or returns `sync_lease_lost` after another opener already terminalized the run. Ordinary per-Source detect/discover/snapshot/config failures become failed Source outcomes with stable redacted diagnostics so sibling Sources continue. Internal fatal errors best-effort terminalize as `failed` without overwriting an already terminal/stale row.

## Logs Behavior

Operational logs may combine:

- sync job state
- export summaries
- other operational summaries added by future specs

Canonical rule:

- logs are an operational view
- logs may be derived or synthesized
- logs are not required to expose every audit event

## Relationship Between Jobs, Logs, And Activity

- Jobs describe operational execution.
- Logs present operational execution for the UI.
- Activity events describe what happened in the product domain and import lifecycle.

If jobs and activity disagree, activity is the authoritative domain audit and the gap must be treated as an implementation bug.

## Current In-Scope Operational Features

The current normative operational scope includes:

- source sync queueing and execution
- sync status summaries
- export summaries
- inspection of last run failures

The current normative operational scope does not require:

- a general-purpose background worker system
- distributed job processing
- job types for indexing or auto-tagging

## Library Health And Repair

Library health and repair are operational surfaces owned by the deep Library seam, not Activity Event rewrites:

- `health()` classifies schema/migration (checksums plus SQLite quick/integrity/foreign-key checks), referenced content, projection/FTS agreement across every searchable field, staging partials, unreferenced CAS blobs, incomplete Captures/Attempts/current projections, and Session counter drift
- empty successful projections are valid when `current_attempt_id` points at a succeeded Attempt for the claimed generation
- Captures interrupted before parser execution are incomplete until they have an Attempt or a Capture-scoped `capture_failed` recovery Activity; repair appends that Activity and never invents a Normalization Attempt
- user-facing diagnostics use typed issue codes and stable redacted summaries; raw filesystem paths, SQL, and payloads must not leak
- safe open reconciliation removes only canonical `{64 lowercase hex}.partial` staging files and reports counts; noncanonical staging entries are reported, never silently deleted
- CAS discovery/repair never follows symlinks and never reads or deletes outside the Distill home, even if a corrupted `blob_path` is absolute or traverses parents
- explicit `repair` is idempotent, uses transactions for related SQLite mutations, requires caller opt-in for destructive actions, and returns typed named action counts
- repair never deletes referenced content or mutates immutable Captures/Facts
- `operations_status` is `ok` / `active` / `failed` based on Sync Run lease health (issue #22). Export crash recovery remains issue #25.
