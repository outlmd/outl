import { For, Show } from "solid-js";
import type { TrashEntry } from "@outl/shared/api/trash";
import type { PageView } from "@outl/shared/api/types";
import { createTrashRestore, deletedAgo } from "@outl/shared/trust";

import { haptic } from "../lib/haptics";
import { createSheetDrag } from "../lib/sheet-drag";

/**
 * Every block deleted out of this workspace, from any device, and the
 * way back (issue 455).
 *
 * Nothing here decides what can be restored or what to say about it:
 * `entry.refusal` is the backend's verdict worded for this surface, a
 * refused row is rendered rather than hidden, and the retry / notice
 * rules live in `createTrashRestore` (`@outl/shared/trust`), shared with
 * the desktop panel. A restore that lands applies the landing page
 * through `onView`, which is the navigation, so the sheet closes.
 */
export function TrashSheet(props: {
  open: boolean;
  onClose: () => void;
  onView: (view: PageView) => void;
  onMessage: (text: string) => void;
}) {
  const t = createTrashRestore({
    open: () => props.open,
    onView: (v) => props.onView(v),
    onError: (m) => props.onMessage(m),
  });
  const data = t.rows;
  const drag = createSheetDrag(() => props.onClose());

  async function restore(entry: TrashEntry) {
    haptic("light");
    if (await t.restore(entry)) props.onClose();
    else haptic("warning");
  }

  return (
    <Show when={props.open}>
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
            <h2 class="text-[22px] font-semibold tracking-tight">Trash</h2>
            <Show when={data()}>
              {(rows) => (
                <span class="text-[13px] text-(--color-outl-fg-dim) tabular-nums">
                  {rows().length} deleted
                </span>
              )}
            </Show>
          </div>
        </header>
        <div class="ios-scroll flex-1 overflow-y-auto px-4 pb-6">
          <Show
            when={(data()?.length ?? 0) > 0}
            fallback={
              <Show when={!data.loading}>
                <p class="py-16 text-center text-[15px] text-(--color-outl-fg-dim)">
                  Nothing has been deleted in this workspace. A deleted block comes
                  here instead of disappearing, from any of your devices.
                </p>
              </Show>
            }
          >
            <ul class="mt-2 overflow-hidden rounded-xl bg-(--color-outl-bg-elev)">
              <For each={data()}>
                {(entry) => (
                  <li class="flex items-start gap-3 border-b border-(--color-outl-border)/50 px-3 py-2.5 last:border-b-0">
                    <div class="min-w-0 flex-1">
                      <p class="truncate text-[15px] leading-snug">
                        <Show when={entry.page} fallback={entry.text}>
                          {(slug) => <>page <code class="font-mono">{slug()}</code></>}
                        </Show>
                      </p>
                      <p class="mt-0.5 text-[12px] text-(--color-outl-fg-dim)">
                        {deletedAgo(entry.deleted_at_ms)}
                        <Show when={entry.blocks > 1}>
                          {" · "}
                          {entry.blocks} blocks
                        </Show>
                      </p>
                      {/* `||`, so an empty-string refusal still falls
                          through to the attempt's notice. */}
                      <Show when={entry.refusal || t.notice(entry)}>
                        {(why) => (
                          <p class="mt-1 text-[12px] leading-snug text-(--color-outl-todo-open-fg)">{why()}</p>
                        )}
                      </Show>
                    </div>
                    <Show
                      when={!entry.refusal}
                      fallback={
                        <span class="shrink-0 pt-0.5 text-[11px] font-medium uppercase text-(--color-outl-fg-dimmer)">
                          Can't restore
                        </span>
                      }
                    >
                      <button
                        type="button"
                        disabled={t.busy() === entry.id}
                        class="flex min-h-11 min-w-11 shrink-0 items-center justify-center rounded-lg px-3 text-[15px] font-medium text-(--color-outl-accent) active:bg-(--color-outl-border)/40 disabled:opacity-50"
                        onClick={() => void restore(entry)}
                      >
                        {t.busy() === entry.id ? "Restoring…" : "Restore"}
                      </button>
                    </Show>
                  </li>
                )}
              </For>
            </ul>
          </Show>
        </div>
      </div>
    </Show>
  );
}
