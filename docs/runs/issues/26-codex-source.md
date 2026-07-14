# Issue Session — #26 Codex Source

## Issue

- Issue: [#26](https://github.com/AustinKelsay/distill/issues/26)
- Fixed point before session: `33f6f0a`
- Status: Complete
- Implementation commit: `bb6ffa7`
- Review packet: `docs/runs/reviews/26-codex-source.md`

## Intended Contracts

- Codex is implemented as a concrete Rust `SourceAdapter` over a configured Codex home;
  detection verifies the root and the `codex` executable while keeping caller diagnostics generic.
- Discovery recursively finds live and archived session JSONL, ignores `session_index.jsonl` and
  `history.jsonl` as captures, derives provider Session Identity from rollout names or session
  metadata, and folds archived candidates before live candidates so live wins deterministically.
- Snapshot reads exact source bytes with checksum and byte-size metadata. The shared ingest path
  persists those bytes in Distill-owned content so replay survives source deletion.
- Parse preserves each raw provider row as a Capture Fact, projects visible user/assistant dialogue,
  filters bootstrap/developer instruction noise from transcript Messages, and retains tool,
  reasoning, metadata, and unknown-role records as structured facts/artifacts. Missing provider ids
  receive deterministic synthetic provenance.
- Codex runs through generic Library detection, Sync checkpoints, Activity, CLI, and desktop
  surfaces. No adapter code writes SQLite or embeds caller-specific policy.

## Planned Evidence

- Adapter unit corpus covers detection, deterministic live/archive and metadata-id dedupe, exact
  snapshots, structured parsing, synthetic identities, auxiliary-file exclusion, and typed stage
  errors.
- Library integration covers generic detection, Sync progress/outcomes, projection/search,
  Activity, live-over-archive behavior, and replay after removing the Codex home.
- Full Library/Sync/Fixture tests, workspace formatting, and denied-warning Clippy remain green.

## Review

Independent Grok xhigh standards/spec review initially found stale governed docs, missing executable
detection enforcement, metadata-only duplicate edge cases, instruction noise artifact leakage,
filesystem-order tie handling, and stale Sync comments. All findings were applied before the
implementation commit and are recorded in the review packet.
