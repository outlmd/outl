/**
 * `createCaretNav(deps)` — the mobile half of an arrow key that walks
 * out of the block being edited.
 *
 * `@outl/shared/caret-nav` owns the whole decision (whether the
 * keystroke leaves the block, and which block it hands the caret to),
 * so what is here is the two things only this client knows: which
 * blocks are on screen, and that entering Insert here means
 * `startEdit` plus a caret parked once the textarea mounts. The
 * desktop's `outline-caret-nav.ts` is the same wrapper over the same
 * pair, which is what stops the two from drifting.
 *
 * **Only a hardware keyboard reaches this** — an iPad with a keyboard,
 * or an Android phone with one attached. The soft keyboard has no arrow
 * keys, so on a bare phone this code never runs.
 *
 * Two rules, and both are about ordering:
 *
 * - **The answer is synchronous, the work is not.** The textarea needs
 *   to know *now* whether to swallow the key (`preventDefault` only
 *   counts during the dispatch), while the commit is a backend
 *   round-trip. So the target is resolved up front and the commit runs
 *   behind it — async-by-default, like every other write in this client.
 * - **The commit lands before the new block is opened.** `commitEdit`
 *   ends with `setEditingId(null)`, so opening the neighbour first and
 *   committing after would drop the user straight back out of Insert.
 *   A commit that fails therefore moves nothing: `commitEdit` keeps the
 *   editor open on the draft so the user can retry, and walking the
 *   cursor onto another block on top of that would strand text they
 *   can no longer see. It **resolves** `false` there rather than
 *   rejecting (`withError` already surfaced the message), so the step
 *   reads the answer: `startEdit` would overwrite the unsaved draft.
 * - **One step at a time.** Holding an arrow is the gesture this
 *   exists for, and the key repeats faster than a save round-trip, so
 *   a repeat landing while a step is in flight is swallowed rather
 *   than starting a second commit of the same draft.
 *
 * Deps are accessors, never values: a destructured prop freezes at
 * first render in Solid, and the same rule governs a dependency object.
 */

import { createSignal } from "solid-js";

import type { BlockNode } from "@outl/shared/api/types";
import { type CaretExit, resolveCaretExit } from "@outl/shared/caret-nav";
import { charOffsetToUtf16Offset } from "@outl/shared/paste";
import { parkCaret } from "../lib/textarea";

export interface CaretNavDeps {
  /** The blocks the cursor may walk: the zoom's subtree, else the page. */
  navBlocks: () => BlockNode[];
  /** Flush the in-flight draft and leave Insert. Resolves `false`
   *  when the save failed and the editor is still open on the draft. */
  commitEdit: () => Promise<boolean>;
  /** Enter Insert on a block, seeding the draft with its wire text. */
  startEdit: (id: string, initial: string) => void;
}

export function createCaretNav(deps: CaretNavDeps) {
  /**
   * Codepoint offset the caret should open at in the next textarea to
   * mount, or `null` for the end of the text.
   *
   * Owned here, like `Journal.embeds.ts` owns its own, so the component
   * holds no state for the feature. It is a pending value rather than a
   * prop threaded down through `BlockRow` and `BlockBody`: the element
   * is what has to be written to, and `Journal` already receives it
   * through `onTextareaMount`.
   */
  const [pending, setPending] = createSignal<number | null>(null);
  /** A step's commit is still in flight. */
  let stepping = false;
  /**
   * Hand Insert mode to `id`'s neighbour in `exit`'s direction.
   *
   * Answers whether the step was taken. `false` is the outline's edge
   * (or a block that left the walk), where the webview's own caret
   * motion is still the right answer and nothing is committed — a `←`
   * in the page's first block must not drop the user out of Insert.
   */
  function onCaretExit(id: string, exit: CaretExit): boolean {
    // Swallowed: the caret is already on its way out of this block.
    if (stepping) return true;
    const target = resolveCaretExit(deps.navBlocks(), id, exit);
    if (!target) return false;
    stepping = true;
    void (async () => {
      let left = false;
      try {
        left = await deps.commitEdit();
      } catch {
        // Anything that escapes `commitEdit` is a bug there, and the
        // right answer here is still to leave the user on their unsaved
        // text. Caught rather than left to `void`, which would make it
        // an unhandled rejection.
      } finally {
        stepping = false;
      }
      if (!left) return;
      setPending(target.caret);
      deps.startEdit(target.id, target.text);
    })();
    return true;
  }

  /**
   * Open `el`'s caret where the last step asked, if one did. Every
   * other entry into Insert leaves the textarea's own "end of the text"
   * alone, because a tap into a block means "type here".
   *
   * `parkCaret` rather than `setSelectionRange`, because it re-places
   * the caret inside a microtask and so survives the Solid `value=`
   * re-binding that a raw call loses on iOS.
   */
  function applyPendingCaret(el: HTMLTextAreaElement) {
    // `!= null`, not a truthy check: offset 0 is where a `→` step lands
    // and is falsy.
    const caret = pending();
    if (caret == null) return;
    setPending(null);
    parkCaret(el, charOffsetToUtf16Offset(el.value, caret));
  }

  return { onCaretExit, applyPendingCaret };
}
