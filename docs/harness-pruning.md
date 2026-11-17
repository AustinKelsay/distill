# Harness pruning

Checked on 2026-11-15 against each harness's own docs and source (#89). A harness that deletes sessions before Distill has captured them is the one way the archive can lose data, so `distill status` reads the settings below and warns when one could delete sessions sooner than the timer sweeps. It also warns when nothing has swept for longer than twice the timer's interval. The reader for each setting is in `crates/distill-core/src/pruning.rs`.

Two other things change a session after it is written. **Compaction** summarises the conversation for the model. **Rewriting** replaces a stored file or rows in place. The sweep keeps a rewritten file beside its earlier capture, so a rewrite loses nothing that a sweep has already captured. Anything written and then rewritten between two sweeps is lost.

Paths are the defaults. Each harness keeps its settings beside the folder its sessions are in, so status finds them from the harness's roots in `config.toml`, and a relocated harness is read where it lives.

## Summary

| Harness | Deletes sessions on its own | When | Setting | Status reads it |
| --- | --- | --- | --- | --- |
| Claude Code | Yes, by default | Older than 30 days, in the background after a session starts | `cleanupPeriodDays` in `~/.claude/settings.json` (default 30, minimum 1); `desktopSessionCleanupPeriodDays` for Desktop sessions (default 0, never) | Yes: the owner's settings and the managed settings files |
| Codex | No | | | |
| OpenCode | No; only spilled full tool outputs | A fixed 7 days | None | The fixed 7 days |
| Droid | Only orphaned `/btw` side questions and empty drafts | A fixed 7 days | None | The fixed 7 days |
| Pi | No | | | |
| Cursor | No | | | |
| Grok | Only when set | Idle longer than the TTL, when a session starts | `[storage] cleanup_ttl_days` in `~/.grok/config.toml` (unset or 0: off) | Yes |
| Antigravity | No age or count rule; trims by size | When a conversation outgrows its storage limit | None | |
| Hermes | Only when set | Ended and inactive longer than the retention, at startup, at most once a day | `sessions.auto_prune` (default false) and `sessions.retention_days` (default 90) in `~/.hermes/config.yaml` | Yes |
| T3 Code | No (threads); its event log, yes | Every 6 hours, events older than about a day | None (hard-coded) | Not needed: Distill reads the thread tables, not the event log |
| Goose | Only empty sessions | When Goose Desktop starts | None | |

## Claude Code

