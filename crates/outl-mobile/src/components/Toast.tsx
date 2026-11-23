import { JSX, Show, createEffect, onCleanup } from "solid-js";

/**
 * Which pill this is.
 *
 * `error` is the destructive fill with a warning glyph; `success` is the
 * accent fill with a check, for a confirmation like "Renamed. 12 links
 * updated.". Both are `Palette` fill *pairs* with a measured contrast
 * (`destructive`/`destructive_fg`, `accent`/`accent_fg`), so no theme
 * can make either unreadable — which is why `accent_alt`, the palette's
 * nominal "success" hue, is not used: it is authored as ink and has no
 * `_fg` partner.
 */
export type ToastTone = "error" | "success";

export interface ToastProps {
  /** Message text. `null` means hidden. */
  message: string | null;
  /**
   * Default `error`. Before this existed the pill was unconditionally
   * destructive, so a rename summary was announced in the colour a
   * failure wears — one token carrying two meanings (root `CLAUDE.md`
   * invariant 13). `Journal.toast.ts` is the one place that decides
   * which tone a message gets.
   */
  tone?: ToastTone;
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

  // A summary with several lines (a merge report) has to show all of them,
  // since a property conflict is the line the user most needs to read.
  const multiline = () => (props.message ?? "").includes("\n");
  const success = () => props.tone === "success";

  createEffect(() => {
    if (timer !== undefined) {
      clearTimeout(timer);
      timer = undefined;
    }
    if (!props.message) return;
    const ms = props.autoDismissMs ?? (multiline() ? 12000 : 5000);
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
        style="padding-top: max(var(--safe-area-inset-top), 12px);"
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
          class="pointer-events-auto mx-4 flex max-w-md items-center gap-3 px-4 py-2.5 shadow-[var(--shadow-capsule)] backdrop-blur-xl"
          classList={{
            "rounded-full": !multiline(),
            "rounded-3xl": multiline(),
            "bg-(--color-outl-destructive)/95": !success(),
            "text-(--color-outl-destructive-fg)": !success(),
            "bg-(--color-outl-accent)/95": success(),
            "text-(--color-outl-accent-fg)": success(),
          }}
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
            {/* Colour is not the only carrier: at a glance the glyph is
                what says "done" rather than "wrong", and it is the half
                that survives a colourblind reader. */}
            <Show
              when={success()}
              fallback={
                <>
                  <circle cx="12" cy="12" r="10" />
                  <path d="M12 8v4M12 16h.01" />
                </>
              }
            >
              <path d="M20 6L9 17l-5-5" />
            </Show>
          </svg>
          <p
            class="flex-1 text-[14px] leading-snug"
            classList={{ truncate: !multiline(), "whitespace-pre-line": multiline() }}
          >
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
