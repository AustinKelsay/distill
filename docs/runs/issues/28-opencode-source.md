# Issue Session — #28 OpenCode Source

## Issue

- Issue: [#28](https://github.com/AustinKelsay/distill/issues/28)
- Fixed point before session: `2ced5ef`
- Status: Complete
- Implementation commit: `cd491d6`
- Review packet: `docs/runs/reviews/28-opencode-source.md`

## Intended Contracts

- OpenCode is a virtual Rust `SourceAdapter` over a configured data root; executable and export
  failures remain typed at the adapter seam while caller diagnostics stay generic and redacted.
- Discovery runs a bounded `opencode db` query, emits deterministic `opencode://session/<id>`
  identities, and never writes SQLite or exposes absolute paths in progress.
- Snapshot preserves the complete bounded `opencode export <sessionId>` stdout payload—including
  any leading export text—as Distill-owned content before parsing, so replay works offline.
- Parse retains raw provider-shaped facts and derives canonical dialogue, reasoning, tool/result,
  file, unknown-role, unknown-structured, metadata, title, project, model, timestamp, and
  deterministic synthetic-identity outputs.
- OpenCode uses the shared Library Sync, Activity, CLI, and desktop caller surfaces without
  provider-specific policy leaking into those layers.

## Evidence

- Adapter unit corpus covers metadata and unknown-role preservation, structured artifacts,
  deterministic synthetic ids, invalid timestamps, malformed exports, and missing executables.
- Library integration covers generic detection, virtual progress, exact stdout replay after source
  removal, mixed parsing/search/activity, redacted command failures, timeout, overflow, and
  malformed export outcomes.
- The shared bounded provider-process runner owns duration, stdout/stderr caps, process-group
  cleanup, and safe subprocess diagnostics.
- Full workspace tests, formatting, and denied-warning Clippy are green.

## Review

Independent Grok xhigh review initially found stale governed docs, duplicated subprocess cleanup,
and missing native evidence. The implementation and documentation remediations were applied,
including shared process-runner reuse, adapter unit coverage, the OpenCode contract matrix, gap
register/ingest wording, and feature-ledger corrections. Final focused rereviews found no material
implementation or specification findings after those corrections.
