# Distill Desktop Accessibility Human Validation

This checklist is intentionally not a screen-reader CI claim. The automated renderer
suite proves keyboard event wiring, semantic names, focus return, contrast tokens,
reduced motion, and a 200% text-size DOM seam. The packaged macOS smoke proves AX focus
enters `Confirm destructive repair`, Tab remains contained, Escape closes, and focus
returns to `Repair library`. The installed Ubuntu package smoke proves the matching
AT-SPI focus containment and return. Neither packaged focus harness asserts
screen-reader speech. A human must still verify assistive-technology announcements in
the packaged desktop runtime, and signed/notarized release packaging remains a separate
gate.

## VoiceOver on macOS

Run against the packaged Tauri app (not only the Vite preview):

1. Turn on VoiceOver and navigate by headings, landmarks, forms, and buttons.
2. Confirm the first-run form announces labels, required fields, and the submit action.
3. Start a Sync Run, move to its live status, cancel it, and confirm the cancelled state is announced.
4. Open Repair library, confirm the dialog title and warning are announced, Tab stays within the dialog, Escape closes it, and focus returns to Repair library. Automated AX focus containment/return is already covered by `desktop:smoke:macos`; this step still requires human VoiceOver speech verification.
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
VoiceOver/Narrator speech remains a human release gate. Packaged macOS AX and Linux
AT-SPI focus assertions are recorded by their smokes, but do not replace screen-reader
validation or signed/notarized release evidence.
