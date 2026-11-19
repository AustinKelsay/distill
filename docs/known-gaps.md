# Known gaps

What Distill does not do, or does only partly. Update this page when a change adds or removes a limit.

## Capture

- With any privacy exclusion set, a harness whose sessions record no working directory is not archived at all: Antigravity, Hermes, and Cursor (its stores and transcripts). Neither is a T3 Code thread whose worktree and project root are unknown, nor new native references for it.
- Cursor's IDE chat store (`state.vscdb`) is not read: its chats with messages are also agent transcripts Distill archives, which keep no tool results or reasoning (`docs/cursor-ide-chats.md`). Cursor's SDK agent stores (`projects/*/sdk-agent-store`, how T3 Code runs Cursor) encrypt their blobs, so they are archived but not parsed; their agents are covered by Cursor's transcripts and the T3 threads that ran them.
- OpenCode's spilled tool outputs are looked for only in `tool-output/` beside the database Distill reads. With `OPENCODE_DB` set elsewhere they are not captured.
- Format drift inside a record type a parser knows (a renamed field) is not counted; only a capture that then parses to nothing shows it.
- A config that lists every harness Distill knew when it was written gets new harnesses switched on; one that lists only some gets nothing added.
- Each new capture costs two directory fsyncs (about 18 ms), so a first sweep of a large machine takes a few minutes.

## Findings

- Names, postal addresses, phone numbers and companies are found only by the model pass, only in what the owner and the agent wrote, and only once it has run. A detail cut across two parts of a long session can be missed. Personal details are not masked for a trusted reader.
- Findings go out of date when the fingerprint key changes: a lost key means every session is scanned again, and old fingerprints match nothing new.
- A model reply that is not the expected JSON fails under the strict parse; that session is retried after the rest.

## Search

- Words longer than 64 characters are not indexed, and there is no phrase or prefix search.
- Search by meaning embeds only what the owner wrote and the card summary. Mirrors and steward sessions get no vectors, so `--mirrors` and `--steward` find them by words only. Each search embeds the query in Ollama and reads the whole vectors file.
- A card written by a tick is embedded in the same tick but joins the word index at the next one, because the index step runs before the card step.
- Measured on the owner's archive (#117): asked the way the owner would say it, 16 of 24 sessions the owner started were in the top five. That set is the yardstick for any search change.

## Safety checks

- The exposure check sees listeners only through `lsof`, and only Distill's own processes running at the time; without `lsof` that rule is unchecked. On Linux the disk counts as encrypted only on a dm-crypt device. The timer's unit files sit outside the home with the usual modes; they hold paths, not session data.
- The pruning check reads the owner's own harness settings and Claude Code's managed settings, not project-level settings, MDM profiles or server-managed settings. Antigravity's trimming by size and in-place rewrites have no setting to read (`docs/harness-pruning.md`).
- Whether a model is stored on this machine is read from `OLLAMA_MODELS`, else `~/.ollama/models`.

## The window

- Findings are covered with a chip naming their kind rather than blurred (ADR 0007). The list's chips are not clickable; a finding is shown only in an open session. A covered secret copies as its chip, a shown one as its value.
- The session view has no search inside a session and no jump to the passage a search matched; it reads the whole session at once. A session opened from Findings or Approvals is headed by its key, not its card.
- Approvals shows only pending requests; answered, collected and expired ones are in the read log. A request's expiry is recorded when the next call touches requests.
- Setup cannot move the home, and does not set `sessions_per_run`.
- A previous build's `recipes/`, `exports/` and `steward/` folders in the home are reported by the exposure check and otherwise ignored, never deleted.
