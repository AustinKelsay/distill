# Issue Session — #24 Transactional Curation and Workflow State

## Issue

- Issue: [#24](https://github.com/AustinKelsay/distill/issues/24)
- Fixed point before session: `2921523`
- Worker session: Grok 4.5 xhigh implementation + independent standards/spec rereview
- Commit: `5f6fd09`
- Status: Complete

## Intended Contracts

- Curation targets a Session Identity `(source_kind, external_session_id)` rather than exposing database ids to CLI, Tauri, or React callers.
- Tag and label names normalize to trimmed lower-case values. Blank, unknown, duplicate, and missing-session mutations are typed no-ops (`changed: false`) with no Activity side effects.
- Tags are manual session assignments. Labels are manual session assignments from the explicit local catalog (`train`, `holdout`, `exclude`, `sensitive`, `favorite`).
- Enabling a dataset label removes the other dataset labels in the same transaction, emits one `label_toggled` Activity Event for each removal and the new enable, and preserves orthogonal modifier labels.
- Every changed assignment and its Activity Event commit together. A returned curation state contains manual labels/tags with origins and the derived workflow state so callers update immediately without re-querying storage.
- CLI, Tauri host, bridge, and React remain typed callers; the renderer updates the selected list/detail row from the Library mutation result.

## Planned Evidence

- Library contracts cover normalization, missing/duplicate no-ops, dataset exclusivity, modifier preservation, workflow priority, manual origins, and Activity atomicity.
- CLI and Tauri host tests exercise typed tag/label commands and JSON results.
- React tests show immediate label/tag updates and manual-origin display without a second list/detail fetch.

## Review

Independent Grok standards/spec rereview: **PASS**. Low evidence notes were resolved with
stronger renderer no-refetch coverage, host remove-tag coverage, consistent CLI identity
validation, and non-manual label collision preservation. The review packet is
`docs/runs/reviews/24-transactional-curation.md`.

Final checks: Library curation 9; CLI/host 9 each; desktop bridge/renderer Vitest 16; Rust
fmt/clippy/workspace tests; desktop frontend build; optimized Tauri no-bundle build.

CodeRabbit pre-commit review was attempted but rate-limited by the service (13-minute wait), so
no CodeRabbit result was available for this slice.
