import { For, Show, createMemo } from "solid-js";

import {
  backlinkRowOp,
  openRef,
  setBacklinksOrder,
} from "@outl/shared/api/commands";
import {
  backlinkCommitPlan,
  backlinkEditTarget,
} from "@outl/shared/backlink-edit";
import {
  CrumbTrail,
  crumbTrailText,
  MarkdownInline,
  MarkdownTable,
} from "@outl/shared/markdown";
import { lateSectionVisible, sameCrumbTrail } from "@outl/shared/outline";
import type { Backlink, PageKind } from "@outl/shared/api/types";
import { PageGlyph } from "@outl/shared/icons";

import { backlinkCursor, startBacklinkEdit } from "../lib/cursor-region";
import { resetForNavigation } from "../lib/page-change";
import { applyPageState } from "../lib/page-view";
import { trackBacklinks } from "../lib/backlinks-fetch";
import { appState, setAppState } from "../lib/store";
import { BacklinkRowEditor } from "./BacklinkRowEditor";

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
 * - `lateSectionVisible` says no (`@outl/shared/outline`, which owns
 *   the rule and the measurement): there are no rows in hand, so a
 *   first load draws nothing at all and an empty answer leaves nothing
 *   to take away. An empty section doesn't earn its space here, and a
 *   skeleton that collapses into one is the blink.
 *
 * Each backlink row is **navigable and editable**: vim `j/k` extends
 * past the outline's last block into this section, and a click, `i` or
 * `Enter` on a row edits the cited block **in place**, on the page it
 * lives on (`<BacklinkRowEditor />`). A row used to be a destination
 * instead — acting on one cost the user the very view that assembled
 * these references.
 *
 * Opening the source page is the **group header**, which is where it
 * already was. An orphan row (`source_page` null) has no page to address
 * and stays read-only; `backlinkEditTarget` is the one owner of that
 * verdict, shared with mobile.
 *
 * **Which row holds that cursor is `backlinkCursor()`'s answer, never
 * `appState.selectedBacklinkBlockId` directly.** The outline owns the
 * cursor until it releases it, so a writer that sets `selectedBlockId`
 * without clearing this field cannot leave two cursors on screen —
 * `lib/store.ts` carries the rule and the regression behind it. For the
 * same reason a *hover* is not a cursor move; see the row's own note.
 */
