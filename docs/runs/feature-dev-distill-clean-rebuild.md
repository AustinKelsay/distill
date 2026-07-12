# Distill Clean Rebuild — Feature Dev Run Ledger

## Run

- Run ID: `2026-07-11-distill-clean-rebuild`
- Loop: Matt Pocock skills v1.1 / Plebdev Feature Dev loop v0.4.0
- Target repo: `/Users/plebdev/Desktop/Projects/distill-clean-rebuild` (`AustinKelsay/distill`)
- Base branch: `staging` at `1a74ca1`
- Feature branch: `feature/distill-clean-rebuild`
- Human owner: Austin Kelsay
- Started: 2026-07-11
- Current status: #18–#24 complete; #25 ready
- Skill setup status: Complete — GitHub Issues, canonical triage labels, and single product-domain context
- Sub-agent policy: Grok 4.5 xhigh only unless the human explicitly authorizes a small number of Luna high workers

## Goal

Rebuild Distill completely from scratch in a cleaner, more elegant fashion, end to end. Preserve the proven product invariants and lessons from both the Electron implementation and the Rust/Slint experiment, while allowing the loop to choose the technology stack, tools, interfaces, trade-offs, and implementation sequence. Run as many feature-development slices as needed until the clean rebuild is complete and verified.

## Durable Artifacts

