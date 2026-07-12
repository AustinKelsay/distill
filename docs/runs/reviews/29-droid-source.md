# Review Packet — #29 Droid Source

## Issue

- Issue: [#29](https://github.com/AustinKelsay/distill/issues/29)
- Slice type: AFK tracer bullet
- Acceptance criteria: default/override-root detection, recursive file discovery, exact JSONL
  replay, canonical mixed-block parsing, typed malformed-input handling, and generic Library Sync
  reuse
- Baseline: `b426f23`
- Implementation: `3f3205a`; evidence hardening: `315db4e`

## Implementation Evidence

- `implement` session: Grok 4.5 xhigh worker, independently audited and integrated by Codex
- Adapter evidence: file-backed detection, default `$HOME/.factory/sessions` resolution, recursive
  deterministic candidates, settings sidecar exclusion, duplicate-id first-wins behavior, exact
  snapshot bytes/hash/size, typed parse stages, sidecar metadata, and synthetic provenance
- Library evidence: Droid uses the shared Sync adapter path, generic Source/Candidate progress,
  Activity/projection/search, typed warning outcomes, and replay from Distill-owned bytes after
  source removal
- Tests: adapter unit corpus (7), `library_droid_source` (6), Codex/Claude/OpenCode/Sync suites,
  Library all-features tests, formatting, and denied-warning Clippy
- CodeRabbit: local review was attempted before commit and returned a service rate-limit response
  requiring a 23-minute wait; no findings were produced.

## Review Instructions

Review only this issue's slice unless a severe cross-slice regression is demonstrated. Check:

- default/override/disabled/absent/unreadable root precedence and redaction;
- recursive deterministic discovery, sidecar exclusion, duplicate resolution, and logical paths;
- session-start, filename-stem, and synthetic identity precedence;
- exact snapshot bytes/hash/size and replay after source removal;
- mixed string/structured content, visible roles, unknown roles/blocks, artifacts, metadata, and
  invalid timestamps;
- malformed JSON/UTF-8 stage errors, generic Sync outcomes, progress redaction, and no SQLite or
  subprocess policy leakage;
- governed connector, ingest, gap-register, matrix, issue-session, and feature-ledger truthfulness.

## Reviewer Output

Initial independent Grok xhigh review:

```text
FAIL — governed docs still marked Droid deferred and native evidence was missing for duplicate
identity, session-start precedence, recursion, malformed UTF-8, exact snapshot hash, and library
sidecar metadata.
```

Remediation applied: native and Library evidence was expanded, mixed-array strings are preserved
as text blocks, the stale subprocess comment was corrected, and all governed Droid references were
updated to implemented with the new matrix rows and executable suite.

Final focused rereview:

```text
PASS — behavioral contracts and governed docs are aligned after evidence remediation.
```

CodeRabbit status: rate-limited before analysis; no review findings returned.
