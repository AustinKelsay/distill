# Issue Session — #34 Scale And Latency

## Issue

- Issue: [#34](https://github.com/AustinKelsay/distill/issues/34)
- Fixed point before session: `1dc5519`
- Implementation commit: `90c8dbc`
- Status: Complete
- Review packet: `docs/runs/reviews/34-scale-latency.md`

## Intended Contracts

- A fixed-seed temporary home reaches 25,000 Sessions, 1,000,000 current-projection
  messages, and at least 10 GiB logical content without private histories or committed
  generated artifacts. The size floor uses benchmark-owned sparse padding.
- The public `Library` API measures cold and warm first-page, FTS search-page, detail,
  and manual-curation operations with actionable p50/p95 JSON evidence.
- Sync and export progress callbacks remain within a 500 ms maximum observed gap across
  multiple safe checkpoints; cancellation reaches a durable cancelled result within 1 s.
- The full run is explicitly environment-gated; selecting it without
  `DISTILL_SCALE_BENCH=1` fails instead of silently producing a false green.

## Evidence

- `crates/distill-library/tests/library_scale_budgets.rs` provides the bounded smoke and
  ignored full benchmark. `tests/support/scale_corpus.rs` owns the deterministic seed,
  selective FTS probe, counts, logical sparse padding, machine string, and JSON helpers.
- Migration `crates/distill-library/migrations/0006_sessions_list_page_index.sql` adds a
  partial expression index matching the existing `COALESCE(updated_at, '')` list/cursor
  key. This removed the all-session scan/sort exposed by the first full run.
- Full command:
  `DISTILL_SCALE_BENCH=1 DISTILL_SCALE_MACHINE="$(uname -sm)" cargo test -p distill-library --test library_scale_budgets scale_full_corpus_latency_budgets -- --ignored --nocapture`
- Full recorded result on `Darwin arm64`:
  - corpus: 25,000 sessions, 1,000,000 messages, 10,737,418,240 logical bytes;
  - list page warm p95 1.066 ms / 150 ms budget;
  - search page warm p95 5.618 ms / 150 ms budget;
  - detail warm p95 0.113 ms / 150 ms budget;
  - curation warm p95 0.147 ms / 100 ms budget;
  - Sync max progress gap 3.189 ms / 500 ms budget;
  - export max progress gap 11.441 ms / 500 ms budget;
  - Sync cancel acknowledgement 1.778 ms / 1 s budget;
  - export cancellation at `Writing:2`, 0.196 ms / 1 s budget.
- The corpus/latency measurements use the full target home. Progress and cancellation
  use separate bounded Fixture/export homes so safe checkpoints can be exercised quickly;
  they are Library seam contracts, not claims that a full-corpus Sync/export was run.
- The ignored full test without `DISTILL_SCALE_BENCH=1` fails with the typed
  `DISTILL_SCALE_BENCH_required` panic rather than passing without evidence.

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy -p distill-library --test library_scale_budgets -- -D warnings`
- `cargo test -p distill-library --test library_scale_budgets -- --nocapture`
- Full env-gated benchmark above
- Independent Grok 4.5 xhigh review: initial FAIL findings fixed; final rereview PASS with
  no blockers.
- CodeRabbit CLI attempt returned a rate-limit response (`waitTime: 24 minutes`), so no
  findings were available; repository-wide Rust gates passed and are recorded below.
- `cargo fmt --all -- --check` — pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — pass.
- `cargo test --workspace` — pass.
- `cargo test -p distill-library --features test-faults` — pass.
- `cargo build -p distill-desktop --release` — pass.

## Remaining Scope

The benchmark remains a Library-only performance contract. Tauri/React frame time,
packaged WebView behavior, and other-hardware reproducibility remain #35/#36 or scheduled
benchmark concerns; this slice makes no UI or packaging claim.
