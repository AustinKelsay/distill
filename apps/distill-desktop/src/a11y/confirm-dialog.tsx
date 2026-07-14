/**
 * Native modal confirmation dialog with accessible name, description, and focus return.
 */

import { useEffect, useId, useRef } from "react";
import { returnFocus } from "./focus-return";

type ConfirmDialogProps = {
  open: boolean;
  title: string;
  description: string;
  confirmLabel: string;
  cancelLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
  returnFocusTo: HTMLElement | null;
};

/**
 * Open a dialog with showModal when available, otherwise the open attribute (jsdom).
 * @param dialog - dialog element to open
 */
function openDialog(dialog: HTMLDialogElement): void {
  if (typeof dialog.showModal === "function") {
    if (!dialog.open) dialog.showModal();
    return;
  }
  dialog.setAttribute("open", "");
}

/**
 * Close a dialog with close() when available, otherwise remove the open attribute.
 * @param dialog - dialog element to close
 */
function closeDialog(dialog: HTMLDialogElement): void {
  if (typeof dialog.close === "function") {
    if (dialog.open) dialog.close();
    return;
  }
  dialog.removeAttribute("open");
}

/**
 * Render one native `<dialog>` confirm step with Escape/cancel and focus return.
 * @param props - controlled open state, copy, callbacks, and focus restore target
 */
export function ConfirmDialog({
  open,
  title,
  description,
  confirmLabel,
  cancelLabel = "Cancel",
  onConfirm,
  onCancel,
  returnFocusTo,
}: ConfirmDialogProps) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const cancelRef = useRef<HTMLButtonElement>(null);
  const titleId = useId();
  const descriptionId = useId();

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open) {
      openDialog(dialog);
      cancelRef.current?.focus();
      return;
    }
    closeDialog(dialog);
  }, [open]);

  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    /**
     * Map the native Escape/`cancel` event onto the controlled cancel path.
     * @param event - dialog cancel event
     */
    function onNativeCancel(event: Event) {
      event.preventDefault();
      onCancel();
      returnFocus(returnFocusTo);
    }
    dialog.addEventListener("cancel", onNativeCancel);
    return () => {
      dialog.removeEventListener("cancel", onNativeCancel);
    };
  }, [onCancel, returnFocusTo]);

  useEffect(() => {
    if (!open) return;
    const dialog = dialogRef.current;
    if (!dialog) return;
    const activeDialog: HTMLDialogElement = dialog;
    /**
     * Keep the fallback Escape path only when the runtime has no native modal API.
     * @param event - document keydown
     */
    function onKeyDown(event: KeyboardEvent) {
      if (event.key === "Escape") {
        if (typeof activeDialog.showModal === "function") return;
        event.preventDefault();
        onCancel();
        returnFocus(returnFocusTo);
        return;
      }
      if (event.key !== "Tab") return;
      const focusable = [
        cancelRef.current,
        activeDialog.querySelector<HTMLButtonElement>(
          'button:not([disabled]):not([data-dialog-cancel="true"])',
        ),
      ].filter((element): element is HTMLButtonElement => element !== null);
      if (focusable.length < 2) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [open, onCancel, returnFocusTo]);

  return (
    <dialog
      ref={dialogRef}
      className="confirm-dialog"
      aria-modal="true"
      aria-labelledby={titleId}
      aria-describedby={descriptionId}
      data-testid="repair-confirm-dialog"
    >
      <h2 id={titleId}>{title}</h2>
      <p id={descriptionId}>{description}</p>
      <div
        className="confirm-dialog-actions"
        role="group"
        aria-label="Repair confirmation"
      >
        <button
          ref={cancelRef}
          data-dialog-cancel="true"
          type="button"
          onClick={() => {
            onCancel();
            returnFocus(returnFocusTo);
          }}
        >
          {cancelLabel}
        </button>
        <button
          type="button"
          onClick={() => {
            onConfirm();
            returnFocus(returnFocusTo);
          }}
        >
          {confirmLabel}
        </button>
      </div>
    </dialog>
  );
}
