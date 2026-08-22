/**
 * Page history — what the op log says happened to the page that's open.
 *
 * Opened by the `⏱` button in the page header, closed with `Esc` or a
 * click outside. **Read-only**: it shows the past, it does not restore
 * it. `outl recover` is the one restore path that exists, and it covers
 * a narrow, provably-additive case (see `outl_actions::recover`); a
 * general "put this revision back" button needs its own safety
 * argument, which this panel deliberately does not assume.
 *
 * Not the undo stack. `Cmd+Z` walks *this session's* mutations;
 * this walks every device's, from the beginning of the workspace.
 *
 * Nothing here decides what an event is. `outl_actions::timeline` owns
 * which blocks count as the page's (including the ones deleted out of
 * it), which ops are not events at all (fold, snooze, page-model
 * bookkeeping, an edit that changed nothing), and the order. This
 * renders the answer.
 */

import { For, Show, createResource } from "solid-js";

import { pageTimeline } from "@outl/shared/api/commands";
import type { TimelineEvent } from "@outl/shared/api/types";

import { appState, setAppState } from "../lib/store";

export interface TimelinePanelDeps {
  setError: (message: string) => void;
}

/**
 * `at_ms` is the wall-clock half of the event's HLC. The *ordering*
 * came from the backend (HLC, actor-tiebroken) and is already applied,
 * so this is presentation only — a device with a skewed clock can show
 * a time that looks out of order, and re-sorting on it here would put
 * the events in the wrong order to make the labels look right.
 */
function formatAt(ms: number): string {
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return `ms:${ms}`;
  return d.toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** First line only — a block can be many lines and a row is one. */
function firstLine(text: string | null): string {
  if (!text) return "";
  const line = text.split("\n")[0] ?? "";
  return line;
}

/** A short label for the kind of change, used as the row's eyebrow. */
function label(event: TimelineEvent): string {
  switch (event.change) {
    case "created":
      return "created";
    case "edited":
      return event.from === null ? "written" : "edited";
    case "deleted":
      return "deleted";
    case "restored":
      return "restored from trash";
    case "moved":
      return "moved";
    case "property":
      return event.to === null ? `${event.key} cleared` : `${event.key} set`;
    // A `Change` variant added backend-side renders as its own tag
    // rather than as a blank row. The CLI's renderer does the same.
    default:
      return event.change;
  }
}

export function TimelinePanel(props: TimelinePanelDeps) {
  // Gated on the panel being open AND a page being loaded: the read
  // walks every block in the page under the workspace lock, which is
  // not something to do on every navigation on the chance the user
  // might open the panel.
  const target = () =>
    appState.timelineOpen && appState.page ? appState.page.id : undefined;

  // The error is re-thrown, not swallowed into `null`. Returning `null`
  // leaves `timeline.error` unset and the `Show` below falsy forever, so
  // a failed IPC call parks the panel on "Reading the op log…" while the
  // real message goes to a toast the user may already have dismissed.
  const [timeline] = createResource(target, async (pageId) => {
    try {
      return await pageTimeline(pageId);
    } catch (e) {
      props.setError(e instanceof Error ? e.message : String(e));
      throw e;
    }
  });

  const failed = () => timeline.error as unknown;

  return (
    <Show when={appState.timelineOpen}>
      <div
        class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
        onClick={() => setAppState("timelineOpen", false)}
      >
        <div
          class="mt-16 max-h-[80vh] w-[680px] max-w-[92vw] overflow-hidden rounded-lg border border-(--color-outl-border) bg-(--color-outl-bg-elev) shadow-2xl"
          onClick={(e) => e.stopPropagation()}
        >
          <header class="flex items-baseline justify-between border-b border-(--color-outl-border) px-5 py-3">
            <h2 class="text-lg font-semibold">
              History
              <Show when={timeline()}>
                {(t) => (
                  <span class="ml-2 font-mono text-sm font-normal opacity-60">
                    {t().slug}
                  </span>
                )}
              </Show>
            </h2>
            <span class="text-xs opacity-50">Esc to close</span>
          </header>

          <div class="max-h-[64vh] overflow-y-auto px-5 py-3">
            <Show when={!failed()} fallback={
              <p class="py-6 text-center text-sm opacity-60">
                Could not read the history: {String(failed())}
              </p>
            }>
            <Show
              when={timeline() && !timeline.loading}
              fallback={
                <p class="py-6 text-center text-sm opacity-60">
                  Reading the op log…
                </p>
              }
            >
              <Show
                when={(timeline()?.events ?? []).length > 0}
                fallback={
                  <p class="py-6 text-center text-sm opacity-60">
                    Nothing in the op log for this page yet.
                  </p>
                }
              >
                <For each={timeline()?.events ?? []}>
                  {(event) => <Row event={event} />}
                </For>
              </Show>
            </Show>
            </Show>
          </div>

          {/* A capped list that doesn't say so reads as the whole
              history, which is the one thing a history must not do. */}
          <Show when={timeline()?.truncated}>
            <footer class="border-t border-(--color-outl-border) px-5 py-2 text-xs opacity-60">
              Showing the {timeline()?.events.length} most recent of{" "}
              {timeline()?.total} changes. For the rest, run{" "}
              <code class="font-mono">
                outl page history {timeline()?.slug} --limit 500
              </code>
              .
            </footer>
          </Show>
        </div>
      </div>
    </Show>
  );
}

function Row(props: { event: TimelineEvent }) {
  const e = () => props.event;
  return (
    <div class="border-b border-(--color-outl-border)/40 py-2 last:border-b-0">
      <div class="flex items-baseline gap-2 text-xs opacity-60">
        <span class="font-mono">{formatAt(e().at_ms)}</span>
        <span>{label(e())}</span>
        {/* A row about a block that no longer exists is the row people
            come here for — say so rather than leaving them to click a
            block that isn't there. */}
        <Show when={e().block_deleted && e().change !== "deleted"}>
          <span class="opacity-70">· block since deleted</span>
        </Show>
      </div>

      <Show when={e().change === "edited"}>
        {/* `!== null`, not truthiness: an edit from empty text to
            content carries `from: ""`, which is a real previous value. */}
        <Show when={e().from !== null}>
          <div class="mt-0.5 truncate font-mono text-[13px] text-(--color-outl-fg-dim) line-through decoration-1">
            {firstLine(e().from) || "(empty)"}
          </div>
        </Show>
        <div class="truncate font-mono text-[13px]">{firstLine(e().to)}</div>
      </Show>

      {/* The text a deletion took is the whole reason to look. */}
      <Show when={e().change === "deleted"}>
        <div class="mt-0.5 truncate font-mono text-[13px] text-(--color-outl-fg-dim) line-through decoration-1">
          {firstLine(e().text)}
        </div>
      </Show>

      <Show when={e().change === "property" && e().to}>
        <div class="mt-0.5 truncate font-mono text-[13px]">{e().to}</div>
      </Show>
    </div>
  );
}
