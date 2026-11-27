# Structured reads and personal-detail placeholders stay on the read path

Downstream programs that distill, analyze or build training material from the archive (#214) need machine-readable events, session facts and cards in bulk, and often need personal details replaced with placeholders alongside secrets. Reading `sessions/.../events.jsonl` directly would bypass the read path (ADR 0002), expose raw secrets and leave no trace in the read log. Adding dataset recipes or training-format exporters would reopen ADR 0006.

Instead, structured reads extend the one read path. A downstream program reads through `distill-core::read` (via the CLI's structured JSONL stream) as a named reader, under the same trust rules (ADR 0003) and read log as every other read.

## Structured JSONL shape

The read path offers one versioned JSONL wire shape (`schema_version: 1`), distinct from DISTILL's internal on-disk `events.jsonl` and `meta.json` (which stay private to DISTILL and rebuildable from the raw archives, ADR 0001). Each line is one JSON object for a session:

- `schema_version` (`1`)
- `session` (the session's key, `<harness>/<id>` or `<machine>:<harness>/<id>`)
- `facts` (a wire-level projection of session facts—harness, machine, project, branch, models, times, parent, counts and shape stats—excluding internal rebuild fields such as raw capture origins and parser versions)
- `card` (the current card, or `null` when the session has no current card)
- `events` (the session's events with placeholders applied)
- `scan_coverage` (pattern-scan currency and whether model personal-detail findings are current, plus the scope the model pass covered)
- `children` (omitted unless tree reading was requested; when requested, the array of descendant session records)

Current, applicable pattern findings are a precondition for emitting a session record in every placeholder mode. A session whose pattern scan is missing, out of date or whose findings do not fit its events is withheld and never emitted unmasked; `scan_coverage.pattern_current` is therefore `true` on every emitted record, while `scan_coverage.model_details_current` and `scan_coverage.model_details_scope` state what the optional model personal-detail pass has covered.

Structured JSONL carries no `NOTICE_START` / `NOTICE_END` banners. Those banners guard untrusted session text pasted into an interactive model prompt; structured JSONL keeps roles and fields separate.

## Placeholder levels and path normalization

The read path supports two placeholder levels:

- **`secrets` (default):** unchanged from ADR 0002. Secrets become `[secret: <kind> #<fingerprint>]`, and personal details stay as written for trusted readers. Interactive search, reading and MCP keep this default. Free-text session facts (`facts.project` and `facts.git_branch`) are also scanned in both modes so detected secrets in a path or branch name are replaced on read too.
- **`all` (opt-in):** replaces both secrets (`[secret: <kind> #<fingerprint>]`) and personal details (`[detail: <kind> #<fingerprint>]`) across events, cards and free-text session facts (`facts.project` and `facts.git_branch`, using pattern findings and matching session detail findings). Structural identifiers (`session`, `harness`, `machine`, `models`, `tools`, timestamps and counts) stay as recorded. Keeping each finding's fingerprint preserves coreference within and across sessions without exposing the underlying value. This amends ADR 0002's unqualified rule that personal details are left in place for trusted readers: that remains the default, while readers that ask for `all` receive placeholders for personal details too.

Optional read-time path normalization is orthogonal and combinable with either `secrets` or `all`: it rewrites the session's project root and the owner's home prefix to `<project>` and `<home>` across `facts.project`, events and cards on the way out. When both placeholders and path normalization apply to the same text, all replacement spans are resolved against the original text in a single pass so one transform never shifts another's offsets. Precedence governs shared bytes in order: secret placeholders first, then path normalization (`<project>` over `<home>` when nested), then personal-detail placeholders; if a higher-priority span covers only part of a lower-priority finding (such as a path prefix overlapping part of a personal detail), any uncovered bytes of that finding are still replaced with its placeholder so no raw fragment of a detected secret or personal detail is left behind. Like placeholders, path normalization runs only on read; stored captures, events and cards never change (ADR 0002).

## Trust and batched read-log accounting

A structured read is a read, not a fourth way out beside the read path, encrypted backups (ADR 0009) and spoke exports (ADR 0010). It checks trust once up front and refuses untrusted providers and spokes before emitting anything. Rather than adding a second local-only trust tier (rejected in ADR 0003), a local-only program with no remote model names `ollama` (always trusted locally) or names whichever trusted provider will read its output.

For a single-session structured read, the read log entry is written before the record is returned, as with existing reads. For a multi-session stream, each call generates a unique `batch_id` and appends a durable batch-start record to `log/reads.jsonl` (recording `batch_id`, the reader, placeholder level and requested session count) **before** emitting the first line; if that append fails, nothing is emitted. When the stream finishes or stops early, the read path appends a closing summary line carrying the same `batch_id`, the served session count, the served event count and whether delivery completed or stopped early, so overlapping or interrupted streams pair unambiguously. If a stream is interrupted or the closing append fails, the durable batch-start record already stands in the read log and the call returns an error.

## What stays out

This reaffirms ADR 0006. DISTILL stops at this one structured JSONL stream. Format conversion (ChatML, ShareGPT, Parquet), train/validation splits, deduplication beyond mirrors, outcome grading and training loops live outside DISTILL.

## Consequences

- `tests/read_path_only.rs` stays unchanged: no face reads `events.jsonl` directly.
- Downstream programs get a stable `schema_version: 1` contract without locking DISTILL's internal on-disk files.
- Personal-detail coverage is explicit in `scan_coverage`, so a downstream program can require completed model detail scans instead of assuming coverage.
- `AGENTS.md` and `docs/known-gaps.md` update when the CLI stream (#216) and opt-in personal-detail placeholders (#217) ship.
