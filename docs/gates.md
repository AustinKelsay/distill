# Distill Verification Gates

Canonical beta verification commands for the Rust Library, thin callers, desktop
host, release metadata, and packaged artifacts.

Run from the repository root with Node 22+ and the stable Rust toolchain.

## Core Rust gates

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p distill-library --features test-faults
cargo build --workspace
```

### Continuous PR enforcement (#46)

`.github/workflows/rebuild-ci.yml` is the authoritative Ubuntu CI evidence for the
core rebuild commands enforced on every qualifying pull request into `staging`
(and on `workflow_dispatch`):

- `cargo fmt --all -- --check`
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`
- `cargo test -p distill-library --features test-faults`
- `cargo test -p distill-library --test library_ops_sync --features test-leases`
- `npm ci`
- `npm run desktop:typecheck`
- `npm run desktop:lint`
- `npm run desktop:format`
- `npm run desktop:test`
- `npm run desktop:frontend:build`

The workflow uses the same `dtolnay/rust-toolchain@stable` and
`actions/setup-node@v4` (Node 22, npm cache) conventions as the Linux package
smoke and Rust advisory workflows, with `contents: read` only and bounded job
timeouts. It does **not** run real provider roots, the full-scale
`DISTILL_SCALE_BENCH` corpus, package signing/notarization, Windows jobs, or
human assistive-technology observation. Local Darwin runs of the same commands
remain useful developer feedback; they are not a substitute for the Ubuntu
Actions result once a run ID is recorded.

