# Issue Session — #29 Droid Source

## Issue

- Issue: [#29](https://github.com/AustinKelsay/distill/issues/29)
- Fixed point before session: `b426f23`
- Status: Complete
- Implementation commits: `3f3205a`, `315db4e`
- Review packet: `docs/runs/reviews/29-droid-source.md`

## Intended Contracts

- Droid is a file-backed Rust `SourceAdapter` over a configured root or the default
  `$HOME/.factory/sessions` root; it never opens SQLite or invokes a provider subprocess.
- Discovery recursively finds session `.jsonl` files, excludes `.settings.json` sidecars, emits
  deterministic `droid://session/<id>` identities, and keeps duplicate resolution stable.
- Identity precedence is `session_start.id`, filename stem, then deterministic synthetic identity.
- Snapshot preserves exact JSONL bytes, checksum, byte size, and source metadata so Distill-owned
  replay remains available after the source root is removed.
- Parse preserves raw Droid facts, visible user/assistant text, structured image/tool/thinking/file
  artifacts, unknown roles/blocks, sidecar metadata, owner/project values, titles, and timestamps.
- Detection and Sync reuse generic Library outcomes, progress, Activity, projection, and redacted
  diagnostics.

## Evidence

- Adapter unit corpus covers configured-root detection, recursive discovery, settings exclusion,
  duplicate first-wins behavior, session-start/stem/synthetic identity precedence, mixed blocks,
  sidecar metadata, invalid timestamps, malformed JSON/UTF-8, and exact snapshot hash/size.
- `library_droid_source` covers default and override roots, disabled/absent/unreadable diagnostics,
  generic Sync progress, mixed parsing/search/activity, sidecar metadata, malformed JSONL, and
  exact replay after source deletion.
- Existing Codex, Claude Code, OpenCode, and Sync contracts were updated only where their expected
  disabled-provider behavior needed to reflect the now-registered Droid adapter.
- Formatting, Library all-features tests, focused Droid tests, and denied-warning Clippy are green.

## Review

Independent Grok xhigh review initially found stale governed docs and several missing native
evidence edges. The implementation pass was retained; local remediation added the missing
identity/discovery/UTF-8/hash/sidecar coverage, preserved string blocks inside mixed arrays, and
updated the connector, ingest, gap, matrix, and feature-ledger docs. A focused rereview is recorded
in the review packet.
