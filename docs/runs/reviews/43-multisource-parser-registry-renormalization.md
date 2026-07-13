# Review Packet — #43 Multi-Source Parser Registry and Same-Capture Renormalization

## Review context

- Issue: [#43](https://github.com/AustinKelsay/distill/issues/43)
- Parent: [#17](https://github.com/AustinKelsay/distill/issues/17)
- Fixed point: `e800706`
- Worker: Grok 4.5 xhigh bounded implementation pass; Codex integration
- Loop: Matt Pocock skills v1.1 / Plebdev Feature Dev v0.4.0
- Review mode: independent Grok two-axis standards/spec review
- Final review status: `STANDARDS_STATUS: pass`; `SPEC_STATUS: pass`

## Scope reviewed

The review covered the Library-owned parser registry for Fixture, Codex, Claude
Code, OpenCode, and Droid; typed Source-kind version advancement; detection and
Sync identity wiring; byte-only provider replay; same-Capture Attempt and
Projection invariants; unknown persisted-kind rejection; focused public tests;
and the canonical architecture, ingest, data-model, matrix, evidence, and gap
documents.

## Verification evidence

- `cargo test -p distill-library --test library_parser_registry --test library_attempt_retry -- --test-threads=1` — 18 passed.
- `cargo test --workspace` — passed; scale full benchmark remains the existing ignored test.
- `cargo test -p distill-library --features test-faults` — passed.
- `cargo fmt --all -- --check` — passed.
- `cargo clippy --workspace --all-targets -- -D warnings` — passed.
- `cargo tree --workspace --locked` — passed.
- `git diff --check` — passed.
- Local `coderabbit review --agent --type all --base staging` reached the
  `summarizing` phase for more than two minutes without a result and was
  terminated; the fresh Grok standards/spec rereview above is the recorded
  fallback.
- Final pushed head `693cf5e`: [Linux package/install smoke run 29218631387](https://github.com/AustinKelsay/distill/actions/runs/29218631387) passed and [Rust advisory run 29218631413](https://github.com/AustinKelsay/distill/actions/runs/29218631413) passed; PR #38 is clean.

## Findings and remediation

The first review identified two documentation/evidence issues: the empty
projection test used changed-byte re-ingest rather than same-Capture replay, and
the ingest spec did not name the unknown-kind no-mutation contract. The test was
rewritten to use a parser-gated Capture that succeeds through
`renormalize_capture` at version 2.0.0 and clears a sibling Projection; the
ingest spec now states the typed `UnknownSourceKind` rejection explicitly.

The final independent review returned:

```text
STANDARDS_STATUS: pass
SPEC_STATUS: pass
```

## Explicit residuals

- Parser registry versions are in-memory per `Library` open; a persisted
  user-editable registry is out of scope.
- Non-Fixture version bumps currently record parser identity/version; provider
  version-gated parse rules are a future slice.
- Replay intentionally omits auxiliary Codex/Claude root metadata and never
  rereads those roots or invokes OpenCode.
- Packaged CLI/Tauri renormalize UX and human accessibility/signing gates are
  outside this issue and remain tracked by the cutover evidence.
