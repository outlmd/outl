/**
 * Actions whose unit is the **whole page**, not the block under the
 * cursor: delete it, walk its undo stack, fold or unfold every block
 * in it.
 *
 * Grouped by that unit rather than by what they feel like, because it
 * is the property that decides their semantics — undo/redo are a
 * per-page stack in `outl_actions::history`, and fold-all has to walk
 * blocks the cursor cannot reach.
 */

import { deletePage, setBlockCollapsed } from "@outl/shared/api/commands";
import { flattenAll, flattenParents } from "@outl/shared/outline";

import { redoPage, undoPage } from "./api";
import { type HandlerCtx } from "./handler-context";
import { type ActionHandlers } from "./shortcuts";
import { appState } from "./store";

export function pageHandlers(ctx: HandlerCtx): ActionHandlers {
  // Plain record, not Solid props — see `HandlerCtx`.
  const { runEach, runOn } = ctx;

  /** Walk every block on the current page and set its `collapsed`
   *  flag to `value`. **Never `flattenVisible`** — the whole point of
   *  `zR` is to expand subtrees currently hidden under a collapsed
   *  parent, and the visible-only walk would silently no-op on every
   *  descendant of a folded node.
   *
   *  `zM` (value=true) uses `flattenParents` so leaves are skipped:
   *  folding a leaf is invisible today, but `outl_actions::set_block_collapsed`
   *  always writes `Op::SetCollapsed` to the log (a CRDT contract:
   *  every flip lands so concurrent flips converge via HLC), so when
   *  the user later adds children underneath they appear collapsed —
   *  future-surprise. `zR` (value=false) uses `flattenAll`: descolapsar
   *  leaf não tem efeito futuro e mantém a contagem de ops simétrica.
   *  Mirror exato do `collect_collapse_candidates` no TUI. */
  async function applyCollapsedToAll(value: boolean) {
    const pageId = appState.page?.id;
    if (!pageId) return;
    const ids = value
      ? flattenParents(appState.outline)
      : flattenAll(appState.outline);
    await runEach(ids, (id) => setBlockCollapsed(pageId, id, value));
  }

  return {
    // Delete the currently-viewed page. Triggered from the sidebar's
    // hover × button (the handler is reused here for keyboard parity
    // if a future binding maps a chord to `DeletePage`). We confirm
    // via the OS dialog before calling the backend; the backend does
    // NOT re-confirm. Returns today's journal so the view navigates
    // away from the deleted page in the same round-trip.
    DeletePage: async () => {
      const slug = appState.page?.slug;
      if (!slug) return;
      const title = appState.page?.title ?? slug;
      const ok = window.confirm(
        `Delete page "${title}"?\n\nThis removes the page and all its blocks. ` +
          `The deletion syncs to paired devices.`,
      );
      if (!ok) return;
      await runOn(deletePage(slug));
    },
    // ── Fold control over the whole page (zR / zM) ───────────────
    UnfoldAll: () => applyCollapsedToAll(false),
    FoldAll: () => applyCollapsedToAll(true),

    // ── undo / redo (Cmd+Z / Cmd+Shift+Z, Normal mode) ───────────
    //
    // Block-level history: reverts / re-applies the last committed
    // mutation on the current page (the backend snapshots the
    // page's `.md` render around every `finish_in_page` mutation).
    // The chords are Normal-mode only, so a focused textarea keeps
    // its own `Cmd+Z`. An empty stack rejects with "nothing to
    // undo" / "nothing to redo" — surfaced via the status bar.
    // Selection is left as-is; if the restored outline no longer
    // contains the selected block, `<OutlineView />`'s
    // auto-selection recovers.
    Undo: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      await runOn(undoPage(pageId));
    },
    Redo: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      await runOn(redoPage(pageId));
    },
  };
}
