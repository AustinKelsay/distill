# Distill Electron Activity And Operations Spec

This document is normative.

## Overview

Distill Electron has two related but separate operational views:

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

Sync jobs may track:

- queued/running/completed/failed status
- warning status for non-fatal sync completions with warning details
- attempts
- scheduling metadata
- aggregated import metrics
- last error

Jobs are allowed to summarize work, but they are not the canonical audit model.

Warning-only sync state is operational only:

- it means the sync completed without a fatal job failure
- warning details and aggregate metrics remain visible in jobs/logs
- warning-only sync state must not by itself imply a canonical `sync_failed` audit event for the overall sync run

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
