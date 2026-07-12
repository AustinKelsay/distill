# Review Packet — #34 Scale And Latency

## Review Scope

- Issue: #34
- Slice: deterministic scale corpus, Library latency budgets, progress cadence, and
  safe-checkpoint cancellation
- Baseline: `1dc5519`
- Implementation: `90c8dbc`

## Review Findings And Remediation

The first independent Grok review identified three evidence blockers:

1. the ignored full test could return success without `DISTILL_SCALE_BENCH=1`;
2. export progress cadence was not measured;
3. export cancellation occurred before any record had been written.

The test now fails when the full test is selected without its explicit environment gate,
records export Writing gaps across ten checkpoints, and cancels at `Writing:2` after two
records have been written.

The first full target run then exposed a real `list_sessions` order scan/sort (warm p95
265 ms). Migration 0006 adds the matching partial expression index. The next full run
exposed an all-match synthetic FTS probe (warm p95 2.328 s); the corpus now places the
probe in the first message of every 97th session, documented as a selective paginated
search workload rather than a pathological all-match stress test.

The progress/cancellation cases use a separate bounded Fixture/export home by design;
they measure safe-checkpoint cadence and acknowledgement at the public Library seam,
not full-corpus Sync/export throughput.

## Reviewer Output

Final independent Grok 4.5 xhigh rereview: PASS, with no blockers. The reviewer noted
only evidence-packaging wording, which is resolved above, and confirmed no private or
committed generated data.

## Verification Record

- Full target JSON and focused checks are recorded in `docs/runs/issues/34-scale-latency.md`.
- `cargo fmt --all -- --check` — pass.
- `cargo clippy --workspace --all-targets -- -D warnings` — pass.
- `cargo test --workspace` — pass.
- `cargo test -p distill-library --features test-faults` — pass.
- `cargo build -p distill-desktop --release` — pass.
- CodeRabbit CLI: attempted on uncommitted changes; service returned rate limit with a
  24-minute wait, so there were no findings to apply.
