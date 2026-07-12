# Distill Search, Curation, And Export Spec

This document is normative.

## Search

### Search Scope

Current canonical search is SQLite FTS over the current materialized session projection.

Indexed inputs:

- projected message text
- session title
- session project path
- message role for every projected message

### Search Semantics

- search results reflect the current projection only
- superseded projection rows must not appear in search results
- exact capture history is not searched directly
- punctuation-heavy user input must be normalized into a safe FTS query form

Current canonical normalization algorithm:

1. extract tokens using the Unicode pattern `[\p{L}\p{N}_-]+`
2. discard every character that is not part of a matched token, including quotes, slashes, commas, colons, and control characters
3. no additional quote-escaping step is required because the token matcher strips quotes before tokenization
4. wrap every token in double quotes
5. join wrapped tokens with ` AND `
6. if zero tokens are extracted, do not issue an FTS query and return no search results

Examples:

- input: `analytics-regression: "beta" env`
- normalized query: `"analytics-regression" AND "beta" AND "env"`

- input: `foo/bar baz?`
- normalized query: `"foo" AND "bar" AND "baz"`

- input: `("quoted") AND weird`
- normalized query: `"quoted" AND "AND" AND "weird"`

- input: `!!! /// ???`
- normalized query: no tokens extracted, so no FTS query is issued and no results are returned

### Session List And Detail Read Model

The query layer may expose list and detail read models, but those read models must derive from the current projection and manual curation state.

Session list must support:

- title
- source kind
- current manual labels
- derived workflow state for review and export readiness
- search results intersected with the active session workflow filter when the UI applies one

Session detail must support:

- title
- external session id
- project path
- source kind
- timestamps available from the current projection, including `started_at` and `updated_at` when present
- source URL when present in the current projection
- session summary when present in the current projection
- raw capture count from the current projection
- parsed session metadata from the current projection
- ordered messages
- manual labels
- manual tags
- artifacts

Current query read models must ignore non-manual label assignments even if future-origin rows exist in storage.

## Manual Tags

Current normative behavior:

- manual, session-level only
- stored with origin as a `CurationOrigin` value plus the assignment timestamp
- cheap and reversible

Current normative manual assignments use `origin = "manual"`.

Starter tag categories remain guidance, not a closed taxonomy:

- source tags
- model tags
- structural tags
- topic tags
- project tags

## Manual Labels

Current normative behavior:

- manual, session-level only
- stronger than tags because labels decide export inclusion and review-routing behavior, while tags remain descriptive only
- intended to drive export and review flows
- `train`, `holdout`, and `exclude` are mutually exclusive dataset labels
- `sensitive` and `favorite` are orthogonal labels that may coexist with at most one dataset label
- enabling one dataset label must remove any conflicting dataset label in the same transaction
- labels take precedence over tags in conflict resolution
- UI surfaces should show labels before tags
- export metadata should list labels before tags

Starter label set:

- `train`
- `holdout`
- `exclude`
- `sensitive`
- `favorite`

The label catalog may expand, but every label remains explicit and local.

Current canonical workflow interpretation:

- `train`: approved for train export unless blocked by `sensitive`
- `holdout`: approved for holdout export unless blocked by `sensitive`
- `exclude`: review-only, never included in standard dataset export
- `sensitive`: review-only modifier, blocks standard dataset export
- `favorite`: bookmark-only, never an export target by itself
- conflicting dataset labels such as both `train` and `holdout` are invalid state and must be treated as `needs_review` until manual relabeling resolves the conflict

Current canonical session workflow states are:

- `needs_review`: session has `exclude`, `sensitive`, or conflicting dataset labels such as both `train` and `holdout`
- `train_ready`: session has `train` and does not have `exclude` or `sensitive`
- `holdout_ready`: session has `holdout` and does not have `exclude` or `sensitive`
- `favorite`: session has `favorite` and is not in another higher-priority workflow state
- `neutral`: session has no review or export-driving labels

Workflow state priority is:

