# Review Packet — #26 Codex Source

## Issue

- Issue: [#26](https://github.com/AustinKelsay/distill/issues/26)
- Slice type: AFK tracer bullet
- Acceptance criteria: concrete Codex SourceAdapter; live/archive discovery with live duplicate
  precedence; exact bytes and source-independent replay; canonical dialogue/facts/artifacts;
  synthetic identity and typed stage errors; generic Library/Sync caller surfaces
- Baseline: `33f6f0a`
- Implementation: `bb6ffa7`

## Implementation Evidence

- `implement` session: Grok 4.5 xhigh worker, independently audited and integrated by Codex
- Adapter evidence: configured-root detection, executable availability classification, recursive
  live/archive discovery, metadata-id fallback, deterministic sorting, exact `fs::read` snapshots,
  checksum/size metadata, structured JSONL facts/artifacts/messages, instruction filtering, and
  deterministic synthetic provenance
- Library evidence: Codex uses `ingest_adapter_with_checkpoints` through the same Sync source,
  candidate progress carries logical `codex://` identities, Activity and projection/search remain
  generic, and replay succeeds after source removal
- Tests: adapter unit corpus (10), `library_codex_source` (3), existing Sync/Fixture suites, all-
  features Library tests, workspace formatting, and denied-warning Clippy
- CodeRabbit: local review returned actionable findings for executable matrix classification,
  traversal error propagation, metadata-id precedence, and bounded discovery peeking. All four
  were applied: the contract is now under `library_codex_source`, discovery returns typed errors,
  metadata ids win over filename hints, and the metadata peek uses a buffered 1 MiB limit. The
  service continued heartbeating for about eleven minutes after those findings and was terminated
  as a stale timeout; no additional result was returned.

## Review Instructions

Review only this issue's slice unless a severe cross-slice regression is demonstrated. Check:

- source-root and executable detection status are typed and caller-safe;
- discovery identity and live-over-archive precedence are deterministic, including metadata-only
  identities;
- raw rows remain replayable and structured provider records do not become provider-specific public
  fields or visible bootstrap transcript noise;
- Sync, Activity, projection, and thin callers reuse generic Library policy;
- canonical connector, architecture, ingest, gap, and contract-matrix docs describe Codex as
  implemented while leaving Claude Code/OpenCode/Droid deferred.

## Reviewer Output

Initial independent Grok xhigh review:

```text
FAIL — 2 high, 2 medium, and 2 low findings.
```

Remediation applied: governed docs and matrix rows were updated; missing `codex` executable now
produces typed `unavailable`; discovery peeks session metadata ids, sorts candidate files, and
filters skipped instruction noise from artifacts; stale OSR comments were corrected.

CodeRabbit remediation applied: the Codex contract row was mapped to the executable Rust suite,
discovery traversal now propagates filesystem failures as typed `discover` errors, session metadata
ids take precedence over filename hints, and discovery identity peeking is bounded to 1 MiB.

Final focused rereview:

```text
PASS from independent Grok rereview; CodeRabbit findings applied, final service attempt timed out.
```
