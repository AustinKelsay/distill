# Scale And Latency Contract

This contract governs Library-only scale evidence for the rebuild. It deliberately does
not claim Tauri/React frame-time, packaging, or private-provider performance.

## Corpus targets

The synthetic benchmark corpus is deterministic and contains no private histories:

- at least 25,000 Session rows;
- at least 1,000,000 current-projection message rows;
- at least 10 GiB of logical Distill-home content, represented by benchmark-owned sparse
  padding when a physical allocation would make a scheduled run unsafe.

The generator seeds the real SQLite schema and current FTS projection in a temporary home
using a fixed seed. It may use bulk SQL for scale setup, but every measured operation goes
through the public `Library` API. No generated database, transcript, or padding file is
committed to the repository.

The search probe is deterministic but selective: it appears in the first message of every
97th synthetic Session. This exercises a representative paginated FTS search without
turning the budget into an all-match stress test over every message.

## Latency budgets

The full benchmark records cold and warm samples separately. Cold means the first measured
operation after opening a fresh `Library` handle; it does not claim an OS page-cache drop.
Warm means repeated operations on the same open handle after one discarded warm-up call.
The report records the host OS, architecture, machine string, corpus counts, logical home
size, sample count, p50, p95, and budget for every operation.

Warm p95 budgets on the recorded representative host are:

- first Session page: 150 ms;
- current-projection search page: 150 ms;
- first Session detail slice: 150 ms;
- one transactional manual curation mutation: 100 ms.

If a full scheduled run misses a budget, the report is an actionable failure containing
the operation, cold/warm class, p95, budget, corpus counts/size, and machine string.
The small pull-request smoke proves generator/API wiring only and does not claim the full
25k/1M/10 GiB budgets.

The current-session list order is backed by a checksummed partial expression index applied
by migration `0006_sessions_list_page_index.sql`; its `COALESCE(updated_at, '')` key is
intentionally identical to the public keyset ordering and cursor predicate.

## Progress and cancellation

While a Sync Run or export is advancing through multiple safe checkpoints, progress events
must be observed at least every 500 ms. Cancellation is acknowledged at the next safe
checkpoint within 1 second of the request; a currently executing candidate/transaction is
allowed to finish. The benchmark records the largest progress gap and cancellation
acknowledgement latency, with the checkpoint that produced each measurement.

The contract measures the public Library progress callbacks and durable terminal result.
It does not infer UI paint timing. If a future long single candidate can exceed the cadence,
the implementation must add a documented heartbeat event rather than silently claiming
that boundary events are wall-clock heartbeats.

## Execution policy

- The always-on test uses a small deterministic corpus and bounded sample count.
- The full corpus is an ignored, environment-gated test invoked with
  `DISTILL_SCALE_BENCH=1`; scheduled/manual jobs may opt into the logical 10 GiB target.
- Benchmark output is JSON on stdout/stderr and is reproducible from the fixed seed,
  reported hardware, and command line. Temporary homes are removed after each run.
- No Criterion/divan dependency or committed large artifact is required for this slice.
