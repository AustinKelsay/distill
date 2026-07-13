# Accessibility And Visual-State Contract

This specification governs the rebuild renderer. The React surface is a thin caller:
it exposes typed Library outcomes through semantic controls and never gains filesystem,
process, or database authority.

## Keyboard and focus

- Search and workflow-lane controls submit through a native form and remain usable with
  Enter.
- Session rows, curation labels, tag add/remove actions, dialogs, Sync Run cancellation,
  and export cancellation are native keyboard controls.
- A pending request exposes an explicit Cancel action. Cancellation invalidates the
  renderer request and returns focus to the initiating control after the state update.
- Repair uses a native `dialog` with an accessible name and description, initial focus on
  Cancel, Escape cancellation, and a two-control Tab loop. Native modal behavior is used
  when available; the renderer fallback covers the test runtime.

## Semantics and status

- Each major surface is a named landmark or group. Controls have visible labels or an
  explicit accessible name; tag removal names include the tag value.
- Loading and refreshing panels expose `aria-busy`. Status and progress text use polite
  live regions; actionable failures use `role="alert"`.
- Session exploration distinguishes first-load `loading` from row-preserving
  `refreshing`, then `empty`, `ready`, `warning`, `error`, or user `cancelled`.

## Visual states and resilient presentation

- Renderer tests keep deterministic evidence for first-run idle, loading, refreshing,
  empty, populated, warning, error, cancelled, migration, and export states.
- Text and controls use scalable `rem` sizing and remain present at a 200% root text
  size. Focus is visible with `:focus-visible`; disabled controls use contrast-preserving
  color tokens rather than opacity alone.
- `prefers-reduced-motion: reduce` disables transitions, animations, and smooth scrolling.
- Body/panel text and interactive color tokens meet the 4.5:1 normal-text contrast target.

## Runtime limits and honest evidence

The checked-in `a11y:smoke` command builds the renderer and runs the deterministic Vitest
contracts. It is not a signed or packaged WebView test and does not claim screen-reader
verification. The macOS package gate (`npm run desktop:package:macos` followed by
`npm run desktop:smoke:macos`) launches the local ad-hoc `.app` through macOS
Accessibility and proves the packaged search/detail/curation/export journey, quit/relaunch
artifact persistence, and packaged repair-dialog focus state: AX focus enters
`Confirm destructive repair`, Tab remains contained, Escape closes the dialog, and focus
returns to `Repair library`. Those checks are Accessibility focus-state evidence only;
they do not convert Accessibility automation into VoiceOver coverage. The Linux packaged
gate has Ubuntu CI evidence in #36/#39 (AT-SPI dialog focus/cancellation alongside the
install journey), and assistive-technology speech validation remains in
`apps/distill-desktop/docs/a11y-human-checklist.md`.
The Linux package gate mirrors the journey under Ubuntu/Xvfb/dbus, locating controls
through the AT-SPI accessible tree and activating them with `xdotool`. It is an
install/runtime smoke rather than screen-reader or AT-SPI conformance evidence.
