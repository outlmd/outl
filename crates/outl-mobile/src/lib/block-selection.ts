import type { BlockNode } from "@outl/shared/api/types";
import { nextVisibleId, previousVisibleId, visualRangeIds } from "@outl/shared/outline";

/**
 * Touch-native multi-block selection state (RFC 0254 phase 3).
 *
 * Mobile has no keyboard and deliberately no modal vim Visual state
 * (the RFC rejects that explicitly — a hidden mode on a touch surface
 * is worse than a touch-native gesture). What it needs instead is the
 * same anchor + cursor pair the desktop's Visual mode already carries
 * in `appState.visualAnchorId` / `appState.selectedBlockId`, driven by
 * taps instead of `j`/`k` — so the range math is the shared
 * `visualRangeIds` from `@outl/shared/outline`, not a reimplementation.
 *
 * Only the **state transitions** live here — start a selection, grow
 * it by tapping another row, grow it one visible row at a time (the
 * toolbar's step buttons, mirroring the desktop's `Shift+↓`/`Shift+↑`).
 * `Journal.tsx` owns everything downstream: which blocks are inside
 * the range (`visualRangeSet`, memoised per render), dispatching the
 * range ops, and the toolbar UI.
 */
export interface BlockSelection {
  /** The block long-pressed (or last reselected) to start the range.
   *  Fixed for the life of the selection — only the cursor moves. */
  anchorId: string;
  /** The other end of the range. Tapping a block, or a toolbar step
   *  button, moves this; the anchor never does. */
  cursorId: string;
}

/** Start a new selection anchored (and cursored) at `blockId` — the
 *  block whose context menu the user picked "Select blocks" from. A
 *  single-block selection is a valid, if trivial, range. */
export function startSelection(blockId: string): BlockSelection {
  return { anchorId: blockId, cursorId: blockId };
}

/**
 * Move the cursor to `blockId`, anchor unchanged. This is the "tap a
 * row to extend" gesture: the resulting range is whatever
 * `visualRangeIds` resolves between the fixed anchor and the new
 * cursor, so tapping a row *above* the anchor grows the range upward
 * and a row below grows it downward — the same either-direction
 * behaviour the desktop gets from `j`/`k` after `V`, just reachable in
 * one tap instead of N.
 */
export function extendSelectionTo(
  selection: BlockSelection,
  blockId: string,
): BlockSelection {
  return { anchorId: selection.anchorId, cursorId: blockId };
}

/** Grow the range by exactly one visible row downward — the toolbar's
 *  discrete step, mirroring the desktop's `SelectRangeDown`
 *  (`Shift+↓`). Clamps at the bottom: past the last visible block,
 *  the cursor stays put rather than falling off the outline. */
export function growSelectionDown(
  selection: BlockSelection,
  outline: BlockNode[],
): BlockSelection {
  const next = nextVisibleId(selection.cursorId, outline);
  return next ? { anchorId: selection.anchorId, cursorId: next } : selection;
}

/** Grow the range by exactly one visible row upward (`SelectRangeUp`
 *  / `Shift+↑` on the desktop). Clamps at the top the same way. */
export function growSelectionUp(
  selection: BlockSelection,
  outline: BlockNode[],
): BlockSelection {
  const prev = previousVisibleId(selection.cursorId, outline);
  return prev ? { anchorId: selection.anchorId, cursorId: prev } : selection;
}

/**
 * Does this selection still resolve against the live outline? A peer
 * edit can delete or fold away either endpoint between when the
 * selection was captured (as `lastSelection`, for `ReselectLastVisual`
 * — "Reselect last selection" in the context menu) and when the user
 * tries to reuse it. `visualRangeIds` already returns `null` for a
 * stale endpoint; this just names that check so call sites don't have
 * to know the range-resolution machinery to ask "is this still good?"
 */
export function selectionIsLive(
  selection: BlockSelection,
  outline: BlockNode[],
): boolean {
  return visualRangeIds(selection.anchorId, selection.cursorId, outline) !== null;
}
