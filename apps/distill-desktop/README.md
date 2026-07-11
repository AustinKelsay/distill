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

`desktop:build` proves the release host with `--no-bundle`. Packaging is deferred to later tickets; `bundle.active` is false for this slice and the checked-in green icons are placeholders required by the Tauri build context.
