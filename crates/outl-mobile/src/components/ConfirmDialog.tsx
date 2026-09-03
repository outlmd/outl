import { Show, JSX, createEffect, onCleanup } from "solid-js";
import { haptic } from "../lib/haptics";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  message: string;
  /** Label for the destructive action button. */
  confirmLabel?: string;
  /** Label for the cancel button. */
  cancelLabel?: string;
  onConfirm: () => void;
  onCancel: () => void;
}

/**
 * iOS-style confirmation alert. Mirrors `UIAlertController`'s
 * structure (centered card, blurred backdrop, title + message,
 * stacked horizontal Cancel / destructive buttons with a hairline
 * divider). Solid + Tailwind because spinning up another native
 * UIKit bridge for one prompt isn't worth the polling cost.
 *
 * The dialog blocks taps to the underlying content via a full-screen
 * backdrop. Cancel also fires when the backdrop is tapped, mirroring
 * iOS sheet conventions.
 */
export function ConfirmDialog(props: ConfirmDialogProps): JSX.Element {
  // Side effects are tied to `props.open`, not to component mount.
  // The component is mounted for the lifetime of its parent and toggled
  // via `open`, so mount-time effects would fire haptic("warning") at
  // app boot and leave a global keydown listener registered forever.
  createEffect(() => {
    if (!props.open) return;
    haptic("warning");
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onCancel();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <Show when={props.open}>
      <div
        class="fixed inset-0 z-[60] flex items-center justify-center bg-black/40 backdrop-blur-[2px] px-8"
        onClick={(e) => {
          if (e.currentTarget === e.target) props.onCancel();
        }}
      >
        <div
          class="w-full max-w-[280px] overflow-hidden rounded-2xl bg-(--color-outl-bg-elev)/95 backdrop-blur-2xl shadow-2xl"
          role="alertdialog"
          aria-modal="true"
          onClick={(e) => e.stopPropagation()}
        >
          <div class="px-4 pt-4 pb-3 text-center">
            <h2 class="text-[17px] font-semibold text-(--color-outl-fg)">
              {props.title}
            </h2>
            <p class="mt-1.5 text-[13px] leading-tight text-(--color-outl-fg-dim)">
              {props.message}
            </p>
          </div>
          <div class="grid grid-cols-2 border-t border-(--color-outl-border)/40">
            <button
              type="button"
              class="border-r border-(--color-outl-border)/40 py-2.5 text-[17px] font-normal text-(--color-outl-accent) active:bg-(--color-outl-border)/20"
              onClick={() => {
                haptic("light");
                props.onCancel();
              }}
            >
              {props.cancelLabel ?? "Cancel"}
            </button>
            <button
              type="button"
              class="py-2.5 text-[17px] font-semibold text-(--color-outl-destructive) active:bg-(--color-outl-border)/20"
              onClick={() => {
                haptic("medium");
                props.onConfirm();
              }}
            >
              {props.confirmLabel ?? "Delete"}
            </button>
          </div>
        </div>
      </div>
    </Show>
  );
}
