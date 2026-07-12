# Review Packet — #25 Recoverable Previewed Export

## Issue

- Issue: [#25](https://github.com/AustinKelsay/distill/issues/25)
- Slice type: AFK tracer bullet
- Acceptance criteria: shared train/holdout eligibility; explicit omission reasons; deterministic
  JSONL projection; same-volume atomic publication; durable lifecycle and restart reconciliation;
  checksum/database/file agreement; typed Library, CLI, Tauri, and React callers; cancellation
  coverage through each caller seam
- Baseline: `5359dd6`
- Implementation: `6e074f5`

## Implementation Evidence

- `implement` session: Grok 4.5 xhigh worker, independently audited and integrated by Codex
- `tdd` used: Yes — 8 Library export contracts, CLI/host cancellation and translation seams,
  bridge coverage, and renderer lifecycle/cancellation tests
- Library evidence: eligibility parity, deterministic JSONL metadata/labels/tags/messages/turn
  pairs, `unreviewed`/`favorite_only`/sensitive/exclude/conflict omissions, cancellation,
  temporary-write recovery, committed-before-rename recovery, rename-before-finalization checksum
  recovery, and finalization-transaction failure recovery
- Lifecycle evidence: migration `0004_exports.sql`; `preparing` → `committed` → same-volume rename
  → `published`; `export_written` is committed only with final bookkeeping; reopen verifies checksum
  and byte count before promoting a committed artifact
- Caller evidence: CLI `export preview|publish --dataset` with deterministic `--cancel`; Tauri
  preview/publish/cancel commands with sticky pre-worker cancellation and duplicate-run rejection;
  React preview/publish/cancel UI with explicit idle/running/success/error/cancelled states
- Gates: Rust fmt, Clippy warnings denied, all-features workspace tests, Library export suite (8),
  CLI integration (10), host integration (11), renderer/Vitest (19), frontend production build,
  optimized Tauri `--no-bundle`
- CodeRabbit: local review was attempted; its export-specific post-rename permissions finding was
  applied. Remaining reported findings were pre-existing baseline health/ingest/performance or
  stale-report issues outside this slice; the false-positive CLI finding was verified against the
  current source.

## Review Instructions

Review only this issue's slice unless a severe cross-slice regression is demonstrated. Keep
standards and spec findings separate.

Check:

- Preview and publish use one current-projection eligibility policy and expose blocked sessions
  with stable reasons, including `unreviewed` and `favorite_only`.
- JSONL ordering, projection metadata, manual labels-before-tags, message metadata, and turn-pair
  derivation are deterministic and covered by a golden fixture.
- Temporary writes, bookkeeping, rename, finalization, cancellation, and reopen reconciliation do
  not invent published paths or `export_written` events; checksum and byte-count verification gate
  recovery.
- CLI, Tauri, and React remain thin typed callers. Desktop cancellation is registered before the
  worker starts, sticky across the pre-worker race window, rejects duplicate home/dataset runs, and
  cleans up with an identity-checked guard.
- Canonical docs and the gap register describe the implemented Fixture export while leaving
  provider adapters and final packaging as the remaining rebuild work.

## Reviewer Output

Independent Grok xhigh standards/spec rereview initially found five findings: a pre-worker desktop
cancellation race, stale gap-register wording, missing `unreviewed`/`favorite_only` reasons,
mutable export rows incorrectly listed as append-only, and an unsupported-destination wording gap.
The first four were fixed in code/docs. The destination concern is resolved by the closed v1
caller contract: no arbitrary destination parameter exists, and unknown destination arguments fail
at the CLI/Tauri validation boundary before Library publication.

Final focused rereview:

```text
PASS after applying the cancellation, omission-policy, data-model, and gap-register fixes.
```

Known low-scope notes: the export module remains a large deep-module candidate and the React App
still owns several first-run panels; both are intentional follow-up architecture work, not blockers
for this bounded export contract.
