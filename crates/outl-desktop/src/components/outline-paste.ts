/**
 * The desktop's half of an external-clipboard paste.
 *
 * `<BlockEditor />` decides the route (`choosePasteRoute`) and owns the
 * textarea; this owns the three steps between that decision and the
 * workspace. The third is the one easy to drop, and dropping it was
 * [issue #488](https://github.com/outlmd/outl/issues/488): a caret
 * paste rewrites the block the user is typing in, the draft is seeded
 * once per row, so an editor left on the pre-paste string commits it
 * straight back over the paste on the next Esc or blur.
 *
 * Its own module rather than two more closures in `<OutlineView />` for
 * the reason mobile's `Journal.block-ops.ts` is one: the sequence is
 * worth a test of its own, and a component that mounts the whole
 * 3-pane shell is not where that test can live.
 *
 * Deps are accessors, never values — Solid's reactivity rides the
 * getter, and a snapshot taken here would freeze at first render.
 */

import type { PageView, PasteReply } from "@outl/shared/api/types";
import { editBlock } from "@outl/shared/api/commands";
import {
  pasteEditorUpdate,
  type PasteEditorUpdate,
} from "@outl/shared/paste";

export interface PasteOpsDeps {
  /** The open page's id, or `undefined` before the first view lands. */
  pageId: () => string | undefined;
  /** The block in edit mode, or `null` when none is. */
  editingId: () => string | null;
  /** Fold a fresh view into the store. */
  applyView: (view: PageView) => void;
  /** Await a command, recording a failure instead of throwing. */
  handleError: <T>(promise: Promise<T>) => Promise<T | undefined>;
}

export function createPasteOps(deps: PasteOpsDeps) {
  /**
   * Persist the in-flight textarea draft before a paste splices at
   * `caret`. The caret is measured against the draft
   * (`textarea.value`), but the backend splices against
   * `host_text_for_caret` — if the user typed since the last commit the
   * two diverge and the paste lands at the wrong offset. No-op when the
   * block isn't the one being edited (a paste from a click without an
   * open editor). Edit mode is left untouched (`editBlock` doesn't flip
   * `editingBlockId`), so the user keeps typing after a plain paste.
   */
  async function flushDraft(pageId: string, id: string, hostText: string) {
    if (deps.editingId() !== id) return;
    const committed = await deps.handleError(editBlock(pageId, id, hostText));
    if (committed) deps.applyView(committed);
  }

  /**
   * Run one of the two paste commands and hand the editor back what it
   * has to become. One function for both because they differ only in
   * which command runs.
   */
  async function run(
    id: string,
    hostText: string,
    command: (pageId: string) => Promise<PasteReply>,
  ): Promise<PasteEditorUpdate | null> {
    const pageId = deps.pageId();
    if (!pageId) return null;
    await flushDraft(pageId, id, hostText);
    const reply = await deps.handleError(command(pageId));
    if (!reply) return null;
    deps.applyView(reply.view);
    return pasteEditorUpdate(reply);
  }

  // `flushDraft` stays private: `run` already runs it, first.
  return { run };
}
