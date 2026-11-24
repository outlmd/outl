/**
 * What is selected, and what happens to **N** blocks — the range half of
 * the journal's mutation surface (issue #265 phase 2).
 *
 * Mobile has no keyboard and deliberately no modal vim Visual state (RFC
 * 0254 rejects one explicitly — a hidden mode on a touch surface is
 * worse than a gesture the user can see). The anchor + cursor model is
 * the desktop's Visual mode unchanged (`visualRangeIds` from
 * `@outl/shared/outline`); only how it is *driven* differs — a
 * long-press menu item starts it, a tap on any other block extends it,
 * a floating toolbar (`<SelectionToolbar />`) fires the same range ops
 * the desktop's `>` / `<` / `⌘⇧↑↓` / `y` / `d` chords do.
 *
 * Split from `Journal.block-ops.ts` on the question, not the line count:
 * that module answers "what happens to one block", this one owns the
 * selection state machine as well as the ops. They share
 * `JournalBlockDeps` because a range op is N single-block commands plus
 * a range.
 */

import type { PageView } from "@outl/shared/api/types";
import {
  copyMarkdown,
  deleteBlock,
  indentBlock,
  moveBlockDown,
  moveBlockUp,
  outdentBlock,
} from "@outl/shared/api/commands";
import {
  createMoveGate,
  rangeMoveTargets,
  visibleRangeSlice,
} from "@outl/shared/outline";

import {
  type BlockSelection,
  extendSelectionTo,
  growSelectionDown,
  growSelectionUp,
  selectionIsLive,
  startSelection,
} from "../lib/block-selection";
import { haptic } from "../lib/haptics";

import type { JournalBlockDeps } from "./Journal.block-ops";

/** `JournalBlockDeps` plus the selection state a range op drives. */
export interface JournalSelectionDeps extends JournalBlockDeps {
  selection: () => BlockSelection | null;
  setSelection: (sel: BlockSelection | null) => void;
  lastSelection: () => BlockSelection | null;
  setLastSelection: (sel: BlockSelection) => void;
  setPendingRangeDelete: (ids: string[]) => void;
  /** Flush an in-flight edit before selection mode takes over the row. */
  commitEdit: () => Promise<unknown>;
}

