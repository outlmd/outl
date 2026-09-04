import { JSX, Show, createEffect, onCleanup } from "solid-js";

export interface ToastProps {
  /** Message text. `null` means hidden. */
  message: string | null;
  /** Optional retry handler — shown as a button beside the message. */
  onRetry?: () => void;
  /** Dismiss callback (called on tap-X or after auto-dismiss). */
  onDismiss: () => void;
  /**
   * Auto-dismiss after this many ms. Default 5s. Set `0` to keep
   * the toast pinned until the user explicitly dismisses (use for
   * blocking errors that need a real decision).
   */
  autoDismissMs?: number;
}

/**
 * iOS-style banner that slides in from the top safe-area. Replaces
 * the old "error sits as `<p>` in the middle of the outline"
 * pattern, which had no dismiss and no retry — just a wall of red
 * text until the next op succeeded.
 *
 * The toast is `role="status"` so screen readers announce the
 * message politely (without interrupting). It auto-dismisses after
 * `autoDismissMs` unless a retry handler is provided — retries
 * pin the toast so the user actually sees the affordance.
 */
export function Toast(props: ToastProps): JSX.Element {
  let timer: ReturnType<typeof setTimeout> | undefined;

  createEffect(() => {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    if (!props.message) return;
    const ms = props.autoDismissMs ?? 5000;
    // If there's a retry, keep it pinned — the affordance is the
    // whole point of the toast.
    if (ms <= 0 || props.onRetry) return;
    timer = setTimeout(props.onDismiss, ms);
  });

  onCleanup(() => {
    if (timer !== undefined) clearTimeout(timer);
  });

  // Swipe-up to dismiss — iOS in-app notification gesture. Track the
  // first touch Y, compare with current; if user pulled up more than
  // 24pt we treat it as a dismiss intent.
  let touchStartY: number | null = null;

  function onTouchStart(e: TouchEvent) {
    touchStartY = e.touches[0]?.clientY ?? null;
  }

  function onTouchMove(e: TouchEvent) {
    if (touchStartY === null) return;
    const y = e.touches[0]?.clientY ?? touchStartY;
    if (touchStartY - y > 24) {
      touchStartY = null;
      props.onDismiss();
    }
  }

  function onTouchEnd() {
    touchStartY = null;
  }

  return (
    <Show when={props.message}>
      <div
        role="status"
        aria-live="polite"
        class="pointer-events-none outl-toast-in fixed inset-x-0 top-0 z-50 flex justify-center"
        style="padding-top: max(env(safe-area-inset-top), 12px);"
      >
        {/* Pill-shaped in-app notification, Apple/Bear style: capsule
            rounded-full, blurred translucent bg, shadow elevation,
            swipe-up to dismiss. Sits centered horizontally with a
            small horizontal margin so on narrow screens it stays
            inset from the edges. */}
        <div
          onTouchStart={onTouchStart}
          onTouchMove={onTouchMove}
          onTouchEnd={onTouchEnd}
          onTouchCancel={onTouchEnd}
          class="pointer-events-auto mx-4 flex max-w-md items-center gap-3 rounded-full bg-(--color-outl-destructive)/95 px-4 py-2.5 text-white shadow-[var(--shadow-capsule)] backdrop-blur-xl"
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
            stroke-linejoin="round"
            class="shrink-0"
            aria-hidden="true"
          >
            <circle cx="12" cy="12" r="10" />
            <path d="M12 8v4M12 16h.01" />
          </svg>
          <p class="flex-1 truncate text-[14px] leading-snug">
            {props.message}
          </p>
          <Show when={props.onRetry}>
            <button
              type="button"
              onClick={() => {
                props.onRetry?.();
                props.onDismiss();
              }}
              class="shrink-0 rounded-full bg-white/20 px-3 py-1 text-[13px] font-semibold active:bg-white/30"
            >
              Retry
            </button>
          </Show>
          <button
            type="button"
            aria-label="Dismiss"
            onClick={props.onDismiss}
            class="shrink-0 rounded-full p-1 active:bg-white/20"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="2.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </div>
      </div>
    </Show>
  );
}
