/**
 * Vim Visual mode: the contiguous block range, and the ops over it.
 *
 * The range is two store fields (`visualAnchorId` + `selectedBlockId`)
 * resolved at read time, never a materialised list — so a peer edit
 * between two keystrokes cannot leave a stale selection behind.
 *
 * `exitVisual` is exported because the Esc cascade in
 * `chrome-handlers.ts` is also a Visual exit, and every exit has to
 * capture `lastVisualRange` for `g v`. One owner, two callers.
 */

import {
  deleteBlock,
  indentBlock,
  moveBlockDown,
  moveBlockUp,
  outdentBlock,
} from "@outl/shared/api/commands";
import type { PageView } from "@outl/shared/api/types";
import {
  flattenVisible,
  nextVisibleId,
  previousVisibleId,
  visibleRangeSlice,
  visualRangeIds,
} from "@outl/shared/outline";

import { copySelectionToClipboard } from "./clipboard-handlers";
import { type HandlerCtx, lookupBlock } from "./handler-context";
import { type ActionHandlers } from "./shortcuts";
import { appState, setAppState } from "./store";

/** Enter (from Normal) or extend (already in Visual) a contiguous
 *  block selection by one row, `dir` = +1 down / -1 up. The non-vim
 *  multi-select path: `Shift+↓` / `Shift+↑` anchor at the current
 *  block and grow the range. Stays inside the outline — unlike plain
 *  `j` / `k`, it never crosses into the backlinks section, because a
 *  batch op has nothing to do with a read-only backlink row. */
function extendVisualRange(dir: 1 | -1) {
  if (appState.mode !== "vim-visual") {
    const id = appState.selectedBlockId;
    if (!id) return;
    setAppState("visualAnchorId", id);
    setAppState("mode", "vim-visual");
  }
  const cur = appState.selectedBlockId;
  if (!cur) return;
  const next =
    dir === 1
      ? nextVisibleId(cur, appState.outline)
      : previousVisibleId(cur, appState.outline);
  if (next) setAppState("selectedBlockId", next);
}

/** Capture the current range for `g v`, drop the anchor, and land
 *  back in Normal. Shared by every Visual exit (`Esc`, `y`, `d`, the
 *  toolbar's Done). */
export function exitVisual() {
  const range = visualRangeIds(
    appState.visualAnchorId,
    appState.selectedBlockId,
    appState.outline,
  );
  if (range) setAppState("lastVisualRange", range);
  setAppState("visualAnchorId", null);
  setAppState("mode", "vim-normal");
}

