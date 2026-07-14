# Review Packet — #28 OpenCode Source

## Issue

- Issue: [#28](https://github.com/AustinKelsay/distill/issues/28)
- Slice type: AFK tracer bullet
- Acceptance criteria: bounded virtual discovery/export, exact stdout replay, canonical mixed-block
  parsing, typed timeout/exit/overflow/malformed handling, and generic Library caller reuse
- Baseline: `2ced5ef`
- Implementation: `cd491d6`

## Implementation Evidence

- `implement` session: Grok 4.5 xhigh worker, independently audited and integrated by Codex
- Adapter evidence: bounded shared subprocess execution, root-local harness/PATH resolution,
  stable virtual identities, full export stdout preservation, typed stage failures, metadata/title/
  project/model/timestamp handling, synthetic identity provenance, and structured artifact mapping
- Library evidence: OpenCode uses the shared `sync_adapter_source` path, generic Source/Candidate
  progress, Activity/projection/search, and replay from Distill-owned bytes after source removal
- Tests: adapter unit corpus (3), `library_opencode_source` (5), Codex/Claude/Sync suites, full
  workspace tests, formatting, workspace Clippy, and denied-warning all-features Clippy
- CodeRabbit: local review was attempted before commit and returned a service rate-limit response
  requiring a six-minute wait; no findings were produced.

## Review Instructions

Review only this issue's slice unless a severe cross-slice regression is demonstrated. Check:

- bounded command duration/output and child cleanup through the shared process policy;
- executable/export health classification and generic redacted diagnostics;
- deterministic virtual identities and progress;
- exact stdout preservation and replay after source removal;
- dialogue, tools/results, reasoning, files, unknown roles/parts, metadata, timestamps, and
  synthetic identity handling;
- generic Sync/Activity/projection/caller reuse and governed docs/matrix/ledger truthfulness.

## Reviewer Output

Initial independent Grok xhigh review:

```text
FAIL — stale governed docs, duplicated subprocess runner, and native evidence gaps.
```

Remediation applied: OpenCode now calls the shared bounded provider-process runner; adapter unit
coverage and integration assertions were expanded; connector/gap/ingest/matrix/ledger docs now
classify OpenCode as implemented and Droid as the only deferred provider. The final focused
rereviews found no material implementation or specification findings after these corrections.

Final focused rereview:

```text
PASS after shared-runner, native-evidence, and governed-document remediation.
```

CodeRabbit status: rate-limited before analysis; no review findings returned.
