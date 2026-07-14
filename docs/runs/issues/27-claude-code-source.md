# Issue Session — #27 Claude Code Source

## Issue

- Issue: [#27](https://github.com/AustinKelsay/distill/issues/27)
- Fixed point before session: `b9f2913`
- Status: Complete
- Implementation commit: `6c44bc4`
- Review packet: `docs/runs/reviews/27-claude-code-source.md`

## Intended Contracts

- Claude Code is a concrete Rust `SourceAdapter` over a configured Claude home;
  detection verifies the root and `claude` executable while caller diagnostics remain generic.
- Discovery recursively finds project JSONL captures under `projects/`, excludes `history.jsonl`
  and `settings.json`, emits deterministic `claude://project/...` identities, and resolves duplicate
  session identities by deterministic source-path order.
- Snapshot reads exact source bytes with checksum and byte-size metadata. Shared ingest persists
  those bytes in Distill-owned content so replay survives source deletion.
- Parse preserves each source row as a Capture Fact, projects visible user/assistant text blocks,
  filters local-command/image-placeholder noise, and retains image, tool, result, file, reasoning,
  unknown, and metadata blocks as structured facts/artifacts. Session metadata, history titles,
  project path, branch, timestamps, and deterministic identity provenance are retained.
- Claude runs through generic Library detection, Sync checkpoints, Activity, CLI, and desktop
  surfaces. No adapter code writes SQLite or embeds caller-specific policy.

## Planned Evidence

- Adapter unit corpus covers project discovery, auxiliary exclusion, deterministic dedupe, exact
  snapshots, mixed blocks, suppressed noise, identity precedence, synthetic identities, and typed
  stage errors.
- Library integration covers generic detection, Sync progress/outcomes, projection/search,
  Activity, replay after deleting the Claude home, and unreadable-project discovery failure.
- Full workspace tests, formatting, and denied-warning Clippy remain green.

## Review

Independent Grok xhigh review initially found only governed-doc drift: the connector appendix, gap
register, feature ledger, and native contract matrix needed to record Claude as implemented. Those
docs and the unreadable-project Library contract are applied in the implementation and follow-up
documentation commits.
