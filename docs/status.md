# Status against the plan

As of 2026-11-13. Milestones are from `docs/plan.md`. "Built" means implemented with tests; exit checks that need elapsed time or daily use are marked as not yet met.

| # | Milestone | Built | Exit check |
| --- | --- | --- | --- |
| 1 | Capture | Sweep for all six sources, append vs. rewrite detection, content-addressed blobs, manifest, `verify`, privacy exclusions, a writer lock so overlapping sweeps can't race, fsync before anything is superseded, launchd/systemd timer (`distill schedule`). | Idempotent re-sweep and clean verify are tested and were checked on real data (2,465 captures). "Runs unattended for a week" is not met yet. |
| 2 | Use it by hand | MCP server and `distill search` / `sessions` make the archive askable. | The list of ten real questions has not been written. |
| 3 | Normalize | Parsers for Claude Code, Codex, OpenCode, Droid and Pi with full trajectories (tool calls, results, reasoning) and git outcome joins. Cursor is archived only. | On real data: 1,976 sessions normalized, 0 failures, 421 header-only captures recorded as empty. |
| 4 | Steward | Cards via `claude -p` / `codex exec` with no tools; a daily dollar cap for Claude (failed calls count) and a daily call limit for harnesses that don't report cost; cards go stale when their session changes; run log; steward-session tagging; `tick` runs it when due. | Checked on real sessions (3 cards, $0.005). "Cards within a day with no human action" needs the timer running. |
| 5 | GUI | Rune setup (5 steps; Finish writes `config.toml`, adds the default recipe and installs the timer) and dashboard with approvals; headless snapshot and runtime tests. | Snapshots checked, including a real-time render against a real home; a clean-machine setup run through the window has not been done. |
| 6 | Datasets | Recipes, `chat-messages` and `trajectory` formats, secret redaction, build ids hashed from the recipe and the output bytes (so new cards, parser fixes or rules mean a new build to approve), credential-named fields redacted inside tool inputs, approval gate, publish; a new build removes older unapproved builds of the same recipe. | Byte-identical rebuild, planted-secret catch and approval gate are tested; a real Pi build redacted 394 secrets with none of the checked patterns left. |
| 7 | Agent-repaired parsers | Repair brief from failing captures; `distill steward repair` starts the agent on its own branch of a Distill checkout; merging is the approval. | Tested with a stub agent only. |

## Known gaps

- Format drift that drops records silently is not detected: parsers skip unknown lines, so only outright parse failures reach the repair queue.
- The steward does not propose new recipes yet; it rebuilds the ones in `recipes/`.
- First normalize runs git once or twice per session (about 75 s for 2,000 sessions); later runs skip unchanged captures.
- No t3code join: its database was locked when checked and its contents are unverified.
- The Cursor chat format is not parsed. Cursor chats don't record their repository, so they are not archived at all while any privacy exclusion is set.
- Each new capture costs two directory fsyncs (about 18 ms), so a first sweep of a large machine takes a few minutes.
