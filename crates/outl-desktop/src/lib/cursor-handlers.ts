/**
 * Where the user is: the selection cursor, which day or page it sits
 * on, which subtree it is zoomed into, and keeping it on screen.
 *
 * Nothing here mutates the workspace — every handler writes view
 * state or opens a page. That is the line that separates this module
 * from `block-handlers.ts`, and it is why `ZoomIn` / `ZoomOut` belong
 * here rather than with the fold ops: zoom is `focusBlockId`, sliced
 * at render time, and never an `Op`.
 */

import {
  nextDay,
  openJournalFor,
  openRef,
  openTodayJournal,
  previousDay,
  todaySlug,
} from "@outl/shared/api/commands";
import type { BlockNode } from "@outl/shared/api/types";
import {
  flattenVisible,
  focusSubtree,
  nextVisibleId,
  previousVisibleId,
} from "@outl/shared/outline";

import { type HandlerCtx } from "./handler-context";
import { type ActionHandlers } from "./shortcuts";
import { appState, setAppState } from "./store";

/** Block ids of each visible backlink, in render order. The
 *  selection cursor traverses this list with `j/k` when the
 *  backlinks section is reachable. */
function backlinkBlockIds(): string[] {
  if (!appState.backlinksOpen) return [];
  return appState.backlinks.map((b) => b.block_id);
}

/** Blocks the selection cursor is allowed to traverse. When zoomed
 *  (`focusBlockId` set and still present), the focused block is the
 *  header title, so the cursor traverses its **children** (the visible
 *  body) — matching what `<OutlineView />` renders. Otherwise the
 *  whole page. A stale focus falls back to the full outline. */
function navBlocks(): BlockNode[] {
  const id = appState.focusBlockId;
  if (!id) return appState.outline;
  const fv = focusSubtree(appState.outline, id);
  return fv ? fv.root.children : appState.outline;
}

/** Pop one level of zoom, to the parent crumb or all the way out. */
function zoomOut() {
  const id = appState.focusBlockId;
  if (!id) return;
  const crumbs = focusSubtree(appState.outline, id)?.breadcrumb ?? [];
  setAppState("focusBlockId", crumbs[crumbs.length - 1]?.id ?? null);
}

