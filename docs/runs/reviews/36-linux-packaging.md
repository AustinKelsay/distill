# Review Packet — #36 Linux Packaging

## Review Scope

- Issue: #36
- Slice: Ubuntu `.deb`/AppImage packaging and installed-host smoke
- Baseline: `dc40858`
- Implementation: `1a73549`, `0d7989d`, `bffe87c`

## Review Checklist

- [x] Ubuntu workflow builds both package formats with documented runtime dependencies.
- [x] CI installs the `.deb` and launches the installed binary under Xvfb/dbus.
- [x] The packaged journey covers Fixture sync, search, detail, train curation, export,
      restart, and artifact/containment checks.
- [x] Capability source remains `core:event:default` only.
- [x] Non-claims remain explicit; Linux and screen-reader evidence is not confused with
      migration, privacy, crash, scale, or export-atomicity proof.

## Reviewer Output

Grok 4.5 xhigh final rereview: PASS. Prior blockers for Debian `Depends`, AT-SPI
accessible-name driving, fail-closed installed-binary selection, and evidence wording
were fixed. Ubuntu workflow `29210244139` is green:
<https://github.com/AustinKelsay/distill/actions/runs/29210244139>.

## Verification Record

CodeRabbit uncommitted review found one major README evidence-status overclaim; it was
fixed before commit. Local gates passed: `cargo fmt --all -- --check`, workspace
clippy with `-D warnings`, workspace tests, release build, desktop typecheck/lint/test,
frontend build, Node/Python syntax checks, formatting, and `git diff --check`. The
Ubuntu smoke recorded `Distill_0.1.0_amd64.deb`, `Distill_0.1.0_amd64.AppImage`,
`/usr/bin/distill-desktop`, `core:event:default`, `ui: passed`, `restart: passed`, and
non-empty `distill.db`/train JSONL artifacts. CodeRabbit's one README finding and the
two CI runtime findings (window-manager activation, offscreen AT-SPI control) were
fixed before the green run. Screen-reader conformance remains explicitly unclaimed.
