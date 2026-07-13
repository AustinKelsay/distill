# Review Packet — #42 CLI Multi-Source Thin-Caller Journey

## Issue

- Issue: [#42](https://github.com/AustinKelsay/distill/issues/42)
- Slice type: AFK public CLI caller contract
- Acceptance criteria: `docs/runs/issues/42-cli-multisource-thin-caller.md`
- Baseline: `aea5810`
- Current diff: working tree; commit pending

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
STANDARDS_STATUS: pending final remediation
STANDARDS_FINDINGS:
- Fixed the TCC-005 executable-symbol path to use path::symbol.
- Held TCC-004/TCC-005 and GAP-R003 at in-progress until this packet and final
  verification close; removed a dead happy-path secret-token assertion.
- The only remaining standards note is a judgement call: the single CLI test
  file grew by ~495 lines, which is acceptable for this bounded public seam.

SPEC_STATUS: pending final remediation
SPEC_FINDINGS:
- The CLI has no raw Capture replay command; the contract is explicitly scoped to
  post-removal projection queryability, while existing Library suites retain
  byte-exact replay evidence.
- Added the missing thin_cli_multisource_caller suite index entry.
- Held evidence statuses until final verification; non-goals remain explicit.
```

## CodeRabbit

- Pending local CodeRabbit attempt. If the service stalls or rate-limits, record
  the fallback and complete a fresh Grok standards/spec rereview.

## Residuals

- CLI raw Capture replay is not a public command; byte-exact provider replay is
  covered at the Library seam.
- Packaged real-provider journeys, assistive-technology speech, signing,
  Windows, production, and parser-registry redesign remain outside #42.