Authoritative Actions evidence for the implementation head `38d3c7a`:
[Distill rebuild CI run 29224511931](https://github.com/AustinKelsay/distill/actions/runs/29224511931)
— green Rust and desktop jobs. Docs-only follow-up commits do not change the
workflow behavior represented by this implementation-head run.

## Sync / Source settings gates (#22)

```bash
cargo test -p distill-library --test library_ops_sync --features test-leases
cargo test -p distill-cli --test cli_fixture_journey
cargo test -p distill-desktop --test host_fixture_journey
npm run desktop:test
```

Provider subprocess duration bounds and large-stdin cleanup are covered on macOS/Linux. Output-byte caps are covered on all platforms via a deterministic helper without spawning. Heartbeat accuracy uses the non-default `test-leases` feature only. The `test-leases` Sync suite is included in the continuous `#46` rebuild CI gate above; CLI/host fixture journeys remain separate caller evidence.

## Desktop gates

```bash
npm run desktop:typecheck
npm run desktop:lint
npm run desktop:format
npm run desktop:test
npm run desktop:frontend:build
cargo build -p distill-desktop
# release host proof without packaging:
cargo tauri build --no-bundle
# or npm run desktop:build when configured
npm run desktop:package:macos
npm run desktop:smoke:macos
# Ubuntu CI only:
npm run desktop:package:linux
npm run desktop:smoke:linux
npm --prefix apps/distill-desktop run test:a11y
npm --prefix apps/distill-desktop run a11y:smoke
cargo test -p distill-library --test library_scale_budgets
# scheduled/manual full corpus (logical 10 GiB padding; never a default PR gate):
DISTILL_SCALE_BENCH=1 cargo test -p distill-library --test library_scale_budgets -- --ignored --nocapture
```

Renderer typecheck/lint/format/test/frontend-build are part of the continuous
`#46` rebuild CI gate. Packaging, installed-host smoke, a11y packaged focus,
and scale budgets remain separate gates below and are **not** claimed by
`rebuild-ci.yml`.

The a11y smoke is a post-build renderer check. It does not claim signed packaged
WebView or screen-reader coverage; the human checklist at
`apps/distill-desktop/docs/a11y-human-checklist.md` records assistive technology
observations. On macOS, `desktop:package:macos` builds an `.app` through
the workspace-installed Tauri CLI with `--no-sign`, and `desktop:smoke:macos` proves
the local ad-hoc bundle metadata, restricted capability source, Fixture sync,
search/detail/curation/export journey, quit/relaunch, artifact persistence, and
write containment. It also proves packaged AX focus enters `Confirm destructive repair`,
Tab remains contained, Escape closes, and focus returns to `Repair library`; this is
Accessibility focus-state evidence only. Issue #48 extends the same harness with
temporary hermetic Codex/Claude/OpenCode/Droid roots, Detect Sources sibling-failure
isolation/redaction, and Start Sync Run before the retained Fixture journey
(`PKG-004`/`PKG-005`). OpenCode uses a local `{root}/bin/opencode` stub only.
The Linux packaged journey also drives the existing bridge-only Attempt-history and
same-Capture renormalize controls (`LPKG-006`); Darwin `PKG-006` remains manual-required
when System Events cannot expose the packaged window. It does not claim
Developer ID signing, hardened runtime, notarization, host-installed providers,
migration, crash recovery, privacy, scale, export atomicity, or VoiceOver coverage.
The Cargo `tauri` subcommand is not required for this package
gate and is unavailable in the recorded environment.

Linux packaging is a Linux-only CI gate in `.github/workflows/linux-package-smoke.yml`.
It installs Ubuntu WebKitGTK/GTK, Xvfb, dbus, AT-SPI, and `xdotool` dependencies,
builds `.deb` and AppImage artifacts, verifies the Debian `Depends` metadata, installs
the `.deb`, and drives the installed `/usr/bin/distill-desktop` under Ubuntu's
`dbus-run-session`/Xvfb environment. The smoke verifies the installed control tree and
checks the same
Fixture/search/detail/Attempt-history/renormalize/train-curation/export/restart/artifact/
containment path as macOS, plus the #48 hermetic multi-Source Detect/Start Sync path
(`LPKG-004`/`LPKG-005`) and the packaged retry path (`LPKG-006`).
It does not claim screen-reader, host-installed provider, migration, crash-recovery,
privacy, scale, or export atomicity coverage.

Scale reports are Library-only JSON evidence. The default test is a bounded synthetic
smoke; the 25k Session / 1M message / 10 GiB logical-home run is environment-gated and
must record hardware, cold/warm samples, p50/p95, progress gaps, and cancel acknowledgement.

## Beta metadata and documentation gates

```bash
npm run check:docs
npm run release:check
```

`check:docs` validates the canonical docs package, matrix/registry parity,
active executable paths, and the explicit retired-Electron boundary. It replaces
the historical `src/test/docs.test.ts` check. `release:check` validates that all
workspace/package/Tauri versions match the beta version and that release files
are present.

## Security and dependency gates

These are the repository-available dependency gates for the rebuild cutover:

```bash
cargo tree --workspace --locked
npm audit --audit-level=moderate --ignore-scripts
```

`cargo tree --locked` proves the Rust workspace resolves from the checked-in lockfile;
`npm audit` is the JavaScript advisory scan for the active desktop workspace.

### RustSec advisory scan (#40)

The checked-in `Cargo.lock` / Cargo workspace is scanned in CI by
`.github/workflows/rust-audit.yml`. That workflow installs a pinned `cargo-audit`
release (`CARGO_AUDIT_VERSION`, currently `0.22.2`) on Ubuntu 24.04 with the same
`dtolnay/rust-toolchain@stable` style as the Linux package smoke, then runs:

```bash
cargo audit --file Cargo.lock
```

The job fails on vulnerability-class advisories (the default `cargo-audit` threshold).
It does not upgrade dependencies or change product code. Unmaintained / unsound
**warnings** alone do not fail the default gate. A local 0.22.2 probe exits 0 with 17
allowed warnings rather than a clean advisory inventory: the GTK3/gtk-rs IDs are
`RUSTSEC-2024-0411` through `RUSTSEC-2024-0420`, plus `RUSTSEC-2024-0370`,
`RUSTSEC-2024-0429`, `RUSTSEC-2025-0075`, `RUSTSEC-2025-0080`,
`RUSTSEC-2025-0081`, `RUSTSEC-2025-0098`, and `RUSTSEC-2025-0100`.
`RUSTSEC-2024-0429` is an unsoundness warning in the GTK3 stack; it is not silently
represented as “no advisories” and remains a follow-up dependency decision outside
this gate. A weekly scheduled run refreshes the live RustSec database even when the
lockfile is unchanged.

Local Darwin hosts may not have `cargo-audit` on `PATH`. That availability limitation
is explicit: this environment does **not** treat a local Darwin `cargo audit` as
authoritative evidence. CI is the authoritative Rust advisory-database evidence for
this gate. Recorded implementation run `29213826861` passed on Ubuntu 24.04 x86_64 and
emitted the same 17 allowed warnings; it is not an advisory-clean claim:
<https://github.com/AustinKelsay/distill/actions/runs/29213826861>.

A non-authoritative advisory inventory observed against the current lockfile
(warnings only; not a CI pass/fail claim) is recorded in
`docs/runs/issues/40-rust-advisory-scan.md`. It lists gtk-rs GTK3 bindings
(`atk`, `atk-sys`, `gdk`, `gdk-sys`, `gdkwayland-sys`, `gdkx11`, `gdkx11-sys`,
`gtk`, `gtk-sys`, `gtk3-macros`) as unmaintained, plus `proc-macro-error` and the
`unic-*` crates as unmaintained, and `glib` `0.18.5` as unsound
(`RUSTSEC-2024-0429`). The final CI run confirms the same warning inventory.

## Documentation-drift gate

`npm run check:docs` verifies the canonical docs package, authority order, gap
register, matrix/registry parity, active executable paths, and agent instructions.
A docs-only change must still run the same check.

## Combined launcher

```bash
node scripts/run-library-checks.mjs rebuild   # fmt + clippy + cargo test
node scripts/run-library-checks.mjs library   # same as rebuild
node scripts/run-library-checks.mjs faults    # test-faults suite
node scripts/run-library-checks.mjs desktop   # typecheck/lint/format/test/frontend build
node scripts/run-library-checks.mjs docs      # docs and retired-boundary check
node scripts/run-library-checks.mjs release   # beta version/artifact check
node scripts/run-library-checks.mjs all       # full package
```
