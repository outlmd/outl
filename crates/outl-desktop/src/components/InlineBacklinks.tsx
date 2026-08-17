import { For, Show } from "solid-js";

import {
  openRef,
  pageBacklinks,
  setBacklinksOrder,
  toggleTodo,
} from "@outl/shared/api/commands";
import { MarkdownInline } from "@outl/shared/markdown";
import { sameCrumbTrail } from "@outl/shared/outline";
import type { Backlink } from "@outl/shared/api/types";

import { appState, setAppState, setOutline } from "../lib/store";

/** What one click on a backlink's bullet does next. The toggle walks
 *  the cycle a step at a time, so the label names the *next* stop,
 *  not "done" from wherever it is. */
function backlinkTodoLabel(todo: Backlink["todo"]): string {
  if (todo === "DONE") return "Mark not done";
  if (todo === "DOING") return "Mark done";
  if (todo === "TODO") return "Mark doing";
  return "Mark as TODO";
}

/**
 * Inline backlinks section — rendered **below** the outline, not
 * as a side panel.
 *
 * Mirrors the TUI's `view::backlinks::render_backlinks_inline`:
 * a soft horizontal rule + a `Backlinks · N ref(s)` header, then
 * each referencing source page contributes a header (icon + title)
 * and one row per referencing block. Multiple backlinks from the
 * same source page collapse under one header — same UX as
 * `outl-tui` so a user moving between clients sees the same
 * structure.
 *
 * Hidden when:
 *
 * - `appState.backlinksOpen === false` (toggled with
 *   `Cmd/Ctrl+Shift+B`), or
 * - there are no backlinks for the current page (empty section
 *   doesn't earn its space).
 *
 * Each backlink row is **navigable**: vim `j/k` extends past the
 * outline's last block into this section (cursor lives at
 * `appState.selectedBacklinkBlockId`), and `Enter` opens the
 * source page positioned on the referencing block. Mouse click
 * does the same — both flows funnel through `openBacklink` so the
 * cursor lands at the same place no matter how the user
 * triggered the open.
 */
