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
  openTodayJournal,
  previousDay,
  todaySlug,
} from "@outl/shared/api/commands";
import {
  flattenVisible,
  nextVisibleId,
  previousVisibleId,
} from "@outl/shared/outline";

import { openBacklinkContext } from "./backlink-jump";
import { backlinkRowOps } from "./backlink-handlers";
import {
  backlinkCursor,
  backlinkRowByKey,
  backlinkRowIds,
} from "./cursor-region";
import { type HandlerCtx } from "./handler-context";
import { type ActionHandlers } from "./shortcuts";
import { appState, setAppState, visibleRoots, zoomView } from "./store";

/** Pop one level of zoom, to the parent crumb or all the way out. */
function zoomOut() {
  const id = appState.focusBlockId;
  if (!id) return;
  const crumbs = zoomView()?.breadcrumb ?? [];
  setAppState("focusBlockId", crumbs[crumbs.length - 1]?.id ?? null);
}

export function cursorHandlers(ctx: HandlerCtx): ActionHandlers {
  // Plain record, not Solid props — see `HandlerCtx`.
  const { navigateTo, safeCall } = ctx;
  const onBacklink = backlinkRowOps(ctx);

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
    await navigateTo(openJournalFor(slug));
  };

  /** One day either side of **today**, ignoring the open page. The
   *  anchor is the difference from `stepDay`, and it is the whole
   *  reason `/yesterday` is not `PrevDay` under another name. */
  const dayFromToday = async (step: (slug: string) => Promise<string>) => {
    const today = await safeCall(todaySlug());
    if (!today) return;
    const slug = await safeCall(step(today));
    if (!slug) return;
    await navigateTo(openJournalFor(slug));
  };

  return {
    // ── page-level navigation ────────────────────────────────────
    OpenToday: async () => {
      await navigateTo(openTodayJournal());
    },
    PrevDay: () => stepDay(previousDay),
    NextDay: () => stepDay(nextDay),
    // Anchored on today, where `PrevDay` / `NextDay` step from the
    // open journal. On a 2020 page `[` means 2020; `/yesterday` has
    // to mean yesterday there too.
    OpenYesterday: () => dayFromToday(previousDay),
    OpenTomorrow: () => dayFromToday(nextDay),

    // ── selection (vim j/k/↑/↓) ──────────────────────────────────
    //
    // The cursor spans two regions: the outline blocks (top) and
    // the inline backlinks (bottom). Only one of
    // `selectedBlockId` / `selectedBacklinkBlockId` is non-null at
    // any time so navigation stays a single linear motion.
    //
    // Backlinks are only reachable when the section is on screen
    // (`backlinksOpen`, not zoomed, at least one ref); otherwise `j`
    // past the last outline block clamps at the bottom — same behaviour
    // the TUI ships when `show_backlinks == false`.
    //
    // That condition is `backlinkRowIds()`'s (`lib/cursor-region.ts`),
    // never a copy here. It used to live in a local helper, and it was
    // the **only** place that knew it: `backlinkCursor()` did not, so
    // after `Cmd+Shift+B` the cursor still claimed to be on a row the
    // section had taken away, and nine mutating chords wrote to another
    // page from there.
    SelectionDown: () => {
      const backlinkIds = backlinkRowIds();
      const curBacklink = backlinkCursor();
      if (curBacklink) {
        const idx = backlinkIds.indexOf(curBacklink);
        if (idx >= 0 && idx < backlinkIds.length - 1) {
          setAppState("selectedBacklinkBlockId", backlinkIds[idx + 1]);
        }
        // last backlink → stay (no wrap into outline)
        return;
      }
      const cur = appState.selectedBlockId;
      const nav = visibleRoots();
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
      const backlinkIds = backlinkRowIds();
      const curBacklink = backlinkCursor();
      if (curBacklink) {
        const idx = backlinkIds.indexOf(curBacklink);
        if (idx > 0) {
          setAppState("selectedBacklinkBlockId", backlinkIds[idx - 1]);
          return;
        }
        // First backlink → step back into the outline's last
        // visible block.
        const list = flattenVisible(visibleRoots());
        if (list.length > 0) {
          setAppState("selectedBacklinkBlockId", null);
          setAppState("selectedBlockId", list[list.length - 1]);
        }
        return;
      }
      const cur = appState.selectedBlockId;
      const nav = visibleRoots();
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
      // Normal-mode `Enter`: **edit the block under the cursor**,
      // wherever that cursor is.
      //
      // Unlike the TUI, the desktop's Normal mode has no character
      // cursor — only a selected block — so "the ref under the
      // cursor" cannot be resolved. An earlier version approximated
      // it as "the first `[[ref]]` in the block", which made every
      // ref-carrying block impossible to edit via Enter. Following
      // a ref on the desktop is the click on the token
      // (`onRefClick` in OutlineView).
      //
      // A backlink row used to be the one exception: it opened the
      // source page instead, because the row was read-only. It is not
      // any more, so the exception goes with the thing that justified
      // it — this now answers `Enter` the same way the TUI does, where
      // `Focus::Backlink` means `enter_insert_backlink`. The source
      // page stays reachable from the group header, which
      // `lib/keyboard-reach.ts` makes a keyboard stop.
      if (onBacklink.enterInsert()) return;
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

    // `g o` on a backlink row: open its page at the block its breadcrumb
    // names. Acts on the cursor's row even when `sameCrumbTrail` hides
    // that row's breadcrumb, which is the case the click cannot reach.
    OpenBacklinkContext: async () => {
      const key = backlinkCursor();
      const row = key ? backlinkRowByKey(key) : undefined;
      if (!row) {
        ctx.deps.setError(
          "g o opens a backlink's page at its block: move the cursor onto a backlink row first",
        );
        return;
      }
      await openBacklinkContext(row.link);
    },

    // ── Viewport (zz) ────────────────────────────────────────────
    CenterViewport: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      const el = document.querySelector<HTMLElement>(`[data-block-id="${id}"]`);
      if (el) el.scrollIntoView({ block: "center", behavior: "smooth" });
    },
  };
}