export function InlineBacklinks() {
  /**
   * Which row holds the keyboard cursor, resolved **once per render**
   * rather than once per row.
   *
   * The same shape `<OutlineView />` gives `focus()` and `<BlockRow />`
   * gives `visualSet`, and for the same reason: a per-row predicate is
   * O(N²) the moment anything inside it is not O(1), and one of
   * `backlinkCursor`'s ingredients is not. It asks whether the section
   * is on screen, which includes `zoomView()` — and that is
   * `focusSubtree(outline, focusBlockId)`, an O(page) walk, whenever
   * `focusBlockId` is set but the block it names has left the outline.
   *
   * That state is reachable and this branch made it expensive: a peer
   * deletes or moves the zoomed block, so `zoomView()` answers `null`,
   * `<PageSections zoomed>` is false and this section renders — with
   * `focusBlockId` still set, because the self-heal that clears it is a
   * `createEffect` and therefore runs *after* the render. One render,
   * measured on the busiest real page (1,556 rows, a 688-block
   * outline): **851 ms against 166 ms** to mount, and **724.7 ms
   * against 2.6 ms** for a single cursor move, which is a keystroke.
   * Linear in rows with the whole page as its constant.
   *
   * The memo removes the shape rather than the state: the walk runs
   * once, and each row's subscription set shrinks to this one signal.
   * **Reading `appState.focusBlockId` instead would be wrong**, not
   * merely cheaper — a stale id means "not zoomed" to what is drawn and
   * would mean "zoomed" here, which is a cursor in a section nothing
   * painted. `lib/cursor-region.ts` owns that rule and keeps it.
   */
  const cursor = createMemo(backlinkCursor);

  /**
   * The click on a row's bullet, the mouse twin of the keyboard's
   * `Cmd+T` (`backlink-handlers.ts`).
   *
   * **Both go through `backlinkCommitPlan`**, which is what stops the
   * two answering differently: a page can cite itself, and then the
   * reply *is* a fresh view of the page on screen. Discarding it there
   * left the outline's own checkbox stale while the keyboard path
   * refreshed it — one block showing two states on one screen,
   * depending on how the user had toggled it.
   *
   * It is still a second *call site* rather than a second owner: this
   * one targets the row the mouse hit, not `backlinkCursor()`'s.
   *
   * **`backlinkRowOp`, never `toggleTodo`** — the same rule the chord
   * follows, for the same reason. `toggle_todo` acts on the page the
   * user is looking at and reports a stale block id as a raw
   * `ActionError`, which is a ULID in the status line; the cross-page
   * command refuses in `outl_actions::refusal`'s words. A click on the
   * bullet of a row a peer has trashed is the commonest way to reach
   * that refusal, because it needs no keyboard.
   */
  async function toggleBacklinkTodo(link: Backlink) {
    const sourcePage = link.source_page;
    const currentSlug = appState.page?.slug;
    if (!sourcePage || !currentSlug) return;
    const plan = backlinkCommitPlan({
      sourcePageId: sourcePage.id,
      openPageId: appState.page?.id,
    });
    try {
      const view = await backlinkRowOp(
        sourcePage.id,
        link.block_id,
        "toggle-todo",
      );
      if (plan.applyView) applyPageState(view);
      // The mutation lands on the *source* page, so nothing on this page
      // re-renders on its own — and the backlinks effect is keyed on the
      // page, which didn't change. Refetch the projection directly: this
      // is one of the mutations that changes the current page's own
      // backlinks, the exception that effect's comment calls out.
      if (plan.refetchBacklinks) {
        await trackBacklinks();
      }
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

  /**
   * Open the source page from a group header (jumps to the page, no
   * per-block positioning).
   *
   * **Through `applyPageState`, like every other reply.** That function
   * is the single writer of what a `PageView` says about the page —
   * the page, its `key:: value` chips, the parser's warnings, and the
   * sticky "this page isn't syncing" banner — and this used to write
   * `page` + outline and nothing else, which is exactly the second
   * `applyView` the desktop paid to delete.
   *
   * The banner is why this is not cosmetic. `<PageAheadOfLogBanner />`
   * claims a page has stopped converging with the user's other
   * devices; carried over from the page they just left, it names the
   * wrong page as broken — and the page it names is healthy, so
   * nothing the user does makes it go away until the next reply that
   * does run through here.
   *
   * Pre-existing, and this branch is what made it reachable in one
   * gesture: `Enter` and the row click both became edit gestures, so
   * the group header is now the **only** door to the source page.
   *
   * `resetForNavigation()` first, the shape `<RemindersPanel />`'s own
   * `jumpTo` uses: this is a navigation, so the zoom and any backlink
   * cursor or open row editor go with it.
   * `installPageChangeEffect` would also catch it on an id change, but
   * not when the user asks for the page they are already on — which a
   * self-citing page's header is.
   */
  async function jumpTo(sourceSlug: string | undefined) {
    if (!sourceSlug) return;
    try {
      const view = await openRef(sourceSlug);
      // Backlinks refetch via `installBacklinksEffect`'s page key.
      resetForNavigation();
      applyPageState(view);
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
    icon?: string;
    kind?: PageKind;
    entries: Backlink[];
  }> {
    const groups = new Map<
      string,
      { key: string; title: string; icon?: string; kind?: PageKind; entries: Backlink[] }
    >();
    for (const b of appState.backlinks) {
      const key = b.source_page?.slug ?? "__orphan__";
      const title = b.source_page?.title ?? "(orphan)";
      const icon = b.source_page?.icon;
      const kind = b.source_page?.kind;
      const existing = groups.get(key);
      if (existing) existing.entries.push(b);
      else groups.set(key, { key, title, icon, kind, entries: [b] });
    }
    return [...groups.values()];
  }

  return (
    <Show
      when={
        appState.backlinksOpen &&
        lateSectionVisible({
          // The one fetch owner flips this flag, so "a reply for this
          // page has landed" needs no second signal.
          answered: !appState.backlinksLoading,
          count: appState.backlinks.length,
          // The desktop never draws an empty backlinks box.
          keepWhenEmpty: false,
        })
      }
    >
      <section class="mt-6">
        {/* Soft full-width rule — mirrors the TUI's `─` separator. */}
        <div class="border-t border-(--color-outl-border) opacity-60" />

        <header class="mt-3 mb-2 flex items-baseline justify-between gap-2">
          <span class="flex items-baseline gap-2 text-xs font-semibold uppercase tracking-wide opacity-60">
            {/* Always a real count: the section is only on screen once
                a scan has produced rows, so this never stands in for a
                pending one (`0 refs` would be a wrong answer). */}
            <span>
              Backlinks · {appState.backlinks.length} ref
              {appState.backlinks.length === 1 ? "" : "s"}
            </span>
            {/* Reachable only on a refetch, which is the one moment the
                mark is owed: the rows are up, the user is looking at
                them, and they are about to change. The outline keeps
                the caret and the keystrokes throughout (`UX.md` →
                Never block a keystroke). */}
            <Show when={appState.backlinksLoading}>
              <span
                data-backlinks-pending
                class="animate-pulse font-normal normal-case tracking-normal opacity-70"
              >
                loading…
              </span>
            </Show>
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
                  <PageGlyph icon={group.icon} kind={group.kind} class="self-center" />
                  <span>{group.title}</span>
                  <span class="text-xs opacity-50">{group.entries.length}</span>
                </button>

                <ul class="mt-1 space-y-1 pl-6">
                  <For each={group.entries}>
                    {(link, index) => {
                      // Through the `cursor` memo over
                      // `backlinkCursor`, never the raw field: the
                      // outline holds the cursor until it releases it,
                      // so a stale id cannot paint an accent bar on a
                      // row the keyboard is not on. The memo is why
                      // this is one comparison per row and not one
                      // page walk per row — see its doc.
                      const selected = () => cursor() === link.block_id;
                      // The `(page, block)` pair an edit of this row is
                      // addressed to, or `null` for an orphan — one
                      // owner across both GUI clients, so neither
                      // decides for itself which rows are editable.
                      const editTarget = () => backlinkEditTarget(link);
                      // The same pair, but only while *this* row is the
                      // one in Insert — the editing identity is global
                      // (one row at a time) and the row has to match it.
                      const editing = () =>
                        appState.editingBacklink?.blockId === link.block_id
                          ? editTarget()
                          : null;
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
                      /**
                       * The row in read mode — the cited block's text,
                       * and the gesture that starts editing it.
                       *
                       * A named local rather than an inline `fallback`,
                       * the shape `<BlockRow />` uses for its own
                       * bullet and body: the two states are 50 lines
                       * and 10, and nesting the long one inside the
                       * short one's prop buries the branch.
                       */
                      const rowText = () => (
                        <button
                          type="button"
                          // Editing in place, rather than opening the
                          // source page: the view that assembled these
                          // references is the one the user came to act
                          // in, and acting on a row used to cost it.
                          // Opening the page stays on the group header.
                          //
                          // An orphan backlink has no page to address
                          // (`source_page` is null when the citing
                          // block lives outside any page), so its row
                          // stays read-only.
                          onClick={() => {
                            const target = editTarget();
                            if (target) startBacklinkEdit(target);
                          }}
                          // No `onMouseEnter`: a pointer crossing this
                          // section is not a cursor move. It used to
                          // write `selectedBacklinkBlockId`, which is
                          // the keyboard's own cursor, so merely
                          // passing over a row on the way elsewhere
                          // took `j` and `Enter` off the block the user
                          // had left them on. The hover affordance is
                          // the `hover:bg-…` below, which costs no
                          // state.
                          class={`block min-w-0 flex-1 rounded px-1 py-0.5 text-left text-sm leading-snug opacity-90 hover:bg-(--color-outl-fg)/5 hover:opacity-100 ${
                            link.todo === "DONE"
                              ? "line-through opacity-60"
                              : ""
                          }`}
                        >
                          <Show
                            when={link.source_block.table}
                            fallback={
                              <MarkdownInline
                                tokens={link.source_block.tokens}
                                variant="inline"
                              />
                            }
                          >
                            {/* A citing block that is a table reads as
                                a grid here too. Inert (no `onEdit`) —
                                the row's own click is the edit
                                gesture, and a table has no cell
                                editor by design (#329). */}
                            <MarkdownTable
                              table={link.source_block.table!}
                              variant="inline"
                            />
                          </Show>
                        </button>
                      );
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
                              // The tooltip stays plain text — a
                              // `title=` cannot hold markup — while the
                              // row renders. One owner for both forms.
                              title={crumbTrailText(link.ancestors)}
                            >
                              <CrumbTrail crumbs={link.ancestors} />
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
                            <Show
                              when={editing()}
                              fallback={rowText()}
                            >
                              {/* Mounted only while this row is the one
                                  being edited, which is what seeds its
                                  draft once per edit session rather than
                                  once per row — see the editor's doc. */}
                              {(target) => (
                                <div class="min-w-0 flex-1 px-1 py-0.5 text-sm leading-snug">
                                  <BacklinkRowEditor
                                    link={link}
                                    target={target()}
                                  />
                                </div>
                              )}
                            </Show>
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
