# Distill Privacy And Capability Spec

This document is normative for the v1 hostile-input, diagnostic-redaction, and desktop-capability boundary.

## v1 Privacy Boundary

Distill is local-first and treats conversation content as local operator data. The `sensitive` label is an export-only policy: it blocks standard `train` and `holdout` publication and does not provide encryption, access control, or deletion.

Version 1 provides no application-level encryption at rest, per-session delete, retention purge, or secure-forget. The documented protection boundary is OS filesystem permissions, restrictive Distill-home modes, path containment, bounded provider processes, bounded input sizes, and redacted caller diagnostics. These omissions are intentional product scope, not implied capabilities.

## Hostile Input Contracts

All five v1 SourceAdapters (Fixture, Codex, Claude Code, OpenCode, and Droid) use the shared Library ingest boundary:

- file-backed candidates are checked against the configured Source root before snapshot; parent traversal and symlink escapes never become Captures
- discovery does not follow directory or file symlinks; an in-root symlink is skipped rather than treated as a second source of truth
- Capture bytes are bounded by the configured Library limit before a file-backed candidate is read into memory; virtual captures are bounded by the same storage path
- JSON documents and JSONL lines are UTF-8, byte-size, and nesting-depth bounded before provider projection
- malformed UTF-8, malformed JSON, deep JSON, oversized JSON, and provider parse failures become typed failed Attempts; they never publish a false Capture Projection
- provider subprocesses use the shared timeout, output, and stdin bounds in `docs/specs/activity-and-ops.md` and `docs/specs/connectors.md`
- HTML and script-looking text remains literal transcript data; the renderer does not render provider text as markup

## Diagnostic And Payload Redaction

Caller-facing Library, CLI, and Tauri errors use stable typed codes and safe messages. Raw filesystem paths, traversal strings, SQL, command arguments/output, provider payloads, credentials, and secret-shaped values are not included in caller diagnostics.

Activity and Operations read models redact path-bearing, SQL, command/output, provider/raw-payload, and secret-bearing fields recursively. Malformed payload JSON is represented as an empty object. Legacy import reports use the same redaction policy. Raw Capture bytes remain recoverable only through the Library-owned replay boundary and are never copied into Activity or operational logs.

## Desktop Capability Boundary

The packaged Tauri renderer receives only `core:event:default` in its default capability. No filesystem, shell, process, SQL, dialog, HTTP, or OS plugin permissions are granted. All registered commands remain explicit host functions; they validate path arguments (non-empty, no NUL, no parent traversal), closed enums, positive identifiers, page bounds, and curation identities before crossing into the Library.

The renderer bridge uses only Tauri `invoke` and `listen`. Host command arguments are validated in Rust, and progress events are emitted from typed Rust enums and consumed through typed bridge contracts. The bridge has no ambient filesystem, process, SQL, shell, or markup authority.

The macOS packaged smoke additionally inspects the built bundle and capability source,
then exercises the packaged renderer against a temporary home and Fixture root. The
journey records that the app writes the Library database/export only under the chosen
home, leaves the Fixture source unchanged, and preserves the export across a
quit/relaunch. This is runtime containment evidence for the local ad-hoc `.app`, not
an application-encryption, notarization, or secure-deletion claim.

The Linux CI package smoke installs the generated Debian package on Ubuntu, launches
the installed host under Xvfb/dbus, and applies the same capability-source, chosen-home,
Fixture-hash, and restart checks. AppImage creation is verified as an artifact; the
Debian install is the primary runtime proof. This remains containment evidence, not an
application-encryption, package-signing, or screen-reader claim.

## Required Evidence

The hostile-input contract is executable through:

- `crates/distill-library/tests/library_hostile_inputs.rs` for traversal, symlink policy, oversized Captures, deep/oversized JSON, malformed UTF-8, literal HTML/script content, secret redaction, and safe provider-bound errors
- `crates/distill-library/tests/library_opencode_source.rs` and `crates/distill-library/tests/library_ops_sync.rs` for OpenCode timeout/output bounds and large-stdin behavior
- `apps/distill-desktop/src-tauri/tests/host_hostile_inputs.rs` for typed host validation, safe Library error translation, and the least-privilege capability file
- `apps/distill-desktop/src/bridge.test.ts` for the invoke/listen-only renderer bridge and exact command payloads
- `apps/distill-desktop/scripts/macos-package-smoke.mjs` for the local macOS bundle capability, chosen-home containment, Fixture immutability, and restart artifact checks
- `apps/distill-desktop/scripts/linux-package-smoke.mjs` and `.github/workflows/linux-package-smoke.yml` for Ubuntu installed-host capability, chosen-home containment, Fixture immutability, and restart artifact checks

These contracts are privacy hardening, not a promise of application encryption or deletion semantics absent from v1.
