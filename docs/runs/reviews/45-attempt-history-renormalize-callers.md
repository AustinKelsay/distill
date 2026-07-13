# Review Packet — #45 Attempt History and Capture Renormalization Callers

## Review context

- Issue: [#45](https://github.com/AustinKelsay/distill/issues/45)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point: `d93eb66`
- Worker: Grok 4.5 High bounded implementation; Codex integration and remediation
- Loop: Matt Pocock skills v1.1 / Plebdev Feature Dev loop v0.4.0
- Review mode: independent Grok standards/spec review, followed by Codex remediation
- Final review status: `STANDARDS_STATUS: pass`; `SPEC_STATUS: pass`

## Scope reviewed

The review covered the public CLI and Tauri host seams, Tauri command
registration, typed bridge and React Attempt-history/renormalize states,
Activity-based Capture discovery, parser-version advancement boundaries,
same-Capture retry behavior, root-removal replay, unknown-kind isolation,
redaction, race cancellation, and canonical architecture/matrix/evidence/gap
updates.

## Findings and remediation

The independent review identified a P0 cross-Session race: an in-flight
Attempt read or renormalize could complete after the user selected a different
Session. It was fixed by invalidating the Attempt request generation whenever
Session selection or the explorer reset clears Attempt state, with a dedicated
pending-promise renderer test. Activity discovery was also changed from a
single-page read to a bounded cursor walk, while still requiring a
Session-matching Activity event; it never falls back to an unrelated Capture.
Evidence rows remain pending until final-head CI rather than claiming passed
for unstaged work.

The final review returned:

```text
STANDARDS_STATUS: pass
SPEC_STATUS: pass
FINDINGS: none after remediation
```

## Verification evidence

- `cargo test -p distill-cli --test cli_fixture_journey` — 19 passed.
- `cargo test -p distill-desktop --test host_multisource_journey -- --test-threads=1` — 7 passed.
- `npm --prefix apps/distill-desktop test -- --run` — 45 tests passed across 6 files.
- `npm run desktop:typecheck` — passed.
- `npm run desktop:lint` — passed.
- `npm run desktop:format` — passed.
- `npm run desktop:frontend:build` — passed.
- `cargo test --workspace` — passed; the existing full-scale benchmark remains ignored by default.
- `cargo test -p distill-library --features test-faults` — passed.
- `cargo fmt --all` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo tree --workspace --locked` — passed.
- `git diff --check` — passed.
- Local `coderabbit review --agent --type all --base staging` reached
  `summarizing` after setup and was terminated after the bounded wait; the
  independent Grok review is the recorded fallback.

## Explicit residuals

- Exact implementation-head Linux package/install/smoke [run 29222714955](https://github.com/AustinKelsay/distill/actions/runs/29222714955) and Rust advisory [run 29222714957](https://github.com/AustinKelsay/distill/actions/runs/29222714957) passed on `afcd4a7`; subsequent closeout commits are documentation-only.
- Packaged real-provider machine roots are not claimed; caller journeys use
  hermetic roots and a local OpenCode fake.
- VoiceOver/Narrator speech, Developer ID signing/notarization/stapling,
  Windows packaging, Electron retirement, and user-facing parser-version
  preference remain out of scope or human-gated.
