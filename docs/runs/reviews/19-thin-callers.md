# Review Packet — #19 Thin Native Fixture Callers

## Issue

- Issue: [#19](https://github.com/AustinKelsay/distill/issues/19)
- Slice type: AFK tracer bullet
- Acceptance criteria: CLI human/JSON/exit contract; async typed Tauri host; sandboxed React source/sync/session/health journey; documented passing Cargo/npm workspaces
- Baseline: `5655cde`
- Current diff: `git diff 5655cde...e9cd49a`

## Implementation Summary

The existing Fixture journey is now callable through a real Rust CLI and an async Tauri 2 host. A minimal React first-run renderer uses one explicit bridge, renders the journey states/results, and receives only typed progress and result data under an event-only capability file.

## Implementation Evidence

- `implement` session: Grok 4.5 xhigh worker, integrated by Codex
- `tdd` used: Yes — real CLI binary, host runner, React bridge fake, and production bridge translation
- Red test, if applicable: Library caller journey/identity result was introduced at its public seam before caller implementation
- Green implementation, if applicable: Library 6, CLI 4, host 4, renderer 5 tests pass
- Refactor, if applicable: none beyond the caller-oriented Library result types required by both CLI and host
- Commands run: Cargo fmt check, Clippy warnings denied, workspace tests/build, renderer typecheck/lint/format/test/build, Tauri release `--no-bundle`, legacy TypeScript build and 103 Node 26 tests

## Review Instructions

Review only this issue's slice unless you find a severe cross-slice regression. Keep standards and spec findings separate.

Check:

- Acceptance criteria are met.
- Tests verify behavior through public interfaces.
- No implementation-only tests are masquerading as behavior tests.
- No obvious incomplete work, TODO placeholders, or unrelated changes.
- Relevant test, typecheck, build, or visual verification commands pass.
- The renderer has no ambient filesystem, process, SQLite, shell, or Node authority.

Local CodeRabbit was attempted before commit and rate-limited for 18 minutes.

## Reviewer Output

```text
STANDARDS_STATUS: pass
STANDARDS_FINDINGS:
- No hard violations. Worthy judgements: centralize duplicate Library error-code maps and remove empty HostState. Caller-specific validation remains boundary-specific; cancellation is #22.

SPEC_STATUS: changes_requested
SPEC_FINDINGS:
- Reviewer claimed the event-only capability denies the registered app command.
```

Resolution:

- Centralized stable error codes on `LibraryError::code()` and removed empty `HostState`.
- Kept CLI/host validation local because usage errors and IPC validation are distinct caller contracts.
- Deferred cancellation to #22 as designed.
- Rejected the Spec finding: [official Tauri 2 capability documentation](https://v2.tauri.app/security/capabilities/) states that commands registered with `Builder::invoke_handler` are allowed for all app windows by default. This app does not opt into `AppManifest::commands`; `core:event:default` is the only renderer permission required for progress listening.

Focused re-review for `5655cde...e39c451`:

```text
STANDARDS_STATUS: pass
SPEC_STATUS: pass
```

The Spec reviewer withdrew the ACL finding after confirming `invoke_handler`, plain `tauri_build::build()`, and the absence of `AppManifest::commands`.
