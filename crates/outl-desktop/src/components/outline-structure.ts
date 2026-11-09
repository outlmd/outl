/**
 * The desktop's half of a Tab / Shift-Tab indent or outdent run from
 * inside a block's textarea.
 *
 * `<BlockEditor />`'s Tab handler commits the in-flight draft before
 * calling these — a commit flips `editingBlockId` to `null` (or, for
 * an unchanged draft, does so directly) so the row re-renders out of
 * edit mode. Without re-entering it here, the moved block's row
 * re-renders in its new position with a fresh `<BlockEditor />`
 * instance that never mounts, and the next keystroke runs as a
 * Normal-mode command against whatever was selected before (#477).
 *
 * `onIndent` / `onOutdent` always put the moved block back into edit
 * mode on *itself*, regardless of whether the move landed, was a
 * backend no-op (`indent_block` / `outdent_block` swallow
 * `NoPreviousSibling` / `AlreadyAtRoot` and return the unchanged view),
 * or the command rejected outright — the user never asked to leave
 * edit mode, so a failure must not strand them outside it either.
 *
 * Its own module for the same reason `./outline-paste` is one: the
 * sequence is worth a test of its own, and `<OutlineView />` mounts the
 * whole 3-pane shell, which is not where that test can live.
 *
 * Deps are accessors, never values — Solid's reactivity rides the
 * getter, and a snapshot taken here would freeze at first render.
 */

import type { PageView } from "@outl/shared/api/types";
import { indentBlock, outdentBlock } from "@outl/shared/api/commands";

export interface StructureOpsDeps {
  /** The open page's id, or `undefined` before the first view lands. */
  pageId: () => string | undefined;
  /** Fold a fresh view into the store. */
  applyView: (view: PageView) => void;
  /** Await a command, recording a failure instead of throwing. */
  handleError: <T>(promise: Promise<T>) => Promise<T | undefined>;
  setSelectedBlockId: (id: string) => void;
  setEditingId: (id: string) => void;
}

export function createStructureOps(deps: StructureOpsDeps) {
  async function run(
    id: string,
    command: (pageId: string, id: string) => Promise<PageView>,
  ) {
    const pageId = deps.pageId();
    if (!pageId) return;
    const view = await deps.handleError(command(pageId, id));
    if (view) deps.applyView(view);
    deps.setSelectedBlockId(id);
    deps.setEditingId(id);
  }

  return {
    onIndent: (id: string) => run(id, indentBlock),
    onOutdent: (id: string) => run(id, outdentBlock),
  };
}
