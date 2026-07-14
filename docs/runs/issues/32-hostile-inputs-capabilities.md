# Issue Session — #32 Hostile Inputs And Desktop Capabilities

## Issue

- Issue: [#32](https://github.com/AustinKelsay/distill/issues/32)
- Fixed point before session: `f420b37`
- Status: Complete
- Review packet: `docs/runs/reviews/32-hostile-inputs-capabilities.md`

## Intended Contracts

- Fixture, Codex, Claude Code, OpenCode, and Droid hostile inputs are bounded and typed: traversal/symlink escapes, oversized Captures, deep/oversized JSON, malformed UTF-8, provider timeout/output bounds, and malformed exports do not create false Captures or projections.
- HTML/script-looking provider content remains literal transcript data.
- Activity, Operations, migration, CLI, and Tauri diagnostics redact filesystem paths, SQL, command/output streams, provider/raw payloads, and secret-shaped values.
- Tauri path/enumeration inputs are validated before Library work, the default capability grants only core events, and the renderer bridge uses only typed `invoke`/`listen` calls.
- `sensitive` is export-only. Version 1 provides no application encryption, per-session delete, retention purge, or secure-forget.

## Evidence

- `library_hostile_inputs` covers traversal, symlink policy, oversized captures, deep/oversized JSON, malformed UTF-8, literal script payloads, secret redaction, and safe provider-bound errors.
- Existing `library_opencode_source` and `library_ops_sync` cover provider timeout/output/large-stdin bounds and malformed exports.
- `host_hostile_inputs` covers Tauri path validation, safe Library error translation, and the events-only capability file; `bridge.test.ts` covers exact command payloads, listener race cleanup, and deny-list imports.
- CLI runtime failures now use `safe_caller_message`, matching Tauri host behavior.
- Verification: `cargo fmt --all -- --check`, denied-warning workspace Clippy, workspace tests, focused hostile/host/bridge tests, desktop typecheck/lint/format/build, and release Tauri build.
- Independent Grok 4.5 xhigh rereview: PASS with no blocker or material correctness gap. Follow-up risks (runtime bridge schema validation and in-root symlink omission policy) are documented as typed host/adapter policy rather than merge blockers.
