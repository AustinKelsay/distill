# Issue Session — #36 Linux Packaging

## Issue

- Issue: [#36](https://github.com/AustinKelsay/distill/issues/36)
- Fixed point before session: `dc40858`
- Implementation commit: `1a73549`
- Status: In progress — implementation complete; Ubuntu CI pending
- Review packet: `docs/runs/reviews/36-linux-packaging.md`

## Intended Contracts

- Ubuntu 24.04 CI builds supported `.deb` and AppImage packages with documented
  WebKitGTK/GTK runtime dependencies and the events-only Tauri capability source.
- CI installs the generated `.deb` and launches the installed `/usr/bin/distill-desktop`
  under an Xvfb/dbus session rather than substituting an unpackaged host.
- The installed host completes Fixture sync, search, detail, train curation, export,
  quit/relaunch, and artifact checks; Fixture source hashes remain unchanged and new
  temp-parent files remain under the chosen home/Fixture roots.
- Evidence is explicit about non-claims: no migration, crash recovery, privacy, scale,
  export atomicity, package signing, or screen-reader proof.

## Implementation

- `apps/distill-desktop/src-tauri/tauri.linux.conf.json` declares Debian runtime
  dependencies and disables AppImage media bundling while leaving the macOS default
  target unchanged.
- Root/workspace scripts expose `desktop:package:linux` and `desktop:smoke:linux`.
- `apps/distill-desktop/scripts/linux-package-smoke.mjs` verifies both artifacts and
  Debian `Depends` metadata, then drives the installed window by accessible
  control name under Xvfb, then checks the chosen home, curated train JSONL, restart
  persistence, Fixture SHA-256 contents, and temp-parent containment.
- `.github/workflows/linux-package-smoke.yml` installs Ubuntu build/smoke dependencies,
  packages `.deb`/AppImage, installs the `.deb`, runs the installed-host smoke, and
  uploads both artifacts.
- `apps/distill-desktop/docs/linux-runtime-deps.md` documents the Ubuntu baseline and
  explicit runtime/build dependencies.

## Verification

- Linux-only package and CI commands are not runnable on this Darwin host; the workflow
  is the authoritative Ubuntu evidence path.
- `node --check apps/distill-desktop/scripts/linux-package-smoke.mjs` and repository
  formatting/type checks are required before commit.
- Independent Grok 4.5 xhigh implementation rereview: PASS after Debian `Depends`,
  AT-SPI, fail-closed binary, and evidence-wording remediations. Ubuntu CI is still the
  authoritative runtime evidence path.

## Remaining Scope

The Linux gate remains pending until the Ubuntu workflow completes successfully. This
slice does not add RPM, Windows, signing-store claims, VoiceOver/Narrator coverage, or
application-level privacy/delete semantics.
