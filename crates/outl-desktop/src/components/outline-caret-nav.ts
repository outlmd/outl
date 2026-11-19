/**
 * The desktop's half of an arrow key that walks out of the block being
 * edited.
 *
 * `@outl/shared/caret-nav` owns the whole decision — whether the
 * keystroke leaves the block (`caretExit`) and which block it hands the
 * caret to (`resolveCaretExit`). What is left here is the two things
 * only this client knows: which blocks the cursor may walk
 * (`visibleRoots()`, so a zoom is honoured), and how it enters Insert
 * (`caretIntent` + `editingBlockId`). Mobile wraps the same pair with
 * its own signals, which is what stops the two from drifting.
 *
 * **It is two functions, and the split is load-bearing.**
 * `resolveCaretExit` is pure and synchronous so the editor can decide
 * whether to `preventDefault` *before* it awaits anything: a
 * `preventDefault` only counts during the synchronous dispatch, so a
 * handler that commits first and looks for a neighbour after has
 * already let the browser move the caret. And the editor must not
 * commit on a keystroke that goes nowhere — a `←` in the page's first
 * block would then drop the user out of Insert, which is the opposite
 * of what they pressed. So: resolve, bail when there is no neighbour,
 * commit, then `openCaretLanding`.
 *
 * Its own module for the reason `./outline-paste` and
 * `./outline-structure` are: the sequence deserves a test, and
 * `<OutlineView />` mounts the whole 3-pane shell.
 *
 * Deps are accessors, never values — Solid's reactivity rides the
 * getter.
 */

import type { BlockNode } from "@outl/shared/api/types";
import {
  type CaretExit,
  type CaretLandingTarget,
  resolveCaretExit,
} from "@outl/shared/caret-nav";

export type { CaretLandingTarget };

export interface CaretNavOpsDeps {
  /** The blocks the cursor may walk: the zoom's rows, else the page. */
  navBlocks: () => BlockNode[];
  /** Where the caret should land in the block being opened. */
  setCaretIntent: (offset: number) => void;
  setSelectedBlockId: (id: string) => void;
  setEditingId: (id: string) => void;
}

export function createCaretNavOps(deps: CaretNavOpsDeps) {
  return {
    /** Which block `exit` hands the caret to, or `null` when the
     *  keystroke goes nowhere and belongs to the browser. */
    resolveCaretExit: (id: string, exit: CaretExit) =>
      resolveCaretExit(deps.navBlocks(), id, exit),

    /**
     * Put Insert mode on `target`, caret included.
     *
     * Called after the editor committed its draft, so `editingBlockId`
     * is `null` and the fresh `<BlockEditor />` reads the intent on
     * mount — the same contract `EnterInsertAtEnd` and the Tab handler
     * already use.
     */
    openCaretLanding(target: CaretLandingTarget) {
      deps.setCaretIntent(target.caret);
      deps.setSelectedBlockId(target.id);
      deps.setEditingId(target.id);
    },
  };
}
