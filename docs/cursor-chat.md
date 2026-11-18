# Archive a chat from the Cursor IDE

In Cursor, right-click the chat tab and choose **Export Transcript**. Create
`~/.cursor/chat-exports/` and save the Markdown file directly in it. Use a
distinct filename for each chat and reuse that name when exporting its later
turns. Distill's next sweep and parse pick it up. With custom Cursor roots,
include that folder in the roots list.

The export records a title and export time, then blocks marked **User** and
**Cursor**. The export time does not date the conversation. Exports lack a
working directory, so the usual rule for Cursor applies. If a project-path
exclusion is active, Distill skips capture. Only Markdown files directly in
the export folder are picked up there. Cursor's companion JSON and the
Downloads folder are outside that import.

## Storage decision for #108

Cursor documents local SQLite storage and offers Markdown export for
preservation. In the installed IDE, composer metadata is keyed by conversation
ID. Its internal storage also holds application state. Distill would need to
identify the right conversations, join them to messages across versioned
layouts and establish a workspace for exclusions. For occasional chat-panel
use, Distill accepts the IDE's own transcript and leaves `state.vscdb` alone.
The export has no stable conversation ID for matching against agent captures,
so exporting an already archived chat can yield two search results.

[Cursor chat history](https://docs.cursor.com/en/agent/chat/history)
