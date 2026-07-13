# Distill Desktop (rebuild)

Sandboxed Tauri 2 + React first-run Fixture caller. The renderer talks only through
an explicit typed bridge; it has no Node, filesystem, process, SQLite, or shell
authority. Product policy stays in `crates/distill-library`.

## Commands

From the repository root:

```bash
npm install
npm run desktop:typecheck
npm run desktop:lint
npm run desktop:format
npm run desktop:test
npm run desktop:frontend:build
npm run desktop:build
cargo test -p distill-desktop
cargo build -p distill-desktop
```

Development host (requires platform Tauri dependencies):

```bash
npm run desktop:dev
```

`desktop:build` proves the release host with `--no-bundle`. The macOS package gate uses
the workspace-installed Tauri CLI (the Cargo `tauri` subcommand is not required):

```bash
npm run desktop:package:macos
npm run desktop:smoke:macos
```

Ubuntu/Linux CI only:

```bash
npm run desktop:package:linux
npm run desktop:smoke:linux
```

The package identifier is `dev.distill.desktop`, the minimum macOS version is 12.0,
and the bundle target is an `.app` (DMG distribution is deferred). The checked-in green
icon is a placeholder product mark; `icon.icns` is generated from it and committed so
the package has a deterministic macOS icon.

The local package command intentionally uses `--no-sign`, producing an unsigned/ad-hoc
developer artifact. Developer ID signing, hardened-runtime entitlements, notarization,
and ticket stapling require Apple credentials and are documented release-only gates; no
local smoke result claims those properties. The default capability remains
`core:event:default` only: the renderer receives no filesystem, shell, process, SQL,
dialog, HTTP, or OS-plugin permission.

The macOS smoke script inspects the built `.app`, records bundle/signing metadata, and
can launch it for a short packaged UI journey when Accessibility automation is available.
It never substitutes CLI/host tests for packaged-renderer evidence. The journey scope is
Fixture sync, search, detail, one curation mutation, export, quit/relaunch, artifact
existence/write-containment checks, and packaged repair-dialog AX focus evidence: focus
enters `Confirm destructive repair`, Tab remains contained, Escape closes, and focus
returns to `Repair library`. Those checks are Accessibility focus state only. It does
not claim migration, crash recovery, privacy, scale, export-atomicity, VoiceOver speech,
or Developer ID/notarized signing; those remain their own contract gates and the human
accessibility checklist.

Linux packaging is intended to be built and smoke-tested on Ubuntu 24.04 in
`.github/workflows/linux-package-smoke.yml`. It emits `.deb` and AppImage artifacts;
the CI smoke is designed to verify the `.deb` `Depends` metadata, install it, and
launch the installed `/usr/bin/distill-desktop` under Xvfb/dbus with `xdotool`.
Build/runtime dependencies and the supported baseline are documented in
`apps/distill-desktop/docs/linux-runtime-deps.md`. Once #36 passes in Ubuntu CI, Linux package smoke will
be the runtime evidence for install, Fixture sync, search, detail, train curation,
export, restart, chosen-home/Fixture containment, and the repair-dialog focus/
cancellation contract; AT-SPI focus state does not claim screen-reader conformance, and
the slice does not claim migration,
crash-recovery, privacy, scale, or export-atomicity behavior.
