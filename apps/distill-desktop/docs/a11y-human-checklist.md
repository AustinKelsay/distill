# Distill Desktop Accessibility Human Validation

This checklist is intentionally not a screen-reader CI claim. The automated renderer
suite proves keyboard event wiring, semantic names, focus return, contrast tokens,
reduced motion, and a 200% text-size DOM seam. The installed Ubuntu package smoke also
proves AT-SPI focus containment and return for the repair dialog, without asserting
screen-reader output. A human must still verify assistive-technology behavior in the
packaged desktop runtime.

## VoiceOver on macOS

Run against the packaged Tauri app (not only the Vite preview):

1. Turn on VoiceOver and navigate by headings, landmarks, forms, and buttons.
2. Confirm the first-run form announces labels, required fields, and the submit action.
3. Start a Sync Run, move to its live status, cancel it, and confirm the cancelled state is announced.
4. Open Repair library, confirm the dialog title and warning are announced, Tab stays within the dialog, Escape closes it, and focus returns to Repair library.
5. Load Sessions, search, change the workflow lane, select a session, and confirm the selected state and detail heading are announced.
6. Toggle a curation label, add a tag, and remove a tag; confirm the control names describe each action.
7. Preview and publish an export, cancel a pending publication, and confirm progress, cancellation, and focus return are announced.
8. Repeat at 200% text size and with Reduce Motion enabled; confirm no control or status is unavailable.

## Windows Narrator (when a Windows build exists)

Repeat the same flow with Narrator, checking dialog announcements, live status updates,
focus return, and keyboard-only curation/export actions. Record any WebView-specific
differences in the issue or release notes; do not convert this checklist into an automated
pass claim without a supported packaged test harness.

## Evidence

Record OS version, screen reader version, app build, packaging mode, and any failures.
The VoiceOver/Narrator steps and packaged macOS dialog-focus behavior remain human
release gates. Linux AT-SPI focus assertions are recorded by the packaged smoke, but do
not replace screen-reader validation or signed/notarized release evidence.
