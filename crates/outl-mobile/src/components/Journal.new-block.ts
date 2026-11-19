/**
 * The two block creates that start from a tap taken with the keyboard
 * down: "+" / "Add a block" / "Tap to start writing" (append at the end
 * of the page) and the long-press menu's "New block above".
 *
 * They live together because of what they share, which is not the
 * create — it is the keyboard. Both are two awaits deep (commit the
 * open draft, then `create_block` over IPC) before the new block's
 * textarea can mount and focus itself, and on iOS a `.focus()` that
 * late raises no keyboard. So both arm the keyboard **synchronously,
 * before the first await**, and both carry whatever the user typed
 * during the round-trip into the block they just created.
 * `lib/keyboard-keeper.ts` owns the mechanism and the reasoning.
 *
 * `handleCreateAfter` deliberately stays in `Journal.tsx`: it is a
 * split at the caret, and every way to reach it (Return inside a
 * block, the keyboard bar's "New line") already has the keyboard up,
 * so it has none of this problem.
 *
 * Handed accessors, never values — a destructured prop (or dep) freezes
 * at first render in Solid. No handler reads `this`, so `Journal` can
 * pass them straight to a child as props.
 */

import { batch } from "solid-js";
import type { CreateBlockReply, PageView } from "@outl/shared/api/types";
import { createBlock } from "@outl/shared/api/commands";

import { haptic } from "../lib/haptics";
import {
  armKeyboard,
  releaseKeyboard,
  takeArmedText,
} from "../lib/keyboard-keeper";

export interface JournalNewBlockDeps {
  pageId: () => string | null;
  editingId: () => string | null;
  commitEdit: () => Promise<unknown>;
  applyView: (v: PageView) => void;
  startEdit: (id: string, initial: string) => void;
  withError: <T>(fn: () => Promise<T>) => Promise<T | undefined>;
}

export function createNewBlockOps(deps: JournalNewBlockDeps) {
  /** A new top-level block at the end of the page, in edit mode. */
  async function appendBlock(): Promise<void> {
    const pid = deps.pageId();
    if (!pid) return;
    // First statement that can touch the DOM, and it has to stay first:
    // everything below this line runs outside the tap's gesture window.
    armKeyboard();
    haptic("medium");
    if (deps.editingId()) await deps.commitEdit();
    const reply = await deps.withError(() =>
      createBlock(pid, { afterId: null, parentId: null, text: null }),
    );
    focusNew(reply);
  }

  /**
   * "New block above" (long-press menu, RFC 0254 phase 4b — mirrors the
   * desktop's `O` / `NewBlockAbove`). Uses `beforeId`, the same
   * floor-slot create the desktop uses — never a post-creation
   * `moveBlockUp` walk, which the desktop's own CLAUDE.md flags as the
   * bug this shape replaced.
   */
  async function createBefore(id: string): Promise<void> {
    const pid = deps.pageId();
    if (!pid) return;
    armKeyboard();
    haptic("medium");
    if (deps.editingId()) await deps.commitEdit();
    const reply = await deps.withError(() =>
      createBlock(pid, { beforeId: id, text: "" }),
    );
    focusNew(reply);
  }

  /**
   * Hand the keyboard over from the keeper to the new block's textarea,
   * carrying anything typed in between.
   *
   * The seeded draft is written by the ordinary commit, as an edit
   * against the id `create_block` returned — there is no block id
   * before the backend replies, and keystrokes must never be held
   * against one the op log never issued.
   */
  function focusNew(reply: CreateBlockReply | undefined): void {
    if (!reply) {
      // Nothing to hand focus to. `withError` already toasted the
      // failure; dropping the keyboard is what keeps the user from
      // typing into an invisible field.
      releaseKeyboard();
      return;
    }
    const typed = takeArmedText();
    batch(() => {
      deps.applyView(reply.view);
      deps.startEdit(reply.new_id, typed);
    });
  }

  return { appendBlock, createBefore };
}
