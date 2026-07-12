# Distill Desktop Accessibility Human Validation

This checklist is intentionally not a CI claim. The automated renderer suite proves
keyboard event wiring, semantic names, focus return, contrast tokens, reduced motion,
and a 200% text-size DOM seam. A human must still verify assistive-technology behavior
in the packaged desktop runtime when packaging work lands.

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
This checklist remains a human gate until the macOS and Linux packaging tickets provide
signed/runtime smoke coverage.
