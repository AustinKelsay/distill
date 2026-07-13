# Review Packet — #42 CLI Multi-Source Thin-Caller Journey

## Issue

- Issue: [#42](https://github.com/AustinKelsay/distill/issues/42)
- Slice type: AFK public CLI caller contract
- Acceptance criteria: `docs/runs/issues/42-cli-multisource-thin-caller.md`
- Baseline: `aea5810`
- Current diff: commit `736a388`

## Implementation Summary

The real `distill` binary now drives deterministic Codex, Claude Code, OpenCode,
and Droid roots through the same Source preference, Sync, current-projection
query, Curation, export, Activity, and Operations commands. Codex and OpenCode
remain queryable after their source roots are deleted; a mixed Fixture/unavailable
provider run proves warning isolation and redacted diagnostics. Exact byte replay
remains covered by the provider Library suites because the CLI has no raw Capture
replay command.

## Implementation Evidence

- `implement`/`tdd` seam: the public `distill` binary invoked by
  `crates/distill-cli/tests/cli_fixture_journey.rs`; no repositories, SQLite
  helpers, adapters, or host internals.
- Grok 4.5 xhigh bounded edit pass changed only the CLI integration test file;
  Codex integrated and reviewed the canonical docs.
- Targeted `cargo test -p distill-cli --test cli_fixture_journey -- --test-threads=1`:
  17 passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- None. Final Grok rereview confirmed the path::symbol citation, suite-index
  wiring, evidence statuses, redaction assertions, and CLI/projection boundary.
- Judgement only: the single CLI test file grew by ~495 lines, acceptable for
  this bounded public seam.

SPEC_STATUS: pass
SPEC_FINDINGS:
- None. Final Grok rereview confirmed all #42 acceptance criteria and explicit
  non-goals. CLI post-removal projection queryability is distinguished from the
  existing Library byte-exact replay contracts.
```

## CodeRabbit

- Local CodeRabbit attempt: stalled in summarization for about three minutes and
  was terminated without a report. Fresh Grok standards/spec rereview passed;
  this is the recorded fallback.

## Residuals

- CLI raw Capture replay is not a public command; byte-exact provider replay is
  covered at the Library seam.
- Packaged real-provider journeys, assistive-technology speech, signing,
  Windows, production, and parser-registry redesign remain outside #42.
