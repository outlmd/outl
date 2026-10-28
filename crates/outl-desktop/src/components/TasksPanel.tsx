/**
 * Task list panel — the workspace's open tasks, grouped by the page
 * they live on. `list_open_tasks` caps the rows at 200 and reports the
 * uncapped total, which the header shows ("200 of 1431 open"), so the
 * list says what it is missing; the tail is not reachable from here.
 *
 * Open with `Cmd/Ctrl+Shift+L` (Global) or `g t` in Normal mode; close
 * with `Esc`. Read-only: picking a row opens its page with the cursor
 * on that block, and checking it off happens there, where the context
 * is. The phone's Tasks tab (`TasksSheet.tsx`) is the same list.
 *
 * **Nothing about what counts as a task is decided here.** The rows and
 * their order come from `list_open_tasks` (started tasks first, then
 * the latest journal days, then named pages), so a block that draws a
 * checkbox anywhere else shows up here. The sectioning is
 * `groupTasksByPage` (`@outl/shared/tasks`), shared with the phone's
 * sheet because this client and that one had the same bug in their own
 * copies of it; it collects, and never re-ranks.
 */

import { For, Show, createMemo, createResource } from "solid-js";

import { openPageBySlug, todaySlug } from "@outl/shared/api/commands";
import { listOpenTasks, type OpenTasks, type TaskHit } from "@outl/shared/api/overview";
import type { PageView } from "@outl/shared/api/types";
import { journalHeading } from "@outl/shared/journal";
import { findBlock } from "@outl/shared/outline";
import { groupTasksByPage } from "@outl/shared/tasks";

import { appState, setAppState } from "../lib/store";
import { focusFirstRowOnOpen, rovingKeydown } from "../lib/keyboard-reach";

export interface TasksPanelDeps {
  applyView: (view: PageView) => void;
  setError: (msg: string) => void;
}

export function TasksPanel(props: TasksPanelDeps) {
  // Gated on the panel being open, for the reason `RemindersPanel`
  // gives: the component is mounted for the app's whole life, so an
  // ungated `createResource` would scan the workspace at boot.
  const open = () => (appState.tasksOpen ? true : undefined);

  const [data] = createResource(open, async () => {
    try {
      return await listOpenTasks();
    } catch (e) {
      props.setError(e instanceof Error ? e.message : String(e));
      return { total: 0, tasks: [] } as OpenTasks;
    }
  });
  // Journal headings are relative ("Yesterday"), so they need today.
  const [today] = createResource(open, () => todaySlug().catch(() => null));

  const groups = createMemo(() => groupTasksByPage(data()?.tasks ?? []));

  const label = (slug: string) => {
    const h = journalHeading(slug, today() ?? null);
    return h ? `${h.title}, ${h.date}` : slug;
  };

  const close = () => setAppState("tasksOpen", false);

  // Arrow navigation needs focus inside the list first; `RemindersPanel`
  // lands it through the same helper, which owns both of its rules.
  let list: HTMLDivElement | undefined;
  focusFirstRowOnOpen(() => appState.tasksOpen, data, () => list, "[data-task-row]");

  async function jumpTo(t: TaskHit) {
    try {
      const view = await openPageBySlug(t.source_slug);
      props.applyView(view);
      setAppState("selectedBlockId", t.id);
      // A task with sub-blocks opens zoomed into itself so it is the
      // whole view; a leaf stays in page context, cursor on it. Same
      // rule as a block hit in the picker.
      const node = findBlock(view.outline, t.id);
      if (node && node.children.length > 0) setAppState("focusBlockId", t.id);
      close();
    } catch (e) {
      // The page can be gone (deleted on another device since the list
      // loaded); closing would hide the reason along with the row.
      props.setError(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <Show when={appState.tasksOpen}>
      <div
        class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
        onClick={close}
      >
        <div
          class="mt-16 max-h-[80vh] w-[680px] max-w-[92vw] overflow-hidden rounded-lg border border-(--color-outl-border) bg-(--color-outl-bg-elev) shadow-2xl"
          onClick={(e) => e.stopPropagation()}
        >
          <header class="flex items-baseline justify-between border-b border-(--color-outl-border) px-5 py-3">
            <h2 class="text-lg font-semibold">Tasks</h2>
            <span class="text-xs opacity-50">
              <Show when={data()}>
                {(d) => (
                  <>
                    {d().tasks.length < d().total
                      ? `${d().tasks.length} of ${d().total} open`
                      : `${d().total} open`}
                    {" · "}
                  </>
                )}
              </Show>
              Esc to close
            </span>
          </header>

          <div
            ref={list}
            data-tasks-list
            class="max-h-[64vh] overflow-y-auto px-5 py-3"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                close();
                return;
              }
              rovingKeydown(e, "vertical");
            }}
          >
            <Show
              when={groups().length > 0}
              fallback={
                <Show when={!data.loading}>
                  <p class="py-6 text-center text-sm opacity-60">
                    Nothing open. A block that starts with <code class="font-mono">TODO</code> shows up here.
                  </p>
                </Show>
              }
            >
              <For each={groups()}>
                {(g) => (
                  <section class="mb-5">
                    <h3 class="mb-2 text-sm font-semibold uppercase tracking-wide text-(--color-outl-help-title-fg)">
                      {label(g.slug)}
                    </h3>
                    <For each={g.tasks}>
                      {(t) => (
                        <div class="flex items-start gap-3 border-b border-(--color-outl-border)/40 py-2 last:border-b-0">
                          <span
                            aria-hidden="true"
                            class="mt-1 h-3.5 w-3.5 shrink-0 rounded-full border-[1.5px] border-(--color-outl-todo-open-fg)"
                            classList={{ "bg-(--color-outl-todo-open-fg)/25": t.status === "doing" }}
                          />
                          <button
                            type="button"
                            data-task-row
                            class="block min-w-0 flex-1 truncate text-left text-sm hover:underline"
                            onClick={() => void jumpTo(t)}
                          >
                            {t.text || "Untitled task"}
                          </button>
                          <Show when={t.status === "doing"}>
                            <span class="shrink-0 pt-0.5 text-xs font-medium uppercase text-(--color-outl-todo-open-fg)">
                              Doing
                            </span>
                          </Show>
                        </div>
                      )}
                    </For>
                  </section>
                )}
              </For>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
}