export function visualHandlers(ctx: HandlerCtx): ActionHandlers {
  // Plain record, not Solid props — see `HandlerCtx`.
  const { deps, runEach } = ctx;

  /** Walk every block in the current Visual range and fire `op` for
   *  each. Used by `>` / `<` (indent) and `Cmd/Ctrl+Shift+↑/↓` (move)
   *  so the multi-block ops share one body. Range stays selected after
   *  the op (vim convention) so the user can repeat it — the anchor /
   *  cursor are block ids, stable across the re-render.
   *
   *  `reverse` walks bottom-up: a move-**down** has to shift the last
   *  block past the block below the range first, or an ascending walk
   *  would drag each block over its own not-yet-moved neighbour. Indent
   *  / move-up keep the default top-down order. */
  async function applyVisualBlockOp(
    op: (pageId: string, id: string) => Promise<PageView>,
    reverse = false,
  ) {
    const pageId = appState.page?.id;
    if (!pageId) return;
    const slice = visibleRangeSlice(
      appState.visualAnchorId,
      appState.selectedBlockId,
      appState.outline,
    );
    if (!slice) return;
    const targets = reverse ? [...slice].reverse() : slice;
    await runEach(targets, (id) => op(pageId, id));
  }

  return {
    // ── Vim Visual mode ──────────────────────────────────────────
    //
    // The desktop's Visual mode covers a contiguous range of outline
    // blocks (vim's `V` line-visual semantics). `j` / `k` extend the
    // range, `y` yanks, `d` deletes, `>` / `<` shift indent, `Esc`
    // exits + captures `lastVisualRange` so `gv` can restore.
    EnterVisual: () => {
      const id = appState.selectedBlockId;
      if (!id) return;
      setAppState("visualAnchorId", id);
      setAppState("mode", "vim-visual");
    },
    ReselectLastVisual: () => {
      const range = appState.lastVisualRange;
      if (!range) {
        deps.setError("no previous selection");
        return;
      }
      // Verify both ids still exist in the outline (a peer might
      // have deleted them between sessions). Drop gracefully.
      const ids = flattenVisible(appState.outline);
      if (!ids.includes(range.lo) || !ids.includes(range.hi)) {
        deps.setError("previous selection no longer exists");
        return;
      }
      setAppState("visualAnchorId", range.lo);
      setAppState("selectedBlockId", range.hi);
      setAppState("mode", "vim-visual");
    },
    YankRange: () => {
      const rangeIds = visibleRangeSlice(
        appState.visualAnchorId,
        appState.selectedBlockId,
        appState.outline,
      );
      if (!rangeIds) return;
      const texts: string[] = [];
      for (const id of rangeIds) {
        const block = lookupBlock(id);
        if (block) texts.push(block.text);
      }
      setAppState("yankRegister", texts);
      exitVisual();
      // Copy the whole range as markdown; the backend drops any block
      // whose ancestor is also selected so a parent+child range doesn't
      // duplicate the child.
      void copySelectionToClipboard(ctx, rangeIds);
    },
    DeleteRange: async () => {
      const pageId = appState.page?.id;
      if (!pageId) return;
      const rangeIds = visibleRangeSlice(
        appState.visualAnchorId,
        appState.selectedBlockId,
        appState.outline,
      );
      if (!rangeIds) return;
      // Snapshot the range as ids (NOT indices) up front. NodeIds are
      // stable across the CRDT — `deleteBlock` is `Move(node, TRASH)`,
      // not a re-keying — so we don't have to re-resolve them after
      // each round-trip.
      //
      // Bottom-up: when the range covers both a parent and its
      // children, the children go first so the parent's move-to-trash
      // doesn't pull a still-targeted descendant from under us. Even
      // so some ids refuse ("already in trash", or a peer won the
      // race); `runEach` owns why the walk continues anyway.
      const targets = [...rangeIds].reverse();
      // Selection lands above the deleted range (first block if at top),
      // read from the PRE-delete ids: after applyView `rangeIds[0]` is gone.
      const ids = flattenVisible(appState.outline);
      const prev = ids[Math.max(ids.indexOf(rangeIds[0]) - 1, 0)];
      await runEach(targets, (id) => deleteBlock(pageId, id));
      exitVisual();
      setAppState("selectedBlockId", prev ?? null);
    },
    IndentVisualRange: () => applyVisualBlockOp(indentBlock),
    OutdentVisualRange: () => applyVisualBlockOp(outdentBlock),
    // `Cmd/Ctrl+Shift+↑` — drag the whole range up among its siblings.
    // Top-down walk: the first block slides up past the block above the
    // range, then each following block moves into the slot it vacated.
    // Selection follows (block ids are stable) so the user can repeat.
    MoveVisualRangeUp: () => applyVisualBlockOp(moveBlockUp),
    // `Cmd/Ctrl+Shift+↓` — same, downward. Bottom-up walk so the last
    // block clears the block below the range before its neighbours move.
    MoveVisualRangeDown: () => applyVisualBlockOp(moveBlockDown, true),
    // `Shift+↓` / `Shift+↑` — start (from Normal) or extend a contiguous
    // selection. The non-vim multi-select entry; both chords keep
    // extending once the client is in Visual.
    SelectRangeDown: () => extendVisualRange(1),
    SelectRangeUp: () => extendVisualRange(-1),
  };
}
