# Issue Session — #46 Rebuild CI Gate

## Issue

- Issue: [#46](https://github.com/AustinKelsay/distill/issues/46)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point before session: `2d80280`
- Worker session: Cursor Grok 4.5 bounded CI/docs slice
- Implementation commit: `REPLACE_AFTER_COMMIT` — placeholder until committed
- Status: Implemented — workflow and docs added; first Ubuntu Actions run ID pending post-push (`REPLACE_AFTER_PUSH`)
- Review packet: `docs/runs/reviews/46-rebuild-ci-gate.md`

## Intended Contract

Add continuous pull-request enforcement on Ubuntu for the core Rust Library and
desktop rebuild commands without expanding into release signing, real providers,
Windows packaging, human screen-reader validation, Electron retirement, or
merge/close of #17 / #38.

`.github/workflows/rebuild-ci.yml` must run on PRs targeting `staging` (path-filtered)
and on `workflow_dispatch`, with least-privilege `contents: read` and bounded
timeouts. It is the authoritative CI evidence for the listed core gates in
`docs/gates.md`. Linux package smoke and RustSec advisory scanning remain separate
workflows.

## Acceptance Criteria

- [x] Dedicated workflow file `.github/workflows/rebuild-ci.yml` exists.
- [x] Triggers: `pull_request` → `staging` with path filters covering `crates/**`,
  `apps/distill-desktop/**`, `Cargo.toml`, `Cargo.lock`, `package.json`,
  `package-lock.json`, `scripts/**`, and `.github/workflows/**`; plus
  `workflow_dispatch`.
- [x] Permissions are `contents: read` only; jobs use bounded `timeout-minutes`.
- [x] Ubuntu jobs run at least:
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
- [x] Reuses repository Node/Rust setup conventions (`dtolnay/rust-toolchain@stable`,
  `actions/setup-node@v4` Node 22 + npm cache).
- [x] Does not run real providers, full-scale benchmark, package signing, or Windows.
- [x] `docs/gates.md` identifies `rebuild-ci.yml` as authoritative for these core gates
  while keeping package smoke and RustSec references intact.
- [x] Gap register, contract evidence, and feature-dev ledger record the gate and leave
  explicit residuals.
- [ ] First green Actions run ID recorded (placeholder `REPLACE_AFTER_PUSH`). The first
  implementation run (`29223953816`) caught a pre-existing provider-CLI PATH sensitivity
  in the Codex detection contract; the workflow now makes that test environment hermetic
  with a no-op Codex shim plus retained Cargo/Rustup/system paths, without invoking a
  host-installed provider CLI.

## Scope / Non-goals

In scope: continuous PR enforcement of core fmt/clippy/test and desktop renderer
gates; documentation and run-packet honesty.

Out of scope / residuals:

- Packaged real-provider machine roots
- Human assistive-technology / screen-reader speech observation
- Developer ID signing, hardened runtime, notarization, stapling
- Windows packaging
- Electron retirement
- Merge or close of root issue #17 / PR #38
- Full-scale `DISTILL_SCALE_BENCH` as a default PR cost
- Changes to product Rust/TypeScript code, lockfiles, or unrelated workflows

## Verification Commands

Local / pre-push (developer feedback; not a substitute for Ubuntu CI):

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p distill-library --features test-faults
cargo test -p distill-library --test library_ops_sync --features test-leases
npm ci
npm run desktop:typecheck
npm run desktop:lint
npm run desktop:format
npm run desktop:test
npm run desktop:frontend:build
git diff --check
node scripts/run-library-checks.mjs all   # optional combined launcher
```

Authoritative CI: GitHub Actions workflow `Distill rebuild CI`
(`.github/workflows/rebuild-ci.yml`). Run ID: `REPLACE_AFTER_PUSH`.

## Review note

Local CodeRabbit must be attempted (`coderabbit review --agent --type all --base staging`
or the repository’s current local review invocation). If CodeRabbit is unavailable,
rate-limited, or stalls, bounded Grok review is the recorded fallback — same policy
as #40–#45.

## Remaining Scope

Record the first green Ubuntu run, then update this packet, `docs/gates.md`, and the
feature-dev ledger to replace `REPLACE_AFTER_PUSH`. Residuals listed under
Non-goals remain open.
