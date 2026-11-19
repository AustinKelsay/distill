# Cursor's IDE chat stores (`state.vscdb`)

Shapes only, from the owner's Mac (Cursor, November 2026), read by field names and counts. No values were read. For #108.

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

## What it held on the owner's Mac

- 42 chats: 27 in `agent` mode and 15 in `chat` mode.
- Only 4 chats have any messages: 102 in all, 4 from the owner and 98 from the agent.
- Each of those 4 has the same id as an agent transcript Distill already archives (`~/.cursor/projects/<project>/agent-transcripts/`). The other 38 are empty drafts, including all 15 in `chat` mode.
- The workspace stores hold 4 history prompts in all.

## Decision

Distill does not read `state.vscdb` for now. The 4 chats with messages are archived through Cursor's agent transcripts, which the IDE writes for its chat panel too, but not completely.

- **Archived:** the owner's messages, the agent's replies and its tool calls.
- **Not archived:** a transcript holds no tool results, reasoning or models.
- **Elsewhere:** no other archived store holds those chats, since none of the 4 is in `~/.cursor/chats`.

What the store alone holds is the tool results and reasoning of 4 chats (36 tool records and 31 reasoning records in all). That is too little to justify a parser now.

Archiving it as a capture has two more costs:

- the file is rewritten whenever Cursor runs, so the append-only raw archive would keep copy after copy;
- it is a SQLite file, which fails closed while a privacy exclusion is set.

**Revisit when** either of these is true:

- a chat with messages turns up that has no agent transcript (for example a `chat`-mode chat once the owner uses that mode);
- the IDE's chats hold enough tool results or reasoning that the transcripts' gap matters. Then parse it as a database export, one composer at a time (`…#composer=<id>`):

- events from its `bubbleId` records in `fullConversationHeadersOnly` order;
- the working directory from `composerHeaders.workspaceId` through `workspace.json`.
