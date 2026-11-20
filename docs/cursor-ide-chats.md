# Cursor's IDE chat stores (`state.vscdb`)

Shapes only, read by field names (Cursor, November 2026). For #108.

## Where

- `~/Library/Application Support/Cursor/User/globalStorage/state.vscdb`: SQLite. Every chat from the IDE's chat panel, for every workspace.
- `~/Library/Application Support/Cursor/User/workspaceStorage/<id>/state.vscdb`: one per workspace. `workspace.json` beside it names the folder. These hold UI state and prompt history only.

## The global store

Tables: `ItemTable` and `cursorDiskKV` (both `key TEXT, value BLOB`), and `composerHeaders`.

- **`composerData:<composerId>`** (JSON): one per chat (Cursor calls a chat a *composer*). It holds:
  - `composerId`, `createdAt` (ms), `unifiedMode` (`agent` or `chat`), `isAgentic`, `modelConfig`, `status`, `subComposerIds` and `subagentComposerIds`;
  - `fullConversationHeadersOnly`: the chat's messages in order, each `{bubbleId, type}`.

  Drafts have an empty list.
- **`bubbleId:<composerId>:<bubbleId>`** (JSON): one per message. It holds:
  - `type`: 1 is the owner and 2 is the agent;
  - `text`, `createdAt`;
  - `thinking` on reasoning, `toolFormerData` on a tool call and its result, `modelInfo`;
  - many context fields (attached files, diffs, lints, rules).
- **`composerHeaders`** (`composerId, workspaceId, createdAt, lastUpdatedAt, isArchived, isSubagent, …, value`): an index of chats by workspace. `workspaceId` is the `workspaceStorage` folder's name, so a chat's working directory is found through that folder's `workspace.json`.
- `agentKv:<…>`, `checkpointId:<…>`: agent state and checkpoints, mostly not JSON.

## The workspace stores

`ItemTable` keys such as:

- `composer.composerData`: which chats belong to the workspace;
- `aiService.prompts`: prompt history, the owner's text only, no replies;
- `aiService.generations`;
- `workbench.panel.*` view state.

No messages are kept here. The older `workbench.panel.aichat.view.aichat.chatdata` layout is absent.

## Decision

DISTILL does not read `state.vscdb`. When it was checked (#108), every chat in it with messages had the same id as an agent transcript DISTILL already archives (`~/.cursor/projects/<project>/agent-transcripts/`), and the rest were empty drafts. The transcripts keep the owner's messages, the agent's replies and its tool calls, but no tool results, reasoning or models, and no other archived store holds those chats. What the store alone adds is that tool output, reasoning and model names, which was too little to justify a parser.

Archiving the file as a capture has two more costs: Cursor rewrites it whenever it runs, so the append-only raw archive would keep copy after copy; and it is a SQLite file, which fails closed while a privacy exclusion is set.

**Revisit when** a chat with messages turns up that has no agent transcript (a `chat`-mode chat, say), or when the transcripts' missing tool output starts to matter. Then parse the store as a database export, one composer at a time (`…#composer=<id>`): events from its `bubbleId` records in `fullConversationHeadersOnly` order, and the working directory from `composerHeaders.workspaceId` through `workspace.json`.
