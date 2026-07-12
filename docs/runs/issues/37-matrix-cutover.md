# Issue #37 — Contract Matrix And Electron Cutover Gate

## Status

Evidence complete on `feature/distill-clean-rebuild`; final staging handoff is the
non-draft PR [#38](https://github.com/AustinKelsay/distill/pull/38). The native rebuild
is the routine source-to-export product path. Electron under `src/**` is intentionally
retained as read-only migration/baseline evidence and is not deleted by this ticket.

Run date: 2026-07-12. Primary local toolchain: Node 26.0.0, npm 11.12.1, Rust stable,
macOS Darwin arm64. Linux package evidence is the Ubuntu 24.04 x86_64 CI run recorded
below.

## Acceptance mapping

| #37 requirement | Evidence |
| --- | --- |
| Every scenario has the required evidence fields | `docs/testing/contract-scenario-evidence.md` contains 130 stable IDs, each with spec clause, family/seam, fixture, executable symbol, expected-result reference, durable/Activity effect, platforms, status, and last evidence. The concise normative rows remain in `docs/testing/contract-test-matrix.md`. |
| Attempts, Droid, export lifecycle/crash, overlap/cancellation, migration, checksummed migrations, privacy, accessibility, scale, and packaging are covered | Matrix rows include `AR-*`, `LDR-*`, `EP-*`, `FIR-*`, `OSR-003/004/013/015`, `LMI-*`, `LFT-*`, `LPH-*`, `A11Y-*`, `SCALE-001..004`, `PKG-*`, and `LPKG-*`. Suite index includes Activity/Operations read models and Library export publication. |
| Documented gates pass | The gate results below are from the current branch after a clean Node 26 install. The one Node 22 legacy inspector run is recorded as an informational known baseline, not silently counted as a pass. |
| Honest cutover decision | Native CLI/Tauri/React and packaged macOS/Linux Fixture journeys complete sync → search/detail → curation → JSONL export → restart without Electron. Human assistive-technology, signed-release, Windows, and Rust advisory-database limitations remain explicit. |

## Gate results

| Gate | Command/evidence | Result |
| --- | --- | --- |
| Install | `PATH=/opt/homebrew/Cellar/node/26.0.0/bin:$PATH npm ci --ignore-scripts` | Passed; 283 packages installed. `npm ci` is install-only; the separate advisory result is recorded below. |
| Rust format | `cargo fmt --all -- --check` | Passed. |
| Rust lint | `cargo clippy --workspace --all-targets -- -D warnings` | Passed. |
| Rust workspace tests | `cargo test --workspace` | Passed; all workspace tests green (the default scale full benchmark remains intentionally ignored). |
| Fault/recovery tests | `cargo test -p distill-library --features test-faults` | Passed; fault and export recovery suites green. |
| Lease/cancellation tests | `cargo test -p distill-library --test library_ops_sync --features test-leases` | Passed; 17 tests green, including overlap, cancellation, stale lease, heartbeat, and `OSR-015` large-stdin cleanup. |
| Rust build | `cargo build --workspace` | Passed. |
| Desktop callers | `node scripts/run-library-checks.mjs all` with Node 26 | Passed; Rust, desktop typecheck/lint/format/renderer tests/build, fault suite, and legacy suite all green. |
| Renderer accessibility/state | `npm --prefix apps/distill-desktop run a11y:smoke` | Passed; frontend build plus 12 renderer accessibility/state tests. This is not packaged screen-reader evidence. |
| Legacy documentation drift | `npm test` through the Node 26 `all` launcher | Passed; 104/104 tests. Node 22’s historical 93/103 result is the known `better-sqlite3` inspector incompatibility documented in `docs/gates.md`. |
| Rust dependency resolution | `cargo tree --workspace --locked` | Passed; checked-in lockfile resolves. `cargo audit` is not installed in the pinned toolchain, so no Rust advisory-database claim is made. |
| JavaScript dependency audit | `npm audit --audit-level=moderate --ignore-scripts` | Passed; 0 info/low/moderate/high/critical findings after updating the retained Electron baseline to `^41.10.1`. |
| macOS package | `npm run desktop:package:macos` | Passed; local unsigned/ad-hoc `Distill.app` built with `dev.distill.desktop`, macOS 12.0 minimum, checked-in icon, and events-only capability. |
| macOS packaged smoke | `npm run desktop:smoke:macos` | Passed; Darwin arm64 sync/search/detail/train-curation/export/restart/Fixture-hash/temp-parent evidence. |
| Linux package/install/smoke | Ubuntu workflow [29210575567](https://github.com/AustinKelsay/distill/actions/runs/29210575567) | Passed on latest PR head; `.deb` + AppImage metadata, installed `/usr/bin/distill-desktop`, dbus/Xvfb UI journey, restart/export/containment. |
| Documentation diff | `git diff --check` plus the docs tests above | Passed. |

The combined launcher intentionally does not run the platform-specific Linux package
commands on Darwin; the Ubuntu workflow is the authoritative Linux package gate.

## Native routine-loop evidence

- The thin CLI and Tauri host call the Rust Library for source detection, Sync Runs,
  paging/search/detail, transactional Curation, export preview/publication, Activity,
  Operations, health/repair, cancellation, and legacy-home migration.
- The React renderer is bridge-only and has deterministic first-run, loading, empty,
  populated, warning, error, cancelled, migration, and export states.
- The macOS packaged smoke and the green Ubuntu installed-host smoke each complete a
  real Fixture source-to-export journey and verify restart persistence and source-root
  immutability.
- Provider-specific Library evidence for Codex, Claude Code, OpenCode, Droid, and
  Fixture is recorded in issues #18 and #26–#29 and mapped row-by-row in the registry.

## Remaining honest human or out-of-scope items

- VoiceOver/Narrator output, packaged dialog focus, and assistive-technology behavior
  remain human release checks in `apps/distill-desktop/docs/a11y-human-checklist.md`;
  `A11Y-005` stays `human-gate-open`.
- The macOS artifact is local unsigned/ad-hoc. There is no Developer ID, hardened
  runtime, notarization, stapling, or store-signing claim.
- Windows packaging is out of scope for v1.
- The default scale suite is a bounded smoke; the 25k Session / 1M message / 10 GiB
  logical-home benchmark is scheduled/manual evidence in #34, not a default PR cost.
- `cargo tree --locked` proves Rust dependency reproducibility; a Rust advisory scan
  would require separately installing `cargo-audit` and is not represented as passed.

These residuals do not make Electron a routine dependency. Electron remains in the
repository so the read-only migration seam and legacy baseline can be exercised until a
separate retirement decision is approved.