export function createSelectionOps(deps: JournalSelectionDeps) {
  /** Leave selection mode. Captures the range as `lastSelection`
   *  first — every exit does (the toolbar's Done, a yank, a delete —
   *  vim's `gv` convention: `y`/`d` also drop out of Visual but leave
   *  the range reselectable). */
  function exit(): void {
    const sel = deps.selection();
    if (sel) deps.setLastSelection(sel);
    deps.setSelection(null);
  }

  /** Every block id the active selection covers, in DFS visible order.
   *  The ordering itself lives in `@outl/shared/outline` so this client
   *  and the desktop cannot disagree about what a range contains. */
  function currentRangeIds(): string[] | null {
    const sel = deps.selection();
    const cur = deps.view();
    if (!sel || !cur) return null;
    return visibleRangeSlice(sel.anchorId, sel.cursorId, cur.outline);
  }

  /** Fire `op` for each of `ids`, in the order given, and paint only the
   *  last view that came back. The shared body behind Indent/Outdent and
   *  the move ops. The range stays selected afterward, vim convention:
   *  the user can repeat the op. */
  async function walk(
    op: (pid: string, id: string) => Promise<PageView>,
    pid: string,
    ids: readonly string[],
  ): Promise<void> {
    let lastView: PageView | undefined;
    for (const id of ids) {
      const v = await deps.withError(() => op(pid, id));
      if (v) lastView = v;
    }
    if (lastView) deps.applyView(lastView);
  }

  /** Indent / outdent: every id of the range, top-down. A child of a
   *  selected parent is meant to be re-levelled too. */
  async function applyRangeOp(
    op: (pid: string, id: string) => Promise<PageView>,
  ): Promise<void> {
    const pid = deps.pageId();
    const ids = currentRangeIds();
    if (!pid || !ids || ids.length === 0) return;
    await walk(op, pid, ids);
  }

  /** One range move in flight at a time. The outline is repainted only
   *  after the walk, so a second press that resolved its range early
   *  would read stale edges and scramble. The range is resolved *inside*
   *  the gate for the same reason. */
  const moveOnce = createMoveGate();

  /** Move the range one slot. Walks the range's roots, not every id: a
   *  parent selected with its children moves as one unit, and a range
   *  that cannot move whole (edge, mixed parents) does not move.
   *  Pinned by test, not by comment (`Journal.selection-ops.test.ts` →
   *  "range ops walk order"). */
  async function moveRange(
    op: (pid: string, id: string) => Promise<PageView>,
    dir: "up" | "down",
  ): Promise<void> {
    await moveOnce(async () => {
      const pid = deps.pageId();
      const ids = currentRangeIds();
      const cur = deps.view();
      if (!pid || !ids || !cur) return;
      const targets = rangeMoveTargets(ids, cur.outline, dir);
      if (targets.length === 0) return;
      await walk(op, pid, targets);
    });
  }

  /** Bottom-up delete (children before parents) — mirrors the
   *  desktop's `DeleteRange`: when the range covers a parent and its
   *  descendants, deleting the parent first moves the whole subtree to
   *  trash and the follow-up delete on a descendant then fails
   *  ("already in trash"). `withError` records that per-id instead of
   *  aborting, so one bad id can't strand the rest of the range. */
  async function performDeleteRange(ids: string[]): Promise<void> {
    const pid = deps.pageId();
    if (!pid) return;
    const editing = deps.editingId();
    if (editing && ids.includes(editing)) deps.setEditingId(null);
    let lastView: PageView | undefined;
    for (let i = ids.length - 1; i >= 0; i--) {
      const v = await deps.withError(() => deleteBlock(pid, ids[i]));
      if (v) lastView = v;
    }
    if (lastView) deps.applyView(lastView);
    exit();
  }

  return {
    exit,
    currentRangeIds,
    performDeleteRange,

    /** "Select blocks" (long-press menu) — start a selection anchored
     *  at `id`. Commits any in-flight edit first: entering selection
     *  mid-edit would leave a textarea open underneath a row now
     *  behaving as a tap target for range extension instead of text
     *  input. */
    async selectBlocks(id: string) {
      if (deps.editingId()) await deps.commitEdit();
      haptic("medium");
      deps.setSelection(startSelection(id));
    },

    /** A tap on any row while a selection is active — grows or shrinks
     *  the range to meet it. Reachable only through `<BlockRow />`'s
     *  `onSelectTap`, which mobile only wires while `selection()` is
     *  non-null, but this stays defensive (no-op) if that ever changes. */
    selectTap(id: string) {
      const sel = deps.selection();
      if (!sel) return;
      deps.setSelection(extendSelectionTo(sel, id));
    },

    /** Toolbar `▲`/`▼` — grow the range by exactly one visible row, the
     *  discrete equivalent of the desktop's `Shift+↑`/`Shift+↓`
     *  (`SelectRangeUp` / `SelectRangeDown`) for a row that isn't
     *  directly reachable by tap without scrolling. */
    growUp() {
      const sel = deps.selection();
      const cur = deps.view();
      if (!sel || !cur) return;
      deps.setSelection(growSelectionUp(sel, cur.outline));
    },

    growDown() {
      const sel = deps.selection();
      const cur = deps.view();
      if (!sel || !cur) return;
      deps.setSelection(growSelectionDown(sel, cur.outline));
    },

    /** Context-menu "Reselect last selection" — vim `gv`. Only offered
     *  (see `buildContextActions`'s `canReselect`) when `lastSelection`
     *  still resolves against the live outline; a peer edit or a fold
     *  can strand an endpoint between sessions. */
    reselectLast() {
      const sel = deps.lastSelection();
      const cur = deps.view();
      if (!sel || !cur || !selectionIsLive(sel, cur.outline)) return;
      haptic("medium");
      deps.setSelection(sel);
    },

    async indentRange() {
      haptic("light");
      await applyRangeOp(indentBlock);
    },

    async outdentRange() {
      haptic("light");
      await applyRangeOp(outdentBlock);
    },

    async moveRangeUp() {
      haptic("light");
      await moveRange(moveBlockUp, "up");
    },

    /** Bottom-up walk (`rangeMoveTargets` orders it) — the last block in
     *  the range has to clear the block below the range first. */
    async moveRangeDown() {
      haptic("light");
      await moveRange(moveBlockDown, "down");
    },

    /**
     * Toolbar "Copy" — serialize the whole range as clean outl markdown
     * to the OS clipboard (the backend drops a block whose ancestor is
     * also in the range, so a parent+child selection doesn't duplicate
     * the child — same guarantee the desktop's `YankRange` documents).
     * Exits selection afterward, matching vim's `y` — the desktop's
     * `YankRange` does the same via `exitVisual()`.
     */
    async yankRange() {
      const ids = currentRangeIds();
      if (!ids || ids.length === 0) return;
      haptic("light");
      try {
        const md = await copyMarkdown(ids);
        await navigator.clipboard?.writeText(md);
      } catch {
        // Best-effort — same posture as the single-block "Copy text"
        // action; some webviews refuse `navigator.clipboard` outside a
        // user gesture chain.
      }
      exit();
    },

    /** Toolbar "Delete" — always confirms (`<JournalDeleteDialogs>` via
     *  `pendingRangeDelete`), unlike the single-block swipe delete
     *  (which only prompts when that one block has descendants): a
     *  range is N blocks, any of which may carry children the user
     *  can't see from the toolbar. */
    requestDeleteRange() {
      const ids = currentRangeIds();
      if (!ids || ids.length === 0) return;
      haptic("warning");
      deps.setPendingRangeDelete(ids);
    },
  };
}
