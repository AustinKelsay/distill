# Legacy Electron source — retired

The pre-rebuild Electron/TypeScript product source was removed from the beta
workspace. It is preserved in Git history only and is not a runtime dependency,
build input, test runner, or release artifact.

The Rust Library still supports importing an Electron-shaped home as read-only
data. That compatibility contract is deliberately implemented and tested in the
Rust Library, CLI, Tauri host, and packaged hermetic fixtures; it does not require
the old Electron application to be present or executable.

Use these canonical surfaces for the retained compatibility contract:

- [Legacy migration spec](../../specs/legacy-migration.md)
- [Rust migration tests](../../../crates/distill-library/tests/library_legacy_import.rs)
- [Packaged legacy-home fixtures](../../../apps/distill-desktop/scripts/packaged-hermetic-legacy-home.mjs)

Historical matrix rows marked `legacy-baseline` remain only as provenance. They
are not beta product coverage and do not assert that Electron is installed.
