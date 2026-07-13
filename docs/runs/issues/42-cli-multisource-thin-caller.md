# Issue Session — #42 CLI Multi-Source Thin-Caller Journey

## Issue

- Issue: [#42](https://github.com/AustinKelsay/distill/issues/42)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: `aea5810`
- Worker session: Grok 4.5 xhigh bounded implementation pass; Codex integration
- Commit: pending
- Status: In progress
- Review packet: `docs/runs/reviews/42-cli-multisource-thin-caller.md`

## Intended Contract

The real `distill` binary is an equal thin caller of the public Library seam for
Codex, Claude Code, OpenCode, and Droid. One deterministic integration journey
must configure isolated synthetic Sources, run Sync, inspect the current Session
Projection, search, mutate Curation, preview and publish
`distill-session-jsonl-v1`, and read Activity and Operations through CLI JSON.

At least one file-backed Source and the OpenCode virtual Source must remain
queryable through CLI after the source root or fake executable output is removed.
The existing Library provider suites retain byte-exact replay evidence because
the CLI has no raw Capture replay command. Provider failures remain isolated and
diagnostics stay redacted. The contract does not claim packaged real-provider
behavior, assistive-technology speech, signing, Windows, production, or
parser-registry redesign.

## Testing Seam

- Primary seam: invoke the built `distill` binary with temporary homes and
  deterministic provider roots.
- Forbidden shortcuts: repositories, SQLite helpers, adapters, host internals,
  or private fixture persistence calls.
- Vertical slice: one CLI provider journey first, then the shared multi-source
  journey and canonical evidence updates.

## Verification Plan

- Targeted `distill-cli` integration tests for all four Sources.
- Rust workspace tests/format/tree checks and relevant Node/desktop gates.
- Two-axis Grok standards/spec review.
- Local CodeRabbit attempt with a fresh Grok fallback if the service is
  unavailable or stalls.
