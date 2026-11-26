import { For, Show, createEffect, createMemo } from "solid-js";

import {
  type BacklinkRowOp,
  backlinkRowOp,
  openRef,
  runCodeBlock,
  setBacklinksOrder,
} from "@outl/shared/api/commands";
import { backlinkCommitPlan } from "@outl/shared/backlink-edit";
import { lateSectionVisible, sameCrumbTrail } from "@outl/shared/outline";
import type { Backlink, BlockNode, PageKind } from "@outl/shared/api/types";
import { PageGlyph } from "@outl/shared/icons";

import {
  backlinkCursor,
  backlinkRowByKey,
  backlinkRows,
} from "../lib/cursor-region";
import {
  handleLinkClick,
  handleOpenPage,
  handleRefClick,
  handleTagClick,
} from "../lib/outline-navigation";
import { resetForNavigation } from "../lib/page-change";
import { applyPageState, resolveBacklinkTransclusions } from "../lib/page-view";
import { trackBacklinks } from "../lib/backlinks-fetch";
import { appState, setAppState } from "../lib/store";
import { openBacklinkContext } from "../lib/backlink-jump";
import { BacklinkRowItem } from "./BacklinkRow";
import type { BodyCallbacks } from "./block-callbacks";

interface Group {
  key: string;
  title: string;
  icon?: string;
  kind?: PageKind;
  /** Citing blocks only — what the group header counts. */
  rootKeys: string[];
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
   *
   * The chevron's fold takes the same path with `"collapse"` /
   * `"expand"`: the mouse twin of `ToggleCollapsed` on a row, written to
   * the source page so the fold syncs, and refetched because a folded
   * citing block hides its rows here (`flattenBacklinkRows`).
   */
  async function backlinkRowCommit(
    link: Backlink,
    node: BlockNode,
    op: BacklinkRowOp,
  ) {
    const sourcePage = link.source_page;
    const currentSlug = appState.page?.slug;
    if (!sourcePage || !currentSlug) return;
    const plan = backlinkCommitPlan({
      sourcePageId: sourcePage.id,
      openPageId: appState.page?.id,
    });
    try {
      const view = await backlinkRowOp(sourcePage.id, node.id, op);
      if (plan.applyView) applyPageState(view);
      // The mutation lands on the *source* page, so nothing on this page
      // re-renders on its own — and the backlinks effect is keyed on the
      // page, which didn't change. Refetch the projection directly: this
      // is one of the mutations that changes the current page's own
      // backlinks, the exception that effect's comment calls out.
      if (plan.refetchBacklinks) {
        await trackBacklinks();
      }
      if (op === "collapse") keepCursorOnFold(node.id);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  /**
   * A chevron fold can hide the row the keyboard cursor is on: the
   * cursor sits on a descendant (`a>a1`), the click folds `a`, and the
   * refetch drops `a>a1` from the drawn rows. Left there, the key names
   * nothing — the highlight vanishes, `j` stays put on `-1`, `k` leaves
   * for the outline and every row chord answers "row gone". So the
   * cursor moves up to the row that was folded, the outline's rule for
   * a selection under a fold.
   *
   * A key is the path of ids down to the block, so the folded row is
   * the cursor key's prefix ending at `foldedId`, in whichever drawn copy
   * the cursor was in. Only a hidden cursor moves: one the fold did not
   * touch stays where it is.
   */
  function keepCursorOnFold(foldedId: string) {
    const key = backlinkCursor();
    if (key === null || backlinkRowByKey(key)) return;
    const path = key.split(">");
    const at = path.indexOf(foldedId);
    if (at < 0 || at === path.length - 1) return;
    const folded = path.slice(0, at + 1).join(">");
    if (backlinkRowByKey(folded)) {
      setAppState("selectedBacklinkBlockId", folded);
    }
  }

  /**
   * A fence's Run, from a backlink row.
   *
   * **Addressed to the source page**, the one the block lives on: the
   * outline's handler runs against the open page, and the block is not
   * on it. The reply follows `backlinkCommitPlan`, the rule the bullet
   * above follows — install it only when the source page *is* the open
   * one (a self-citing page), and always refetch, because the result
   * children land under a citing block and change this page's own
   * backlinks.
   *
   * An orphan row has no page to run on, so it says so rather than
   * guessing one.
   */
  async function runBacklinkCodeBlock(link: Backlink, blockId: string) {
    const sourcePage = link.source_page;
    if (!sourcePage) {
      setAppState(
        "lastError",
        "This block has no page, so it can't run from here",
      );
      return;
    }
    const plan = backlinkCommitPlan({
      sourcePageId: sourcePage.id,
      openPageId: appState.page?.id,
    });
    try {
      const reply = await runCodeBlock(sourcePage.id, blockId);
      if (plan.applyView) applyPageState(reply.view);
      if (reply.error) {
        setAppState("lastError", `${reply.language}: ${reply.error}`);
      }
      if (plan.refetchBacklinks) await trackBacklinks();
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    }
  }

  /**
   * What a click inside a backlink row's rendered block does.
   *
   * The navigation handlers are the outline's own
   * (`lib/outline-navigation.ts`): they only navigate, so a cross-page
   * row can take them unchanged. **No `onSetProperty`, by decision**:
   * properties are read-only in a row, and `<BlockBody />` draws them
   * inert without it. Changing one means editing the block's text.
   */
  function backlinkBodyCallbacks(link: Backlink): BodyCallbacks {
    return {
      onRunCodeBlock: (id) => runBacklinkCodeBlock(link, id),
      onOpenPage: (slug) => void handleOpenPage(slug),
      onRefClick: (target) => void handleRefClick(target),
      onTagClick: handleTagClick,
      onLinkClick: handleLinkClick,
    };
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
   * per-block positioning). The other door is a row's breadcrumb line,
   * which lands on the block it names (`openBacklinkContext`).
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
   * the group header was the only door to the source page until the
   * breadcrumb line became the second one.
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
   * The panel's layout, **once per fetch**: citing blocks grouped by
   * source page (`null` source collapses under a synthetic `(orphan)`
   * header), each with the keys of its drawn descendants, and which
   * roots show their breadcrumb.
   *
   * Everything the `<For>`s iterate is a **string key**, so a refetch
   * that replaces every object keeps every row mounted — an open
   * editor and its draft included. The rows read their data back by key
   * (`backlinkRowByKey`). Nothing in here runs per row (invariant 15).
   */
  // The rows draw `((ref))` / `!((embed))` tokens of *other* pages'
  // blocks, which the outline's own resolve never saw. Roots only: the
  // collector walks their children.
  createEffect(() => {
    const roots = appState.backlinks.map((b) => b.source_block);
    if (roots.length > 0) resolveBacklinkTransclusions(roots);
  });

  const layout = createMemo(() => {
    const groups = new Map<string, Group>();
    const childKeys = new Map<string, string[]>();
    const crumbs = new Set<string>();
    const lastRootOf = new Map<string, Backlink>();
    let root: string | null = null;
    for (const row of backlinkRows()) {
      if (row.depth > 0) {
        childKeys.get(root!)!.push(row.key);
        continue;
      }
      root = row.key;
      childKeys.set(row.key, []);
      const b = row.link;
      const gkey = b.source_page?.slug ?? "__orphan__";
      let g = groups.get(gkey);
      if (!g) {
        g = {
          key: gkey,
          title: b.source_page?.title ?? "(orphan)",
          icon: b.source_page?.icon,
          kind: b.source_page?.kind,
          rootKeys: [],
        };
        groups.set(gkey, g);
      }
      // Collapsed against the previous entry in the same group:
      // consecutive references in the same branch show the trail once.
      const prev = lastRootOf.get(gkey);
      if (
        b.ancestors.length > 0 &&
        (!prev || !sameCrumbTrail(prev.ancestors, b.ancestors))
      ) {
        crumbs.add(row.key);
      }
      lastRootOf.set(gkey, b);
      g.rootKeys.push(row.key);
    }
    return { groups, childKeys, crumbs };
  });
  const groupKeys = createMemo(() => [...layout().groups.keys()]);

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
          <For each={groupKeys()}>
            {(gkey) => {
              const group = () => layout().groups.get(gkey);
              return (
                <Show when={group()}>
                  {(g) => (
                    <div>
                      <button
                        type="button"
                        onClick={() =>
                          void jumpTo(
                            gkey === "__orphan__" ? undefined : gkey,
                          )
                        }
                        class="flex w-full items-baseline gap-2 rounded px-1 py-0.5 text-left text-sm font-semibold hover:bg-(--color-outl-fg)/5"
                      >
                        <PageGlyph
                          icon={g().icon}
                          kind={g().kind}
                          class="self-center"
                        />
                        <span>{g().title}</span>
                        <span class="text-xs opacity-50">
                          {g().rootKeys.length}
                        </span>
                      </button>

                      {/* `pl-2`, not `pl-6`: each row now opens with the
                          16px fold-chevron slot, so the bullets sit where
                          they always did. */}
                      <ul class="mt-1 space-y-1 pl-2">
                        <For each={g().rootKeys}>
                          {(key) => (
                            <BacklinkRowItem
                              rowKey={key}
                              cursor={cursor}
                              showCrumbs={layout().crumbs.has(key)}
                              childKeys={layout().childKeys.get(key) ?? []}
                              onToggle={(l, n) =>
                                void backlinkRowCommit(l, n, "toggle-todo")
                              }
                              onFold={(l, n, collapse) =>
                                void backlinkRowCommit(
                                  l,
                                  n,
                                  collapse ? "collapse" : "expand",
                                )
                              }
                              bodyCallbacks={backlinkBodyCallbacks}
                              onCrumbClick={(l) => void openBacklinkContext(l)}
                            />
                          )}
                        </For>
                      </ul>
                    </div>
                  )}
                </Show>
              );
            }}
          </For>
        </div>
      </section>
    </Show>
  );
}
