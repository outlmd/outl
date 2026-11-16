import { JSX, Show } from "solid-js";

import { createSettle } from "../polish";
import { type SyncState, syncStateColor, syncStateLabel } from "./status";

/**
 * The sync indicator itself: a coloured dot, or a spinner while a pass runs.
 *
 * Pure render. Every client wraps it in its own affordance (the desktop's
 * click-to-sync / right-click-for-settings button, mobile's tap-to-sync with
 * press-and-hold for devices), and neither decides what the state *is* — that
 * is `syncState` in `./status`, so the colour and the sentence cannot drift
 * between the two.
 *
 * **It announces the state; the wrapping control announces the action.**
 * The label rides an `aria-live` region here rather than a `title=`, because
 * tooltips do not render in iOS WKWebView at all: without it a colour-blind
 * user gets a grey-ish dot and nothing else. So a client's button says what
 * pressing it does ("Sync now"), never what the dot currently shows, or the
 * two are read out together on every change.
 */
export function SyncDot(props: { state: SyncState }): JSX.Element {
  // One soft pulse when a pass ends, on both clients.
  const settling = createSettle(() => props.state === "syncing");
  return (
    <span
      role="status"
      aria-live="polite"
      aria-label={syncStateLabel(props.state)}
      class="inline-flex h-2.5 w-2.5 items-center justify-center"
    >
      <Show
        when={props.state === "syncing"}
        fallback={
          <span
            aria-hidden="true"
            data-testid="sync-dot"
            class={`h-2.5 w-2.5 rounded-full${settling() ? " outl-sync-settle" : ""}`}
            style={{ background: syncStateColor(props.state) }}
          />
        }
      >
        <span
          aria-hidden="true"
          data-testid="sync-spinner"
          class="h-2.5 w-2.5 animate-spin rounded-full border-2 border-(--color-outl-accent) border-t-transparent"
        />
      </Show>
    </span>
  );
}
