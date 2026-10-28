import { For, Show, createMemo, createResource } from "solid-js";
import { listOpenTasks, type TaskHit } from "@outl/shared/api/overview";
import { journalHeading } from "@outl/shared/journal";
import { groupTasksByPage } from "@outl/shared/tasks";

import { createSheetDrag } from "../lib/sheet-drag";

/**
 * The workspace's open tasks, grouped by the page they live on — the
 * first 200, with the uncapped total in the header ("200 of 1431
 * open").
 *
 * The list is the backend's (`list_open_tasks`), so what counts as a
 * task here is what draws a checkbox everywhere else, and the
 * sectioning is `groupTasksByPage` (`@outl/shared/tasks`), shared with
 * the desktop panel. Tapping a row opens its page zoomed into that
 * block; checking it off happens there, on the row itself, where the
 * context is.
 */
export function TasksSheet(props: {
  open: boolean;
  todaySlug: string | null;
  onClose: () => void;
  onOpenTask: (task: TaskHit) => void;
}) {
  const [data] = createResource(
    () => props.open || null,
    () => listOpenTasks().catch(() => ({ total: 0, tasks: [] as TaskHit[] })),
  );
  const groups = createMemo(() => groupTasksByPage(data()?.tasks ?? []));
  const label = (slug: string) => {
    const h = journalHeading(slug, props.todaySlug);
    return h ? `${h.title}, ${h.date}` : slug;
  };
  const drag = createSheetDrag(() => props.onClose());

  return (
    <Show when={props.open}>
      <div class="outl-fade-in fixed inset-0 z-50 bg-black/40 backdrop-blur-md" onClick={props.onClose} />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-50 flex flex-col overflow-hidden rounded-t-2xl bg-(--color-outl-bg)/90 shadow-2xl backdrop-blur-2xl backdrop-saturate-150"
        style={{
          top: "calc(env(safe-area-inset-top) + 48px)",
          "padding-bottom": "env(safe-area-inset-bottom)",
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
            <h2 class="text-[22px] font-semibold tracking-tight">Tasks</h2>
            <Show when={data()}>
              {(d) => (
                <span class="text-[13px] text-(--color-outl-fg-dim) tabular-nums">
                  {d().tasks.length < d().total
                    ? `${d().tasks.length} of ${d().total} open`
                    : `${d().total} open`}
                </span>
              )}
            </Show>
          </div>
        </header>
        <div class="ios-scroll flex-1 overflow-y-auto px-4 pb-6">
          <Show
            when={groups().length > 0}
            fallback={
              <Show when={!data.loading}>
                <p class="py-16 text-center text-[15px] text-(--color-outl-fg-dim)">
                  Nothing open. A block that starts with TODO shows up here.
                </p>
              </Show>
            }
          >
            <For each={groups()}>
              {(g) => (
                <section class="mt-4">
                  <h3 class="mb-1 text-[12px] font-medium uppercase tracking-wide text-(--color-outl-fg-dimmer)">
                    {label(g.slug)}
                  </h3>
                  <ul class="overflow-hidden rounded-xl bg-(--color-outl-bg-elev)">
                    <For each={g.tasks}>
                      {(t) => (
                        <li class="border-b border-(--color-outl-border)/50 last:border-b-0">
                          <button
                            type="button"
                            onClick={() => props.onOpenTask(t)}
                            class="flex w-full items-start gap-3 px-3 py-2.5 text-left active:bg-(--color-outl-border)/40"
                          >
                            <span
                              aria-hidden="true"
                              class="mt-[3px] h-[18px] w-[18px] shrink-0 rounded-full border-[1.5px] border-(--color-outl-todo-open-fg)"
                              classList={{ "bg-(--color-outl-todo-open-fg)/25": t.status === "doing" }}
                            />
                            <span class="min-w-0 flex-1 text-[15px] leading-snug">
                              {t.text || "Untitled task"}
                            </span>
                            <Show when={t.status === "doing"}>
                              <span class="shrink-0 text-[11px] font-medium uppercase text-(--color-outl-todo-open-fg)">
                                Doing
                              </span>
                            </Show>
                          </button>
                        </li>
                      )}
                    </For>
                  </ul>
                </section>
              )}
            </For>
          </Show>
        </div>
      </div>
    </Show>
  );
}