- Research dossier: `/Users/plebdev/Desktop/Projects/distill/apps/distill-desktop/docs/research/from-scratch-rebuild-dossier.md`
- Electron study: `/Users/plebdev/Desktop/Projects/distill/apps/distill-desktop/docs/research/electron-product-study.md`
- CONTEXT updates: Root `CONTEXT.md` created with the local-conversation-refinery language
- ADRs: `docs/adr/0001-rust-library-with-tauri-shell.md`, `0002-captures-attempts-and-projections-are-distinct.md`, `0003-sqlite-and-content-addressed-files-are-library-internals.md`
- Prototype source branch, if any: None
- Spec issue: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Tickets: [#18–#37](https://github.com/AustinKelsay/distill/issues/18)
- Ticket sessions: #18–#21 implemented; #21 review pending
- Agent briefs: Pending
- Review packets: #18–#24 complete
- Local CodeRabbit report: #23 and #24 pre-commit attempts/findings recorded; post-fix attempts were rate-limited by service
- PR URL: Pending

## Commands

- Install: `npm install` (legacy Electron baseline + desktop workspace)
- Typecheck: `npm run build` (legacy); `npm run desktop:typecheck` (rebuild renderer)
- Test: `npm test` (legacy); `cargo test --workspace` (rebuild); `cargo test -p distill-library --features test-faults` (fault contracts); `npm run desktop:test` (renderer)
- Build: `npm run build` (legacy); `cargo build --workspace`; `npm run desktop:frontend:build`
- Visual verification: legacy `npm start`; rebuild `npm run desktop:dev` (host boundary; packaging deferred)
- Rebuild gates: see `docs/gates.md`; `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `cargo test -p distill-library --features test-faults`

## Ticket Ledger

| Issue | Type | Status | Review thread | Fixes needed | Verified |
| --- | --- | --- | --- | --- | --- |
| #18 Library Fixture tracer | AFK | Complete | Grok xhigh standards + spec | All applied; both axes pass | Rust 5 pass; legacy 93 pass/10 baseline runtime failures |
| #19 Thin Tauri/React/CLI callers | AFK | Complete | Grok xhigh standards + spec | Both axes pass; ACL finding withdrawn | Rust/renderer/Tauri release gates; legacy 103 pass on Node 26 |
| #20 Attempt retry/replay/replacement | AFK | Complete | Grok xhigh standards + spec | Docs inventory and diagnostic safety fixed; both axes pass | Rust/renderer/Tauri release gates; legacy 103 pass; CodeRabbit 0 findings |
| #21 Health/repair/fault recovery | AFK | Complete | Grok xhigh standards + spec | Staging-root symlink blocker fixed; both axes pass | 13 health + 8 fault contracts; Rust/desktop/Tauri/legacy gates pass |
| #22 Async source settings/Sync Runs | AFK | Complete | Grok xhigh AFK worker + two independent reviews + focused rereview | All findings applied; final CodeRabbit attempt rate-limited for 10 minutes | Library 17 OSR checks; CLI/host 7 each; renderer 11; Rust/desktop/Tauri/legacy gates |
| #23 Search/lanes/detail/virtualization | AFK | Complete | Grok xhigh standards + spec rereview; CodeRabbit follow-ups applied | Two CodeRabbit findings fixed; final attempt rate-limited | Rust/desktop/Tauri gates; CLI/host seams; Node 22 legacy baseline unchanged |
| #24 Transactional Curation | AFK | Complete | Grok xhigh standards + spec rereview | Low evidence findings applied; CodeRabbit attempt rate-limited | Library/CLI/host/renderer/Rust/desktop/Tauri gates pass |
| #25 Recoverable export | AFK | Blocked by #18, #24 | — | — | No |
| #26 Codex Source | AFK | Blocked by #18, #22 | — | — | No |
| #27 Claude Code Source | AFK | Blocked by #18, #22 | — | — | No |
| #28 OpenCode Source | AFK | Blocked by #18, #22 | — | — | No |
| #29 Droid Source | AFK | Blocked by #18, #22 | — | — | No |
| #30 Activity/operations diagnostics | AFK | Blocked by #22, #25 | — | — | No |
| #31 Electron migration | AFK | Blocked by #20, #21, #23, #24, #25 | — | — | No |
| #32 Hostile-input/capability audit | AFK | Blocked by #19, #26–#29 | — | — | No |
| #33 Accessibility/visual states | AFK | Blocked by #19, #22–#25, #30 | — | — | No |
| #34 Scale/performance | AFK | Blocked by #22–#25 | — | — | No |
| #35 macOS packaging | AFK | Blocked by #19, #22–#25 | — | — | No |
| #36 Linux packaging | AFK | Blocked by #19, #22–#25 | — | — | No |
| #37 Matrix/cutover | AFK | Blocked by #21, #26–#36 | — | — | No |

## Parked HITL Slices

| Issue | Why parked | Blocks | Required human action | Final PR decision |
| --- | --- | --- | --- | --- |
| — | — | — | — | — |

## Issue Session Ledger

| Issue | Fixed point | Worker session | Commit | Review result | Checks |
| --- | --- | --- | --- | --- | --- |
| #18 | `b471a77` | Grok 4.5 xhigh edit session | `a13bf74`, `b87f5cb` | Both axes pass after all findings applied | fmt/clippy/Library 5 pass; legacy build + 93 pass, 10 Node 22 baseline failures |
| #19 | `5655cde` | Grok 4.5 xhigh edit session | `e9cd49a`, `e39c451` | Both axes pass; ACL finding withdrawn | Rust workspace 14 + renderer 5; Tauri release build; legacy 103 pass on Node 26 |
| #20 | `4564d28` | Grok 4.5 xhigh edit session | `50d8633`, `f4b3514` | Both axes pass after architecture/diagnostic fixes; CodeRabbit 0 findings | fmt/clippy/workspace tests + desktop typecheck/test/frontend/Tauri no-bundle + legacy 103 on Node 26 |
| #21 | `b5713cc` | Grok 4.5 xhigh AFK + audit remediation | `1799b5b`, `f1d3244` | Both axes pass after staging-root symlink hardening | 13 health + 8 fault; CLI 6; host 5; renderer 7; Tauri release; legacy 103 |
| #22 | `3e420df` | Grok 4.5 xhigh AFK implementation | `ab2cc83` | Standards + spec + final focused rereview pass; CodeRabbit prior findings applied, final attempt rate-limited | fmt/clippy/workspace + fault/lease suites; library_ops_sync 17; CLI/host 7; renderer 11; desktop frontend/Tauri; legacy 103 on Node 26 |
| #23 | `d175b5b` | Grok 4.5 xhigh implementation + independent rereview | `9f3043d` | PASS; CodeRabbit findings applied, final attempt rate-limited | fmt/clippy/workspace; Library query 6; CLI/host 8 each; renderer 14; frontend/Tauri release |
| #24 | `2921523` | Grok 4.5 xhigh implementation + independent rereview | `5f6fd09` | PASS; low evidence findings applied; CodeRabbit attempt rate-limited | Library curation 9; CLI/host 9 each; renderer/Vitest 16; fmt/clippy/workspace; frontend/Tauri release |

## Open Questions

- None. Testing seams and the revised twenty-ticket graph were accepted under the human's standing full-control delegation after focused Grok xhigh review.

## Proposed Testing Seams

1. Primary seam: drive the public Rust `Library` interface against a real temporary Distill home, SQLite database, content-addressed file store, and SourceAdapter. The Fixture adapter must use the exact production adapter interface and may not bypass discovery, snapshot, or parsing.
2. Split the primary seam into mandatory contract families: `ingest_projection`, `attempt_retry`, `search_query`, `curation_policy`, `export_publication`, `ops_sync`, `health_migration`, `fault_injection`, and `privacy_hardening`. This keeps one external seam without creating one undifferentiated mega-suite.
3. Fault injection is a first-class Library test capability. Scenarios interrupt between blob staging/rename/database acceptance, projection/FTS/job transitions, and export temp-write/bookkeeping/final rename; then reopen the Library and assert the documented repair state, orphan collection, and missing/corrupt referenced-content health failure.
4. Cancellation and overlap consistency are proven at the Library seam: cancel sync/export at safe checkpoints, attempt concurrent syncs, and assert Captures, Normalization Attempts, Session Projections, Activity Events, Sync Runs, and Export Artifacts remain mutually consistent.
5. Privacy/hardening is split across Library and host contracts: restrictive home/file modes, path canonicalization and symlink/traversal defense, capture and JSON depth/size limits, subprocess timeout/output bounds, redacted diagnostics, hostile provider payloads, renderer capability deny-list, and the explicit v1 absence of application-level encryption.
6. SourceAdapter conformance runs for Codex, Claude Code, OpenCode, Droid, and Fixture, asserting canonical output and stage-typed errors only. Codex file-backed and OpenCode virtual captures also run through the full Library seam to prove replay after source deletion and source-failure isolation.
7. The CLI has a thin command seam covering arguments, exit codes, progress, cancellation, paths, JSON output, and the same Library outcomes; it may not implement product policy.
8. The Tauri host contract validates payloads, generated/shared types, capability restrictions, and exact translation into Library calls. The React renderer uses one typed bridge fake to prove keyboard behavior, focus order, roles/names, virtualization correctness, progress/cancel wiring, export preview, and explicit idle/loading/refreshing/empty/warning/error/cancelled states.
9. Broader accessibility claims use axe/static checks, contrast and reduced-motion checks, scalable-text snapshots, deterministic major-state screenshots, and a packaged keyboard/focus smoke. Screen-reader/assistive-technology claims remain human-validation gates unless a supported automation exists.
10. Packaged macOS and Linux smoke stays short: install/launch, first-run fixture sync, one search/detail/curation/export path, restart, and artifact existence. It does not stand in for fault, privacy, migration, or export-atomicity contracts. Windows packaging is out of scope for v1.
11. The legacy contract matrix must be replaced or remapped before parity claims. Every scenario gets a stable ID, owning spec clause, family/seam, fixture, executable test symbol, expected Library result, durable/audit effect, supported platforms, and status. New required scenarios cover Normalization Attempts, Droid, export publication, crash points, concurrent sync, cancellation, Electron-home import, checksummed migrations, privacy, accessibility, and scale budgets.

Grok xhigh test-architecture review initially rejected the looser proposal. The eleven obligations above incorporate every high-severity finding and the relevant medium findings.

## Escalations

- The original `/Users/plebdev/Desktop/Projects/distill` worktree is heavily dirty with user-owned Rust rebuild and research changes. It is preserved untouched. This run uses the separate clean worktree `/Users/plebdev/Desktop/Projects/distill-clean-rebuild` based on `staging`.
- Feature Dev stops at a non-draft PR into `staging`; production deployment is outside this loop.
- Setup defaults were inferred from the human's explicit full-control delegation: GitHub remote/Issues, default labels, and one Distill product-domain context.
- Product center inferred from the completed research and human goal: Distill is a local conversation refinery, not a generic data platform.
- Design It Twice used four Grok xhigh briefs: minimal interface, extensible adapters, common desktop caller, and adversarial reliability/privacy.
- Chosen stack: Rust Library + rusqlite/SQLite/FTS/CAS, Tauri 2 host, React/TypeScript/Vite renderer, thin Rust CLI, npm and Cargo workspaces, GitHub Actions, contract tests through Library plus renderer/UI tests.
- Rejected as primary stack: TypeScript Library in Electron UtilityProcess (runtime/privilege weight) and Rust/Slint (accessibility/testing/controller lessons from the experiment).
- Testing-seam confirmation: the refined proposal was shown to the human with a recommended "yes" and no objection was supplied across continuation; the human's original instruction grants full control over tools, stack, and trade-offs. The orchestrator therefore accepted the refined seams rather than leave the autonomous run parked indefinitely.
- Spec review: Grok xhigh initially rejected ten underspecified contracts; all ten were incorporated and the focused re-review passed before publishing issue #17.
- Ticket review: Grok xhigh initially rejected the nineteen-ticket draft for oversized early slices and false edges. The revised twenty-ticket graph split the Library tracer from callers, retry from health/fault recovery, moved baseline privacy into owning tickets, removed the CLI catch-up ticket, and corrected provider, scale, and packaging edges. The focused re-review passed before publishing #18–#37.