1. `needs_review`
2. `train_ready`
3. `holdout_ready`
4. `favorite`
5. `neutral`

Current canonical Sessions filter lanes are:

- `All`
- `Needs Review`
- `Train Ready`
- `Holdout Ready`
- `Favorites`

Lane semantics:

- `Needs Review` contains sessions with `exclude`, `sensitive`, or conflicting dataset labels
- `Train Ready` contains sessions with workflow state `train_ready`
- `Holdout Ready` contains sessions with workflow state `holdout_ready`
- `Favorites` contains sessions with label `favorite`
- unlabeled sessions remain visible in `All` only in the current MVP branch

The `sensitive` label is an export-only policy boundary. It does not encrypt
content, delete a Session, purge retained history, or provide secure-forget;
v1 intentionally provides none of those application-level controls. The
hostile-input and capability rules that protect caller diagnostics and the
renderer boundary are specified in `docs/specs/privacy-and-capabilities.md`.

## Export Contract

Current canonical export behavior is approved dataset export from the current materialized projection.

The v1 published format is `distill-session-jsonl-v1`: one deterministic JSON object per line. Preview
and publish share the same eligibility snapshot and policy; preview performs no filesystem, export-row,
or Activity mutation. The Library owns `<distill-home>/exports`, and callers do not select arbitrary
destinations in v1.

Current approved dataset targets are:

- `train`
- `holdout`

Dataset export eligibility rules:

- a session is eligible for `train` export only when it has label `train` and does not have `exclude` or `sensitive`
- a session is eligible for `holdout` export only when it has label `holdout` and does not have `exclude` or `sensitive`
- sessions with conflicting dataset labels such as both `train` and `holdout` are invalid and must not appear in standard dataset export until relabeled
- sessions with `exclude` are review-only and must not appear in standard dataset export
- sessions with `sensitive` are review-only and must not appear in standard dataset export
- `favorite` never makes a session exportable by itself

Required export content:

- source kind
- external session id
- session metadata from the current projection, including `source_url`, `summary`, and parsed session metadata when present
- ordered projected messages with ordinal, role, text, created timestamp, `message_kind`, and parsed message metadata
- manual labels
- manual tags
- turn-pair representation when derivable

Current canonical turn-pair representation is:

```ts
type TurnPair = {
  user: string;
  assistant: string;
};
```

Derivation algorithm:

1. iterate projected messages in ordinal order
2. when a `user` message is encountered, store it as the pending user value
3. if another `user` message appears before a non-meta `assistant`, replace the pending user value with the newer one
4. when an `assistant` message with `message_kind != "meta"` appears while a pending user value exists, emit one `{ user, assistant }` pair and clear the pending user value
5. `assistant` messages with `message_kind = "meta"` and `assistant` messages without a pending user value do not create a pair
6. a trailing `user` message without a following `assistant` does not create a pair

This intentionally mirrors the current implementation rather than a richer future pairing model.

Export source truth is the current session projection, not raw capture history.

For compatibility, additive export fields may be introduced without renaming existing top-level fields, but any exported session or message metadata must still come from the current projection only.

Publication lifecycle is durable and restart-aware:

1. create an `exports` row in `preparing` state and write a temporary file under the Library-owned
   `exports` directory;
2. flush the temporary file, compute its SHA-256 and byte/record counts, and move the row to
   `committed`;
3. atomically rename the temporary file to its same-volume final path;
4. move the row to `published` and append one `export_written` Activity Event in the same SQLite
   transaction.

Cancellation or publication failure is terminal (`cancelled` or `failed_publish`) and must not
report a final path unless the file and bookkeeping agree. Reopening the Library reconciles
incomplete rows and removes only disposable temporary files; it never invents `export_written`
for an unrecovered artifact.

## Explicit Out-Of-Scope Items

The following are not normative in the current spec:

- auto-tagging
- auto-labeling
- embeddings or semantic search
- multi-label review workflows beyond manual toggles
- message-level labels
- dataset versioning
