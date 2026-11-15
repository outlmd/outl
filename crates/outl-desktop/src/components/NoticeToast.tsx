import { For, Show, createEffect, onCleanup } from "solid-js";

import { appState, setAppState } from "../lib/store";

/** Long enough to read a property-conflict line, short enough not to linger. */
const NOTICE_MS = 12_000;

/**
 * Floating outcome toast, same corner and tier as `<ErrorToast />`, in
 * the plain foreground colour: it reports what a page rename / merge
 * did, line by line. Each new notice restarts the timer; the close
 * button dismisses it early.
 */
export function NoticeToast() {
  createEffect(() => {
    if (!appState.notice) return;
    const timer = setTimeout(() => setAppState("notice", null), NOTICE_MS);
    onCleanup(() => clearTimeout(timer));
  });
  return (
    <Show when={appState.notice}>
      {(lines) => (
        <div
          role="status"
          class="pointer-events-none fixed right-4 bottom-12 z-50 flex max-w-sm justify-end"
        >
          <div class="pointer-events-auto flex items-start gap-2 rounded-lg border border-(--color-outl-border)/60 bg-(--color-outl-bg-elev) px-3 py-2 text-xs text-(--color-outl-fg) shadow-lg">
            <div class="min-w-0 break-words">
              <For each={lines()}>{(line) => <div>{line}</div>}</For>
            </div>
            <button
              type="button"
              onClick={() => setAppState("notice", null)}
              class="ml-1 shrink-0 opacity-70 hover:opacity-100"
              aria-label="Dismiss"
            >
              ✕
            </button>
          </div>
        </div>
      )}
    </Show>
  );
}