export function InlineBacklinks() {
  async function openBacklink(link: Backlink) {
    const target = link.source_page?.slug;
    if (!target) return;
    try {
      const view = await openRef(target);
      // Backlinks refetch via OutlineView's per-slug effect once the
      // new page's outline lands; the view no longer carries them.
      setAppState({ page: view.page });
      setOutline(view.outline);
      // Position cursor on the source block (the one we just came
      // from). Reset backlink cursor so j/k keep working in the
      // freshly-opened outline.
      setAppState("selectedBacklinkBlockId", null);
      setAppState("selectedBlockId", link.block_id);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  async function toggleBacklinkTodo(link: Backlink) {
    const sourcePage = link.source_page;
    const currentSlug = appState.page?.slug;
    if (!sourcePage || !currentSlug) return;
    try {
      await toggleTodo(sourcePage.id, link.block_id);
      // The mutation lands on the *source* page, so nothing on this page
      // re-renders on its own — and OutlineView's backlink effect is keyed
      // on the slug, which didn't change. Refetch the projection directly:
      // this is the one mutation that changes the current page's own
      // backlinks, the exception that effect's comment calls out.
      const r = await pageBacklinks(currentSlug);
      setAppState({ backlinks: r.backlinks, backlinksOrder: r.backlinks_order });
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  /** Flip newest ⇄ oldest, persist it, and swap in the re-sorted view
   *  the backend returns (issue #142). No-op with no page open. */
  async function toggleOrder() {
    const slug = appState.page?.slug;
    if (!slug) return;
    const next = appState.backlinksOrder === "newest" ? "oldest" : "newest";
    try {
      const r = await setBacklinksOrder(next, slug);
      setAppState({ backlinksOrder: r.backlinks_order, backlinks: r.backlinks });
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  /** Open the source page from a group header (jumps to the page,
   *  no per-block positioning). */
  async function jumpTo(sourceSlug: string | undefined) {
    if (!sourceSlug) return;
    try {
      const view = await openRef(sourceSlug);
      // Backlinks refetch via OutlineView's per-slug effect.
      setAppState({ page: view.page });
      setOutline(view.outline);
      setAppState("selectedBacklinkBlockId", null);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  /**
   * Group backlinks by their source page. `null` source (orphan
   * blocks with no enclosing page) collapse under a synthetic
   * `(orphan)` header.
   */
  function groupedBySource(): Array<{
    key: string;
    title: string;
    icon: string;
    entries: Backlink[];
  }> {
    const groups = new Map<
      string,
      { key: string; title: string; icon: string; entries: Backlink[] }
    >();
    for (const b of appState.backlinks) {
      const key = b.source_page?.slug ?? "__orphan__";
      const title = b.source_page?.title ?? "(orphan)";
      const fallback = b.source_page?.kind === "journal" ? "📅" : "📄";
      const icon = b.source_page?.icon || fallback;
      const existing = groups.get(key);
      if (existing) existing.entries.push(b);
      else groups.set(key, { key, title, icon, entries: [b] });
    }
    return [...groups.values()];
  }

  return (
    <Show when={appState.backlinksOpen && appState.backlinks.length > 0}>
      <section class="mt-6">
        {/* Soft full-width rule — mirrors the TUI's `─` separator. */}
        <div class="border-t border-(--color-outl-border) opacity-60" />

        <header class="mt-3 mb-2 flex items-baseline justify-between gap-2">
          <span class="text-xs font-semibold uppercase tracking-wide opacity-60">
            Backlinks · {appState.backlinks.length} ref
            {appState.backlinks.length === 1 ? "" : "s"}
          </span>
          {/* Direction toggle (issue #142). The arrow tracks the order:
              ↓ newest-on-top, ↑ oldest-on-top. */}
          <button
            type="button"
            onClick={() => void toggleOrder()}
            title={
              appState.backlinksOrder === "newest"
                ? "Newest first — click for oldest first"
                : "Oldest first — click for newest first"
            }
            class="flex items-center gap-1 rounded px-1.5 py-0.5 text-xs opacity-60 hover:bg-(--color-outl-fg)/5 hover:opacity-100"
          >
            <span aria-hidden="true">
              {appState.backlinksOrder === "newest" ? "↓" : "↑"}
            </span>
            <span>
              {appState.backlinksOrder === "newest" ? "Newest" : "Oldest"}
            </span>
          </button>
        </header>

        <div class="space-y-4">
          <For each={groupedBySource()}>
            {(group) => (
              <div>
                <button
                  type="button"
                  onClick={() =>
                    void jumpTo(
                      group.key === "__orphan__" ? undefined : group.key,
                    )
                  }
                  class="flex w-full items-baseline gap-2 rounded px-1 py-0.5 text-left text-sm font-semibold hover:bg-(--color-outl-fg)/5"
                >
                  <span aria-hidden="true">{group.icon}</span>
                  <span>{group.title}</span>
                  <span class="text-xs opacity-50">{group.entries.length}</span>
                </button>

                <ul class="mt-1 space-y-1 pl-6">
                  <For each={group.entries}>
                    {(link, index) => {
                      const selected = () =>
                        appState.selectedBacklinkBlockId === link.block_id;
                      // Breadcrumb of ancestor blocks as dimmed context.
                      // Collapsed against the previous entry in the same
                      // group: consecutive references in the same branch
                      // show the trail once, then sit under it silently.
                      const prev =
                        index() > 0 ? group.entries[index() - 1] : null;
                      const showCrumbs =
                        link.ancestors.length > 0 &&
                        (!prev ||
                          !sameCrumbTrail(prev.ancestors, link.ancestors));
                      const crumbTrail = link.ancestors
                        .map((c) => c.text)
                        .join(" › ");
                      return (
                        <li
                          // The selected state mirrors the outline's
                          // BlockRow highlight: 3px accent bar on the
                          // left + 6% background. Same visual
                          // language so j/k feels continuous from
                          // outline into backlinks.
                          class={
                            selected()
                              ? "relative -ml-3 rounded bg-(--color-outl-accent)/6 pl-3 before:absolute before:left-0 before:top-1 before:bottom-1 before:w-[3px] before:rounded-r before:bg-(--color-outl-accent)"
                              : ""
                          }
                        >
                          <Show when={showCrumbs}>
                            <div
                              class="truncate px-1 pt-0.5 text-xs opacity-40"
                              title={crumbTrail}
                            >
                              {crumbTrail}
                            </div>
                          </Show>
                          <div class="flex items-start">
                            <button
                              type="button"
                              data-todo={link.todo ?? "none"}
                              onClick={() => void toggleBacklinkTodo(link)}
                              class={`mt-[2px] mr-2 w-3 shrink-0 cursor-pointer text-center text-[13px] leading-none hover:opacity-70 ${
                                link.todo === "DONE"
                                  ? "text-(--color-outl-todo-done-fg)"
                                  : link.todo === "TODO" ||
                                      link.todo === "DOING"
                                    ? "text-(--color-outl-todo-open-fg)"
                                    : "text-(--color-outl-fg-dimmer)"
                              }`}
                              title={backlinkTodoLabel(link.todo)}
                              aria-label={backlinkTodoLabel(link.todo)}
                            >
                              {link.todo === "DONE"
                                ? "▣"
                                : link.todo === "DOING"
                                  ? "▨"
                                  : link.todo === "TODO"
                                    ? "▢"
                                    : "•"}
                            </button>
                            <button
                              type="button"
                              onClick={() => void openBacklink(link)}
                              onMouseEnter={() =>
                                setAppState(
                                  "selectedBacklinkBlockId",
                                  link.block_id,
                                )
                              }
                              class={`block min-w-0 flex-1 rounded px-1 py-0.5 text-left text-sm leading-snug opacity-90 hover:bg-(--color-outl-fg)/5 hover:opacity-100 ${
                                link.todo === "DONE"
                                  ? "line-through opacity-60"
                                  : ""
                              }`}
                            >
                              <MarkdownInline
                                tokens={link.source_block.tokens}
                                variant="inline"
                              />
                            </button>
                          </div>
                        </li>
                      );
                    }}
                  </For>
                </ul>
              </div>
            )}
          </For>
        </div>
      </section>
    </Show>
  );
}
