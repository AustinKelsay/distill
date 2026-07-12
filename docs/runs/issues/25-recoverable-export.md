# Issue Session — #25 Recoverable Previewed Export

## Issue

- Issue: [#25](https://github.com/AustinKelsay/distill/issues/25)
- Fixed point before session: `5359dd6`
- Status: Complete
- Implementation commit: `6e074f5`
- Review packet: `docs/runs/reviews/25-recoverable-export.md`

## Intended Contracts

- The only v1 export format is `distill-session-jsonl-v1`, with dataset target `train` or
  `holdout`; the Library owns the destination under `<distill-home>/exports`.
- Preview and publish use the same current-projection eligibility policy: the target manual
  dataset label is required, while `exclude`, `sensitive`, and conflicting dataset labels block
  publication. Sessions with no dataset label are reported as `unreviewed`; `favorite` without a
  dataset label is reported as `favorite_only`; `favorite` alone is never sufficient.
- Each JSONL line contains current projection metadata, ordered messages with timestamps/kinds and
  parsed metadata, manual labels before manual tags, and deterministic `turn_pairs` using the
  canonical consecutive-user replacement algorithm.
- Publication is durable and recoverable: `preparing` -> `committed` -> same-volume atomic rename
  -> `published`, with checksum, byte count, record count, and `export_written` Activity only after
  the final path and bookkeeping agree. Cancellation and failed publication are terminal typed
  states with no misleading published path.
- Reopening a home reconciles incomplete export rows and disposable temporary files without
  inventing a successful Activity event or deleting referenced output. Preview has no filesystem,
  export-row, or Activity side effects.
- CLI, Tauri, and React are typed callers. They validate dataset/home inputs, expose preview and
  publish outcomes, support safe-checkpoint cancellation, and render explicit
  idle/running/success/error/cancelled states without reimplementing export eligibility. v1 has no
  caller-supplied destination; unsupported destination arguments fail at the closed caller
  boundary before Library publication.

## Planned Evidence

- Library contracts cover eligibility parity, golden JSONL ordering and turn pairs, metadata and
  manual-origin rules, checksums, lifecycle transitions, cancellation, restart reconciliation,
  and database/filesystem agreement.
- CLI and Tauri host tests exercise typed preview/publish/cancel translation and invalid dataset paths.
- React bridge/UI tests cover preview-to-publish flow, cancellation, and explicit lifecycle/error states.
- `test-faults` export boundaries cover temporary write, committed-before-rename, and rename-before
  published/Activity recovery points.

## Review

Independent Grok standards/spec review found and required fixes for caller cancellation race
ordering, gap-register drift, explicit unreviewed/favorite-only omission reasons, and mutable
export data-model classification. Final rereview passed after those fixes; all focused and full
workspace/build gates are recorded in the review packet.
