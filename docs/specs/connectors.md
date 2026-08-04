# Distill Connector Spec

This document is normative.

## Common Connector Interface

Every source connector exposes exactly four operations:

```ts
interface SourceConnector {
  kind: "fixture" | "codex" | "claude_code" | "opencode" | "droid" | "pi";
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
- map source records into canonical Distill shapes

## Forbidden Responsibilities

Connectors must not:

- talk directly to SQLite
- mutate canonical projections
- define search indexing behavior
- define tag or label policy
- define export policy
- emit operational jobs directly
- bypass Distill-owned raw capture preservation

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

The Droid connector resolves a configured root first, then the default
`$HOME/.factory/sessions` root when present. It reports an absent or unreadable root through
typed, caller-safe diagnostics and does not require a provider subprocess.

### Discovery

The canonical capture set is recursive `.jsonl` session files under the sessions root. Sidecar
`<session-id>.settings.json` files and other non-JSONL files are auxiliary metadata, not Captures.
Candidates use deterministic `droid://session/<id>` identities; duplicate external ids resolve by
sorted source path, and the adapter falls back from `session_start.id` to the filename stem and
then to a deterministic synthetic identity.

### Snapshot

Snapshot source truth is the exact session JSONL file bytes, including checksum, byte size, and
source modification metadata. Distill-owned Capture content remains sufficient for replay after
the Droid root is removed.

### Parse

Canonical transcript candidates are visible text blocks in user and assistant `message` rows.
Structured blocks become Capture Artifacts for images, tool calls/results, thinking, files, and
unknown block types. Session-start metadata, sidecar title/model/archive fields, owner, project
path, timestamps, unknown roles, malformed rows, and deterministic synthetic provenance remain
available as canonical facts or session metadata without provider policy leaking into shared code.

## Adding A New Connector

A new connector may be added only when:

1. its source-of-truth capture format is documented
2. its parsing rules are added to this file
3. its contract tests are added to `docs/testing/contract-test-matrix.md`
4. any shared-shape changes are reflected in the canonical specs

## Rebuild SourceAdapter Seam

The Rust Library implements connectors behind an internal `SourceAdapter` trait with the same four responsibilities:

```rust
trait SourceAdapter {
    fn detect(&self) -> Result<DiscoveredSource, SourceStageError>;
    fn discover(&self, source: &DiscoveredSource) -> Result<Vec<CaptureCandidate>, SourceStageError>;
    fn snapshot(&self, candidate: &CaptureCandidate) -> Result<CaptureSnapshot, SourceStageError>;
    fn parse(
        &self,
        candidate: &CaptureCandidate,
        snapshot: &CaptureSnapshot,
    ) -> Result<ParsedCapture, SourceStageError>;
}
```

`DiscoveredSource` also carries the adapter-owned parser identity and version recorded on each Normalization Attempt. This keeps the four-operation seam source-agnostic without hard-coding Fixture parser metadata in ingestion.

Adapters remain forbidden from SQLite, projection mutation, search, Curation, export, and Activity persistence.

### Fixture Appendix

- Detect only the root explicitly supplied by a test or packaged smoke harness (`distill.fixture.json` must be present).
- Discover file-backed and virtual Capture Candidates with deterministic logical identity (`fixture://...` paths).
- Snapshot through the same production preservation path; no test-only persistence shortcut.
- Parse synthetic Fixture JSONL covering dialogue messages plus structured tool/reasoning records as Capture Facts and Artifacts.
- Use explicit Fixture Session IDs when supplied; otherwise apply the production deterministic synthetic-identity rule.

### Pi Appendix

#### Detection

The Pi connector verifies:

- the `pi` executable is available (detection reports `unavailable` with
  `executable_not_found` when the executable is missing, without leaking provider
  text; the root is still validated when the executable is present)
- the Pi sessions root exists and is a directory

#### Discovery

The canonical capture set is Pi session JSONL files under the configured sessions root.

Files are organized by working directory under `sessions/--<encoded-cwd>--/` subdirectories with filenames following the pattern `<timestamp>_<uuid>.jsonl`.

#### Snapshot

Snapshot source truth is the session JSONL file.

#### Parse

Valid Pi session JSONL files begin with a `session` header line containing `id`, `version`, `timestamp`, and `cwd` fields. The header `id` is first-wins (only the first header line contributes the session identity), trimmed, and empty ids are rejected. Headerless files remain discoverable; the parser uses the documented fallbacks below.

Session identity is resolved in the following priority:

1. **Header id**: from the first `session` header line's `id` field (trimmed, non-empty).
2. **Filename stem**: the filename stem (directory-scoped by the relative path from the sessions root to prevent cross-directory collisions) when no header id is present.
3. **Synthetic**: a deterministic SHA-256 digest of the candidate `source_path` when neither header nor filename stem provides an identity.

Filename-stem and synthetic identities are recorded with `{"kind":"synthetic","strategy":"filename_stem"}` and `{"kind":"synthetic","strategy":"source_path_sha256"}` provenance respectively.

Canonical transcript candidates:

- user message entries (`"type": "message"`, `"message.role": "user"`)
- assistant message entries (`"type": "message"`, `"message.role": "assistant"`)

Canonical structured artifacts:

- image blocks
- tool_use blocks
- tool_result blocks
- file blocks
- other structured block types

Canonical non-transcript raw facts:

- compaction entries
- label entries
- the session header
- unknown entry types

The Rust rebuild now implements Fixture (#18), Codex (#26), Claude Code (#27), OpenCode (#28),
Droid (#29), and Pi through the same SourceAdapter and Library ingest seam. Codex detection requires the
configured home and `codex` executable, discovery peeks only for a session metadata id when
rollout filenames do not provide one, and archived candidates are folded before live candidates
so live wins deterministically. Claude Code detection requires the configured home and `claude`
executable, discovers project JSONL captures, and keeps history/settings auxiliary. OpenCode
detection requires a configured data root and executable, discovery uses a bounded `opencode db`
query for deterministic virtual session identities, and snapshot preserves the complete bounded
`opencode export` stdout payload before parsing. Droid is file-backed under its configured or
default sessions root and preserves exact JSONL bytes, sidecar metadata, and deterministic logical
identities. Pi is file-backed under its configured sessions root and discovers Pi session JSONL
files with deterministic identities resolved from the `session` header when available, using
the documented filename-stem or synthetic fallbacks otherwise. Exact
source bytes remain the snapshot truth for every file-backed adapter.
