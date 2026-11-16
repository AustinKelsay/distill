# Status of the rebuild

As of 2026-11-15. The spec is issue #72, with tickets #73 to #91; terms are in `GLOSSARY.md` and decisions in `docs/adr/`. "Built" means implemented with tests. Each ticket updates this file when it lands.

## Built

| Ticket | What is built |
| --- | --- |
| #73 Strip the workspace | The workspace holds only capture and the parsers (ADR 0001). The previous build is tagged `agent-driven-final` (`a6e4be0`). Dataset recipes, builds, approval and publishing, recipe proposals, parser repair, legacy import, git outcome joins, the old steward and its provider setting, the old search, the old MCP tools and the old setup and dashboard screens are removed. Commands: `init`, `harnesses`, `sweep`, `verify`, `parse`, `tick`, `status`, `runs`, `schedule`, `gui`; none prints session text. The tick runs `tick::STEPS` (sweep, then parse) and writes one run-log entry. Status is one report in core (`status::status`). The MCP server and the GUI are shells that expose nothing. Config is written with the spec's names; a previous build's config still loads. Session facts hold no title and no git outcome (`META_VERSION` 4). |
| #89 Harness pruning check | What each of the eleven harnesses prunes, rewrites or compacts, when, and which setting controls it is in `docs/harness-pruning.md` (checked 2026-11-15). Status reads the settings that exist (Claude Code's `cleanupPeriodDays` and `desktopSessionCleanupPeriodDays`, including managed settings; Grok's `[storage] cleanup_ttl_days`; Hermes's `sessions.auto_prune` and `retention_days`) and Droid's fixed 7 days for orphaned side questions (`pruning::retention`). It warns, in `Status::warnings`, when one could delete sessions sooner than the timer sweeps, and when nothing has swept for longer than twice the timer's interval (counting from setup before the first sweep). `distill status` prints the warnings. |

## Kept from the previous build

- **Capture.** The sweep for all eleven harnesses, append vs. rewrite detection, content-addressed captures, the manifest, `verify`, privacy exclusions, the archive writer lock, fsync before anything is superseded, and the launchd/systemd timer. The raw archive's on-disk format is unchanged; a test reads a manifest and a capture the previous build wrote. On real data the previous build checked an idempotent re-sweep and a clean verify (2,465 captures).
- **Parsers.** Claude Code, Codex (plain and `.jsonl.zst`), OpenCode, Droid, Pi, Cursor (chat and ACP agent stores, and the plain transcripts it keeps per project), Grok, Antigravity, Hermes (its `state.db` exported one session at a time, plus its session files), T3 Code threads and Goose, with full events (tool calls, results, reasoning). Every other file under a harness's roots is archived as a sidecar. On real data the previous build parsed 12,824 sessions with no failures.
- **Mirrors.** A T3 thread run on a natively captured harness is a mirror, and so is a Cursor transcript of an agent whose store is archived. A mirror counts as a duplicate only while its original is in the archive and reaches as far as it does (`SessionMeta::is_duplicate`); its only caller was removed, and it is kept for search and cards.
- **The previous build's steward mark.** Sessions that worked inside the home are marked `steward` in their facts. Kept for search and cards to leave them out.
- **Format drift.** A capture over 16 KB that parses to nothing is a failure flagged as drift, shown per harness in `distill status`.

## Parked, with no callers

- `distill-core/src/parked/redact.rs`: the old redactor's patterns and tests, for the detector ticket.
- `distill-core/src/parked/ollama.rs`: the check that a model's weights are stored on this machine rather than served from a cloud (`ollama_model_is_local`, `parse_ollama_list`), with its tests, for the providers ticket.

## Full Disk Access

The timer job no longer needs Full Disk Access on macOS. The previous build needed it only for git outcome joins, which ran `git` inside each session's project, often under `~/Desktop`, `~/Documents` or `~/Downloads`, which macOS protects. The sweep reads only the harnesses' own folders (`~/.claude`, `~/.codex`, `~/.cursor`, `~/.factory`, `~/.pi`, `~/.grok`, `~/.gemini`, `~/.hermes`, `~/.t3`, `~/.local/share/...`), none of which macOS protects, and exclusions compare paths without opening them. The signing step is dropped from `scripts/install.sh` and `scripts/setup-signing.sh` is removed. A harness root moved by hand into a protected folder would need the grant again.

## Known gaps

- The pruning check reads the owner's own harness settings and Claude Code's managed settings files, not project-level settings (a project's `.claude/settings.json` can shorten `cleanupPeriodDays`), MDM profiles or server-managed settings. Antigravity's trimming by size and every in-place rewrite have no setting to read (see `docs/harness-pruning.md`).
- OpenCode spills full tool outputs to `tool-output/` beside its database and deletes them after 7 days. Distill's OpenCode root is the database alone, so they are not captured.
- Format drift is caught only when a capture parses to nothing: a change that drops some record types while others still parse is not detected.
- With any privacy exclusion set, Antigravity and Hermes are not archived at all: their sessions don't record a working directory.
- With any privacy exclusion set, a T3 Code thread whose worktree and project root are unknown is not archived, and neither are new native references for it.
- Cursor's stores are SQLite files and its transcripts record no working directory, so while any privacy exclusion is set none of Cursor is archived. Its IDE chat history (`state.vscdb`) is not read.
- Cursor's SDK agent stores (`projects/*/sdk-agent-store`, how T3 Code runs Cursor) encrypt their blobs and keep the root elsewhere, so they are archived but not parsed. Their agents are covered by Cursor's transcripts and by the T3 threads that ran them; with the store unread, both of those stay.
- A config that lists every harness Distill knew when it was written gets harnesses added since (Goose) switched on; one that lists only some gets nothing added.
- Each new capture costs two directory fsyncs (about 18 ms), so a first sweep of a large machine takes a few minutes.
- A previous build's home keeps its `recipes/`, `exports/` and `steward/` folders and its old cards; they are ignored, never deleted. A card from the previous build counts as absent.
