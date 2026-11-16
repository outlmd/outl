import { For, Show, createResource } from "solid-js";

import { createBlock, editBlock, pageTimeline } from "@outl/shared/api/commands";
import type { PageView, TimelineEvent } from "@outl/shared/api/types";
import { findBlock } from "@outl/shared/outline";
import { DiffText, relativeAgo, restorePlan } from "@outl/shared/trust";

import { createSheetDrag } from "../lib/sheet-drag";
import { haptic } from "../lib/haptics";

/**
 * A page's history, from the op log: every device's edits as a word
 * diff, plus the one-tap restores `restorePlan` allows (previous text of
 * an edit, or the text of a deleted block brought back as a new block).
 * A restore is an ordinary edit / create, so it is new ops in the log,
 * it syncs, and undo takes it back. The desktop's `TimelinePanel` is the
 * same surface in a modal.
 *
 * Opened from the page properties sheet (long-press the title).
 */
export function HistorySheet(props: {
  pageId: string | null;
  view: PageView | null;
  onClose: () => void;
  onView: (view: PageView) => void;
  onMessage: (text: string) => void;
}) {
  const [timeline, { refetch }] = createResource(
    () => props.pageId ?? undefined,
    (id) => pageTimeline(id),
  );
  const drag = createSheetDrag(() => props.onClose());
  const current = (block: string) =>
    props.view ? (findBlock(props.view.outline, block)?.text ?? undefined) : undefined;

  const restore = async (event: TimelineEvent) => {
    const plan = restorePlan(event, current(event.block));
    if (!props.pageId || !plan) return;
    haptic("medium");
    try {
      const view =
        plan.kind === "edit"
          ? await editBlock(props.pageId, plan.block, plan.text)
          : (await createBlock(props.pageId, { text: plan.text })).view;
      props.onView(view);
      props.onMessage(plan.kind === "edit" ? "Version restored" : "Text brought back");
      void refetch();
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <Show when={props.pageId}>
      <div class="outl-fade-in fixed inset-0 z-50 bg-black/40 backdrop-blur-md" onClick={props.onClose} />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-50 flex flex-col overflow-hidden rounded-t-2xl bg-(--color-outl-bg)/90 shadow-2xl backdrop-blur-2xl backdrop-saturate-150"
        style={{
          top: "calc(var(--safe-area-inset-top) + 48px)",
          "padding-bottom": "var(--safe-area-inset-bottom)",
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging() ? "none" : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <header class="px-4 pt-3 pb-2">
          <span
            class="mx-auto block h-3 w-16 cursor-grab py-1"
            style={{ "touch-action": "none" }}
            onPointerDown={drag.onPointerDown}
            onPointerMove={drag.onPointerMove}
            onPointerUp={drag.onPointerUp}
            onPointerCancel={drag.onPointerCancel}
            aria-label="Drag to close"
            role="button"
          >
            <span aria-hidden="true" class="mx-auto block h-1 w-10 rounded-full bg-(--color-outl-border)" />
          </span>
          <div class="mt-2 flex items-baseline justify-between">
            <h2 class="text-[22px] font-semibold tracking-tight">History</h2>
            <Show when={timeline()}>
              {(t) => (
                <span class="text-[13px] text-(--color-outl-fg-dim) tabular-nums">
                  {t().truncated ? `${t().events.length} of ${t().total}` : `${t().total}`} changes
                </span>
              )}
            </Show>
          </div>
        </header>
        <div class="ios-scroll flex-1 overflow-y-auto px-4 pb-6">
          <Show when={!timeline.error} fallback={<p class="py-6 text-center text-[15px] opacity-60">Could not read the history: {String(timeline.error)}</p>}>
            <Show when={timeline()} fallback={<p class="py-6 text-center text-[15px] opacity-60">Reading the op log…</p>}>
              {(t) => (
                <Show when={t().events.length > 0} fallback={<p class="py-6 text-center text-[15px] opacity-60">Nothing in the op log for this page yet.</p>}>
                  <For each={t().events}>
                    {(e) => {
                      const plan = () => restorePlan(e, current(e.block));
                      return (
                        <div class="border-b border-(--color-outl-border)/40 py-3 last:border-b-0">
                          <div class="flex items-center gap-2 text-[13px] text-(--color-outl-fg-dim)">
                            <span>{relativeAgo(e.at_ms)}</span>
                            <span>· {e.change === "edited" && e.from === null ? "written" : e.change}</span>
                            <Show when={plan()}>
                              {(p) => (
                                <button
                                  type="button"
                                  class="ml-auto rounded-full bg-(--color-outl-accent)/12 px-3 py-1 text-[13px] font-medium text-(--color-outl-accent) active:opacity-60"
                                  onClick={() => void restore(e)}
                                >
                                  {p().kind === "edit" ? "Restore" : "Bring back"}
                                </button>
                              )}
                            </Show>
                          </div>
                          <Show when={e.change === "edited"}>
                            <div class="mt-1 whitespace-pre-wrap break-words text-[15px]">
                              <DiffText from={e.from ?? ""} to={e.to ?? ""} />
                            </div>
                          </Show>
                          <Show when={e.change === "deleted"}>
                            <div class="mt-1 text-[15px] text-(--color-outl-fg-dim) line-through">{e.text}</div>
                          </Show>
                        </div>
                      );
                    }}
                  </For>
                </Show>
              )}
            </Show>
          </Show>
        </div>
      </div>
    </Show>
  );
}