export function cursorHandlers(ctx: HandlerCtx): ActionHandlers {
  // Plain record, not Solid props — see `HandlerCtx`.
  const { deps, runOn, safeCall } = ctx;

  /** Walk one journal day from wherever the user is: the open journal
   *  when they are on one, today when they are not (`[[` / `]]` from an
   *  ordinary page means yesterday / tomorrow, not "nothing happens").
   *  `step` is the only thing that differs between the two directions,
   *  and writing it twice is how one of them drifts. */
  const stepDay = async (step: (slug: string) => Promise<string>) => {
    const anchor =
      appState.page?.kind === "journal"
        ? appState.page.slug
        : await safeCall(todaySlug());
    if (!anchor) return;
    const slug = await safeCall(step(anchor));
    if (!slug) return;
    await runOn(openJournalFor(slug));
  };

  return {
    // ── page-level navigation ────────────────────────────────────
    OpenToday: async () => {
      await runOn(openTodayJournal());
    },
    PrevDay: () => stepDay(previousDay),
    NextDay: () => stepDay(nextDay),

    // ── selection (vim j/k/↑/↓) ──────────────────────────────────
    //
    // The cursor spans two regions: the outline blocks (top) and
    // the inline backlinks (bottom). Only one of
    // `selectedBlockId` / `selectedBacklinkBlockId` is non-null at
    // any time so navigation stays a single linear motion.
    //
    // Backlinks are only reachable when the section is visible
    // (`backlinksOpen` + the page has at least one ref); otherwise
    // `j` past the last outline block clamps at the bottom — same
    // behaviour the TUI ships when `show_backlinks == false`.
    SelectionDown: () => {
      const backlinkIds = backlinkBlockIds();
      const curBacklink = appState.selectedBacklinkBlockId;
      if (curBacklink) {
        const idx = backlinkIds.indexOf(curBacklink);
        if (idx >= 0 && idx < backlinkIds.length - 1) {
          setAppState("selectedBacklinkBlockId", backlinkIds[idx + 1]);
        }
        // last backlink → stay (no wrap into outline)
        return;
      }
      const cur = appState.selectedBlockId;
      const nav = navBlocks();
      const next = nextVisibleId(cur, nav);
      const atLast = cur !== null && next === cur;
      if (atLast && backlinkIds.length > 0) {
        // Cross into the backlinks section.
        setAppState("selectedBlockId", null);
        setAppState("selectedBacklinkBlockId", backlinkIds[0]);
        return;
      }
      if (next) setAppState("selectedBlockId", next);
    },
    SelectionUp: () => {
      const backlinkIds = backlinkBlockIds();
      const curBacklink = appState.selectedBacklinkBlockId;
      if (curBacklink) {
        const idx = backlinkIds.indexOf(curBacklink);
        if (idx > 0) {
          setAppState("selectedBacklinkBlockId", backlinkIds[idx - 1]);
          return;
        }
        // First backlink → step back into the outline's last
        // visible block.
        const list = flattenVisible(navBlocks());
        if (list.length > 0) {
          setAppState("selectedBacklinkBlockId", null);
          setAppState("selectedBlockId", list[list.length - 1]);
        }
        return;
      }
      const cur = appState.selectedBlockId;
      const nav = navBlocks();
      const prev = previousVisibleId(cur, nav);
      if (prev) {
        setAppState("selectedBlockId", prev);
        return;
      }
      // Only when the zoom holds nothing: `z i` on a childless block
      // shows an empty page, so every movement key goes quiet and `z o`
      // was the only way out. At the top of a zoom that HAS rows `prev`
      // is null too, and escaping there would eject a reader mid-read.
      if (appState.focusBlockId && flattenVisible(nav).length === 0) zoomOut();
    },

    OpenRefUnderCursor: async () => {
      // Normal-mode `Enter`. Two branches in priority order:
      //
      // 1. Selection is on a backlink row → open the source page and
      //    snap the selection to the referencing block on that page
      //    (backlink rows are read-only, so "open" is the only
      //    meaningful gesture there).
      // 2. Otherwise → enter Insert on the selected block.
      //
      // Unlike the TUI, the desktop's Normal mode has no character
      // cursor — only a selected block — so "the ref under the
      // cursor" cannot be resolved. An earlier version approximated
      // it as "the first `[[ref]]` in the block", which made every
      // ref-carrying block impossible to edit via Enter. Following
      // a ref on the desktop is the click on the token
      // (`onRefClick` in OutlineView).
      const curBacklink = appState.selectedBacklinkBlockId;
      if (curBacklink) {
        const link = appState.backlinks.find((b) => b.block_id === curBacklink);
        const target = link?.source_page?.slug;
        if (!target) return;
        const view = await safeCall(openRef(target));
        if (!view) return;
        deps.applyView(view);
        // Reset cursor to the source block on the freshly-opened
        // page so the user lands exactly where the ref lives.
        setAppState("selectedBacklinkBlockId", null);
        setAppState("selectedBlockId", curBacklink);
        return;
      }
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("editingBlockId", id);
    },
    // ── Zoom / focus (z i / z o, Cmd+Shift+] / Cmd+Shift+[) ───────
    //
    // View-state only: `focusBlockId` slices the outline via
    // `focusSubtree` at render time; no op, no backend round-trip. The
    // breadcrumb click targets in `<OutlineView />` set the same signal.
    ZoomIn: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("focusBlockId", id);
    },
    // The desktop keeps no zoom stack; pop one level by reading the
    // current focus's breadcrumb — its last crumb is the parent to zoom
    // to, and an empty breadcrumb means the focused block is top-level,
    // so zooming out exits to the full page. A stale focus (block gone)
    // also exits.
    ZoomOut: zoomOut,

    // ── Viewport (zz) ────────────────────────────────────────────
    CenterViewport: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      const el = document.querySelector<HTMLElement>(`[data-block-id="${id}"]`);
      if (el) el.scrollIntoView({ block: "center", behavior: "smooth" });
    },
  };
}
