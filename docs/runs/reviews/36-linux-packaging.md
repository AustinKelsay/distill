# Review Packet — #36 Linux Packaging

## Review Scope

- Issue: #36
- Slice: Ubuntu `.deb`/AppImage packaging and installed-host smoke
- Baseline: `dc40858`
- Implementation: pending

## Review Checklist

- [ ] Ubuntu workflow builds both package formats with documented runtime dependencies.
- [ ] CI installs the `.deb` and launches the installed binary under Xvfb/dbus.
- [ ] The packaged journey covers Fixture sync, search, detail, train curation, export,
      restart, and artifact/containment checks.
- [ ] Capability source remains `core:event:default` only.
- [ ] Non-claims remain explicit; Linux and screen-reader evidence is not confused with
      migration, privacy, crash, scale, or export-atomicity proof.

## Reviewer Output

Pending independent Grok 4.5 xhigh implementation rereview and Ubuntu workflow result.

## Verification Record

Pending Linux CI run, CodeRabbit attempt, final gates, and implementation commit.
