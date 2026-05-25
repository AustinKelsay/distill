# distill

`distill` is the umbrella monorepo for the Distill desktop products.

## Apps

- `apps/distill-electron`: the current Electron implementation, renamed from the original standalone `distill` app to `distill-electron`
- `apps/distill-desktop`: the new Rust rewrite scaffold for the future desktop implementation

## Commands

Run these from the monorepo root:

```bash
npm install
npm run doctor
npm run import
npm start
npm test
npm run desktop:check
```

The Electron app’s canonical docs live in [apps/distill-electron/docs/README.md](apps/distill-electron/docs/README.md).
