# Distill Electron Connector Spec

This document is normative.

## Common Connector Interface

Every source connector exposes exactly four operations:

```ts
interface SourceConnector {
  kind: "codex" | "claude_code" | "opencode" | "droid";
  detect(): DiscoveredSource;
  discoverCaptures(): DiscoveredCapture[];
  snapshotCapture(capture: DiscoveredCapture): CaptureSnapshot;
  parseCapture(capture: DiscoveredCapture, snapshot: CaptureSnapshot): ParsedCapture;
}
```

The canonical method names match the current shared connector type and should not drift.

## Allowed Responsibilities

Connectors may:

- detect local installation and source roots
- discover source-specific captures
- read or materialize source-specific raw content
- parse source-specific formats
- map source records into canonical Distill Electron shapes

## Forbidden Responsibilities

Connectors must not:

- talk directly to SQLite
- mutate canonical projections
- define search indexing behavior
- define tag or label policy
- define export policy
- emit operational jobs directly
- bypass Distill Electron-owned raw capture preservation

## Common Parsing Expectations

All connectors must:

- preserve source provenance
- preserve enough raw structure to replay normalization later
- distinguish user-visible transcript content from source noise
- avoid leaking provider-specific policy into the shared ingest layer

## Codex Appendix

### Detection

The Codex connector verifies:

- the `codex` executable is available
- the Codex home directory exists
- live and archived session roots are discoverable when present

### Discovery

The canonical capture set is:

- live session JSONL files
- archived session JSONL files

Auxiliary metadata such as `session_index.jsonl` or `history.jsonl` is informative only.

### Snapshot

Snapshot source truth is the session JSONL file.

### Parse

Canonical transcript candidates:

- user messages
- assistant messages

Canonical non-transcript raw facts:

- reasoning records
- tool/function traffic
- token counters
- bootstrap context
- compaction records
- other provider-specific meta records

When both live and archived copies exist for the same external session id, the live capture is the authoritative current capture candidate.

## Claude Code Appendix

### Detection

The Claude connector verifies:

- the `claude` executable is available
- the Claude home directory exists
- project session roots are discoverable

### Discovery

The canonical capture set is project session JSONL files.

Auxiliary history files are informative only.

### Snapshot

Snapshot source truth is the project session JSONL file.

### Parse

Canonical transcript candidates:

- user text blocks
- assistant text blocks

Canonical structured artifacts:

- image blocks
- tool use blocks
- tool result blocks

Canonical non-transcript raw facts:

- queue operations
- progress records
- thinking blocks
- other meta-only records

## OpenCode Appendix

### Detection

The OpenCode connector verifies:

- the `opencode` executable is available
- the local OpenCode data roots are discoverable

### Discovery

The canonical capture set is one virtual capture per session returned by OpenCode session discovery.

### Snapshot

Snapshot source truth is the exported session payload materialized by `opencode export <sessionId>`.

### Parse

Canonical transcript candidates may include:

- text parts
- reasoning parts
- step-start parts
- step-finish parts
- tool parts when intentionally surfaced as meta transcript entries
- file parts when intentionally surfaced as meta transcript entries
- system-role messages when present

Canonical structured artifacts:

- tool calls and results
- file payloads
- unknown structured parts preserved as raw structured artifacts

## Droid Appendix

### Detection

The Droid connector verifies:

- the `droid` executable is available
- the Droid home directory exists
- the local session root is discoverable

### Discovery

The canonical capture set is local session JSONL files under the Droid sessions root.

Connector discovery must:

- include `*.jsonl` session transcripts
- exclude `*.settings.json` companion files

Auxiliary metadata such as `sessions-index.json` is informative only.

### Snapshot

Snapshot source truth is the session JSONL file.

### Parse

Canonical transcript candidates:

- user text blocks
- assistant text blocks
- user-only text notices may be surfaced as `meta` transcript entries

Canonical structured artifacts:

- tool use blocks
- tool result blocks
- thinking blocks preserved as raw structured artifacts

Canonical non-transcript raw facts:

- `session_start` records
- `todo_state` records
- `compaction_state` records
- system reminder blocks and other non-primary context records

## Adding A New Connector

A new connector may be added only when:

1. its source-of-truth capture format is documented
2. its parsing rules are added to this file
3. its contract tests are added to `docs/testing/contract-test-matrix.md`
4. any shared-shape changes are reflected in the canonical specs
