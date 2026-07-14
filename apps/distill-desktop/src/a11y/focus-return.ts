/**
 * Deterministic focus restoration after cancel controls unmount or dialogs close.
 */

/**
 * Restore keyboard focus to a stable trigger after the active control disappears.
 * @param target - element that should receive focus, or null to no-op
 */
export function returnFocus(target: HTMLElement | null | undefined): void {
  if (!target) return;
  window.setTimeout(() => {
    target.focus();
  }, 0);
}