- **Prunes:** yes. A background sweep after a session starts, at most once per session (it waits at least 10 minutes if another session swept in the last day), deletes everything in the config folder older than `cleanupPeriodDays`: session transcripts in `projects/<project>/*.jsonl` and their set-aside copies, each session's `subagents/`, `tool-results/` and `mcp-tasks/`, and `file-history/`, `plans/`, `debug/`, `paste-cache/`, `session-env/`, `tasks/`, `backups/`. Auto-memory files are kept. It is skipped in `claude -p --bare`, and paused while a settings file can't be parsed.
- **Setting:** `cleanupPeriodDays`, integer days, default 30, minimum 1 (0 fails validation; the docs suggest 3650 to keep sessions for a long time). Desktop and Cowork sessions are kept at any age unless `desktopSessionCleanupPeriodDays` is set (default 0, no limit); a managed `cleanupPeriodDays` applies to them too. Both can be set in `~/.claude/settings.json`, a project's `.claude/settings.json` or `.claude/settings.local.json`, `--settings`, or managed settings, which win.
- **Where:** `CLAUDE_CONFIG_DIR` moves the whole `~/.claude` folder. Managed settings: `managed-settings.json` and `managed-settings.d/*.json` in `/Library/Application Support/ClaudeCode/` (macOS) or `/etc/claude-code/` (Linux), an MDM profile (`com.anthropic.claudecode`), or server-managed settings.
- **Status reads:** `settings.json` beside `projects/`, then the managed files, later ones winning, and takes the shorter of the two periods. It does not read project settings, MDM profiles or server-managed settings.
- **Compaction:** appends to the same file; the summarised messages stay. The exception is `CLAUDE_CODE_TRANSCRIPT_LOCAL_GC=1` (only for `-p` and Agent SDK runs), which drops history before each compaction once a file passes 5 MB. `--no-session-persistence` and `CLAUDE_CODE_SKIP_PROMPT_HISTORY=1` stop transcripts being written at all; `claude purge` deletes on demand.
- Sources: [claude-directory](https://code.claude.com/docs/en/claude-directory), [settings reference](https://code.claude.com/docs/en/settings-reference#cleanupperioddays), [sessions](https://code.claude.com/docs/en/sessions), [env vars](https://code.claude.com/docs/en/env-vars), [managed settings](https://code.claude.com/docs/en/managed-settings).

## Codex

- **Prunes:** no. Rollouts in `~/.codex/sessions` are deleted only when the owner deletes a thread; archiving moves them to `archived_sessions/`.
- **Rewrites:** with the feature flag `[features] local_thread_store_compression` (under development, default off), rollouts untouched for 7 days are compressed to `<name>.jsonl.zst` at startup and the plain file is deleted; resuming converts one back. The flag `background_paginated_rollout_migration` (default off) and `codex migrate-rollouts --apply` rewrite a legacy rollout in place. Distill parses both `.jsonl` and `.jsonl.zst`, and the sweep keeps each as its own capture.
- **Compaction:** appends a `compacted` item to the same rollout. `[history] max_bytes` trims `history.jsonl`, which is prompt history, not sessions.
- **Where:** `CODEX_HOME` (default `~/.codex`) holds `config.toml`; there are also MDM, `/etc/codex/config.toml`, profile and project layers.
- Sources: [compression.rs](https://github.com/openai/codex/blob/main/codex-rs/rollout/src/compression.rs), [features](https://github.com/openai/codex/blob/main/codex-rs/features/src/lib.rs), [rollout migration](https://github.com/openai/codex/blob/main/codex-rs/thread-store/src/local/rollout_migration.rs), [config loader](https://github.com/openai/codex/blob/main/codex-rs/config/src/loader/mod.rs).

## OpenCode

- **Prunes:** no. Sessions in `opencode.db` are deleted only when the owner removes one.
- **Side files:** a tool output longer than 2,000 lines or 50 KB (by default; `tool_output` in its config) is spilled whole to `<data>/tool-output/tool_<id>`, beside `opencode.db`; the tool part keeps a preview and names the file in `state.metadata.outputPath`. OpenCode deletes `tool_*` files whose mtime is older than a fixed 7 days, checking every hour while it runs. Not configurable.
- **Status reads:** the fixed 7 days, so it warns if the timer is set to sweep less often than weekly. The sweep captures the spilled files in `tool-output/` beside the database it reads, each at its own origin (since #107). With any privacy exclusion set, it skips a file that no session names yet (retried next sweep) or that an excluded session names. The parse step puts the full output in the tool result that names it, for sessions in the tables Distill reads (OpenCode's v1 `session`, `message`, `part`).
- **Compaction:** v2 appends a `compaction` message row (`compaction.auto`, `compaction.buffer`, `compaction.keep.tokens`). In v1 (up to 1.19.5), `compaction.prune` (default false) marked old tool outputs as cleared for the model but kept them stored. The first v2 run migrates the v1 tables in place.
- **Where:** `$XDG_DATA_HOME/opencode/opencode.db` (default `~/.local/share/opencode/`), or `OPENCODE_DB`; config in `~/.config/opencode/opencode.json`.
- Sources: [tool-output.ts](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/tool-output.ts), [file-retention.ts](https://github.com/anomalyco/opencode/blob/dev/packages/core/src/file-retention.ts), [database-path.ts](https://github.com/anomalyco/opencode/blob/dev/packages/cli/src/database-path.ts), [v1 truncate.ts](https://github.com/anomalyco/opencode/blob/v1.19.6/packages/opencode/src/tool/truncate.ts), [v1 tool.ts](https://github.com/anomalyco/opencode/blob/v1.19.6/packages/opencode/src/tool/tool.ts), [v1 processor.ts](https://github.com/anomalyco/opencode/blob/v1.19.6/packages/opencode/src/session/processor.ts), [v1 compaction.ts](https://github.com/anomalyco/opencode/blob/v1.19.5/packages/opencode/src/session/compaction.ts), [config docs](https://opencode.ai/docs/config/).

## Droid

- **Prunes:** ordinary sessions, no. Two kinds of file are deleted: an empty draft session (no content, no parent, no title) is deleted with its `.settings.json`, and the daemon deletes orphaned `/btw` side-question forks in `~/.factory/sessions/btw/` once they are 7 days old. Neither is configurable. Sessions whose worktree is removed are archived, not deleted.
- **Setting:** `sessionRetentionDays` (14 to 365) is an organisation setting for Factory's cloud sessions; nothing in the CLI deletes local sessions by it.
- **Status reads:** the fixed 7 days, so it warns if the timer is set to sweep less often than weekly.
- **Compaction:** appends a `compaction_state` record; since v0.227.0 earlier messages stay in the transcript.
- **Where:** `~/.factory/settings.json` and `settings.local.json`, project `.factory/`, and a system file in `/Library/Application Support/Factory/` or `/etc/factory/`.
- **Confidence:** medium. Read from the docs and the strings in the 1.12.1 release, since the CLI is closed source.
- Sources: [settings](https://docs.factory.ai/cli/configuration/settings), [enterprise controls](https://docs.factory.com/enterprise/hierarchical-settings-and-org-control.md), [changelog](https://docs.factory.ai/changelog/cli-updates).

## Pi

- **Prunes:** no. The docs say to delete session files by hand, or with Ctrl+D in `/resume`.
- **Rewrites:** opening a session written in a format older than v3 migrates it and rewrites the whole file in place. Branching and forking write new files.
- **Compaction:** appends a `compaction` entry and keeps the original entries (`compaction.enabled`, default true).
- **Where:** `~/.pi/agent/settings.json` (`PI_CODING_AGENT_DIR`); sessions can be moved with `--session-dir`, `PI_CODING_AGENT_SESSION_DIR` or the `sessionDir` setting.
- Sources: [sessions.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/sessions.md), [session-format.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/session-format.md), [compaction.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/compaction.md), [settings.md](https://github.com/badlogic/pi-mono/blob/main/packages/coding-agent/docs/settings.md).

## Cursor

- **Prunes:** no retention setting is documented, and nothing in the 2026.11.13 CLI deletes chat stores on its own. Chats are deleted from the `agent ls` picker. `conversations_retention_period_days` belongs to the server-side team admin API.
- **Rewrites:** before v2026.06.29 the CLI rewrote a whole transcript at each checkpoint; since then it appends new entries. What `/summarize` (`/compact`) does to stored data is not documented.
- **Where:** `~/.cursor/cli-config.json` (`CURSOR_CONFIG_DIR`, or `$XDG_CONFIG_HOME/cursor/` on Linux).
- **Confidence:** medium to low. Read from the docs and the bundled JavaScript.
- Sources: [CLI changelog](https://cursor.com/docs/cli/changelog.md), [CLI configuration](https://cursor.com/docs/cli/reference/configuration.md).

## Grok

- **Prunes:** only when set. Once per process, when a session starts, Grok deletes a session folder whose newest file is older than the TTL (`chat_history.jsonl`, `updates.jsonl`, `compaction_checkpoints/`, plans), and in every session, the live one too, media, downloads and terminal logs older than the TTL. The live session folder is never deleted.
- **Setting:** `[storage] cleanup_ttl_days` in `config.toml` in Grok's home (`GROK_HOME`, default `~/.grok`), integer days. Unset or 0 turns it off. Before v1.0.38 an unset value meant 30 days, so an older install deletes sessions after 30 days without the key.
- **Status reads:** `config.toml` beside `sessions/`.
- **Rewrites:** `chat_history.jsonl`, the file Distill parses, is replaced whole (temp file then rename) on compaction and on rewind. `updates.jsonl` is the append-only log of everything and is archived as a sidecar.
- This is xAI's Grok Build (`xai-org/grok-build`). The community `superagent-ai/grok-cli` keeps sessions in `~/.grok/grok.db` and never deletes them; Distill doesn't read that file.
- Sources: [persistence.rs](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-shell/src/session/persistence.rs), [config.rs](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-shell/src/agent/config.rs), [config reference](https://github.com/xai-org/grok-build/blob/main/crates/codegen/xai-grok-pager/docs/user-guide/26-config-reference.md), [settings reference](https://docs.x.ai/build/settings/reference).

## Antigravity

- **Prunes:** no age or count rule is documented, and Gemini CLI's `general.sessionRetention` doesn't apply: Gemini CLI cleans only `~/.gemini/tmp/<project>/chats`.
- **Trims by size:** the CLI changelog describes truncating trajectories to a size budget (v1.1.13), force-clearing a session that exceeds its storage limit (v1.2.6) and trimming older history (v1.4.9), and the desktop app has reported "The server cleared a prefix of the conversation as it grew too large." No setting controls it. Whatever is trimmed between two sweeps is lost.
- **Setting:** none for conversations. `historySize` in `~/.gemini/antigravity-cli/settings.json` trims the recall list of typed prompts.
- **Confidence:** medium. Antigravity is closed source.
- Sources: [settings reference](https://antigravity.google/docs/cli/reference), [CLI changelog](https://github.com/google-antigravity/antigravity-cli/blob/main/CHANGELOG.md), [forum report](https://discuss.ai.google.dev/t/google-antigravity-clears-the-conversation-history-automatically-if-its-too-large/187476).

## Hermes

- **Prunes:** only when set; off by default ("deleting history is opt-in"). At startup of the CLI, gateway or cron, at most once per `min_interval_hours` (default 24), Hermes deletes ended sessions inactive longer than `retention_days`, from `state.db` and their files in `~/.hermes/sessions/`. Open, pinned and mid-turn sessions are kept. When the disk is low and the key is absent, Hermes asks once whether to turn it on.
- **Setting:** in `config.yaml` in Hermes's home (`HERMES_HOME`, default `~/.hermes`), under `sessions:`: `auto_prune` (default false), `retention_days` (default 90; 0 deletes every ended session at the next startup; negative skips the sweep), `min_interval_hours` (24). `auto_archive` only hides sessions.
- **Status reads:** `config.yaml` beside `sessions/` and `state.db`.
- **Rewrites:** compression either starts a child session and keeps the parent, or rewrites rows in place, flagging the old ones `compacted` and keeping them; tool-output pruning and rewind keep archived copies.
- **Manual:** `hermes sessions prune` (90 days unless filtered).
- Sources: [config_defaults.py](https://github.com/NousResearch/hermes-agent/blob/main/hermes_cli/config_defaults.py), [hermes_state_maintenance.py](https://github.com/NousResearch/hermes-agent/blob/main/hermes_state_maintenance.py), [sessions.md](https://github.com/NousResearch/hermes-agent/blob/main/website/docs/user-guide/sessions.md).

## T3 Code

- **Prunes:** threads, no. They go only when the owner deletes a thread or its project.
- **Event log:** every 6 hours T3 Code prunes `orchestration_events` in `statev2.sqlite` to about a day, hard-coded. Distill reads the thread projection tables (`orchestration_v2_projection_*`), which are kept, so this loses nothing Distill captures. Those rows are updated in place.
- **Setting:** `storageCleanup` in `userdata/settings.json` covers worktrees, browser artifacts and logs, never threads. `T3CODE_HOME` sets the base folder (default `~/.t3`).
- Sources: [ProjectionMaintenance.ts](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/orchestration-v2/ProjectionMaintenance.ts), [settings.ts](https://github.com/pingdotgg/t3code/blob/main/packages/contracts/src/settings.ts), [storageCleanup.ts](https://github.com/pingdotgg/t3code/blob/main/apps/server/src/storageCleanup.ts).

## Goose

- **Prunes:** no age or count rule. When Goose Desktop starts, it deletes sessions with no messages, no name and no recipe; there is nothing in them to lose.
- **Rewrites:** compaction deletes a session's message rows and inserts them again, the originals kept as visible to the user only, followed by a summary for the agent. Rewinding and forking delete messages after a point.
- **Where:** `~/.local/share/goose/sessions/sessions.db`, or `<GOOSE_PATH_ROOT>/data/sessions/sessions.db`. Session files from before v1.10.0 are imported into the database and left on disk.
- Sources: [session_manager.rs](https://github.com/block/goose/blob/main/crates/goose/src/session/session_manager.rs), [context_mgmt](https://github.com/block/goose/blob/main/crates/goose/src/context_mgmt/mod.rs), [App.tsx](https://github.com/block/goose/blob/main/ui/desktop/src/App.tsx), [paths.rs](https://github.com/block/goose/blob/main/crates/goose/src/config/paths.rs).

## Not covered

- Status reads the owner's own settings files and, for Claude Code, the managed settings files. It does not read project-level settings (a project's `.claude/settings.json` can set a shorter `cleanupPeriodDays`), MDM profiles or server-managed settings.
- Antigravity's size trimming and every in-place rewrite above have no setting to read, so status can't warn about them.
- Sessions OpenCode 2 writes only to its new tables (`session_v2`, `session_message`) are not archived yet, nor their spilled outputs attached (#110).
- OpenCode's spilled outputs are looked for only beside the database Distill reads. With `OPENCODE_DB` pointing elsewhere, they stay in OpenCode's data folder and are not captured.
- These findings are as of the date above. A harness can change its defaults in any release; recheck when one does.
