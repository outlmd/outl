/**
 * What happens to **one** block — the single-block half of the journal's
 * mutation surface (issue #265 phase 1).
 *
 * Every structural op here is the same four-step shape: resolve the page
 * id, fire one backend command through `withError`, fold the returned
 * `PageView` back in with `applyView`, and buzz. That repetition is why
 * they belong together and why they belong out of `Journal()`: the shape
 * is the module's contract, not incidental. The clipboard ops are the
 * exception and say so at their own definitions, because neither
 * `copyBlock` nor `copyBlockRef` mutates the workspace.
 *
 * The range twins live in `Journal.selection-ops.ts`. They are a
 * different question ("what is selected, and what happens to N blocks")
 * and share only `JournalBlockDeps`, so splitting on that seam keeps
 * each module answerable on its own rather than merely shorter.
 *
 * Nothing here reads a signal it was not handed. `Journal` passes
 * accessors, never values, so Solid's reactivity survives the move —
 * the one bug this kind of extraction reliably introduces, and a silent
 * one.
 */

import type { PageView } from "@outl/shared/api/types";
import {
  copyBlockMarkdown,
  copyBlockRef,
  cutBlock,
  deleteBlock,
  editBlock,
  indentBlock,
  moveBlockDown,
  moveBlockUp,
  outdentBlock,
  pasteBlockAfter,
  pasteMarkdown,
  toggleTodo,
} from "@outl/shared/api/commands";
import { countDescendants, findBlock, rawTextWithTodo } from "@outl/shared/outline";
import { pasteEditorUpdate } from "@outl/shared/paste";

import { haptic } from "../lib/haptics";
import { parkCaret, spliceText } from "../lib/textarea";

/**
 * The slice of `Journal`'s state a block operation needs.
 *
 * Shared with `Journal.selection-ops.ts`: a range op is N single-block
 * ops plus a selection, so it needs everything here and then some.
 */
export interface JournalBlockDeps {
  /** The open page's id, or `null` before the first view lands. */
  pageId: () => string | null;
  /** The live view — read for the outline, never mutated here. */
  view: () => PageView | null;
  /** Run a backend command, recording a failure instead of throwing. */
  withError: <T>(fn: () => Promise<T>) => Promise<T | undefined>;
  /** Fold a fresh view in (clears zoom / selection on a page switch). */
  applyView: (v: PageView) => void;
  /** Set the view without `applyView`'s page-switch bookkeeping. */
  setView: (v: PageView) => void;
  editingId: () => string | null;
  setEditingId: (id: string | null) => void;
  draft: () => string;
  setDraft: (text: string) => void;
  blockClipboard: () => string | null;
  setBlockClipboard: (markdown: string) => void;
  setPendingDelete: (pending: { id: string; descendants: number }) => void;
  /**
   * The textarea the user is typing in, or `null` when no block is in
   * edit mode. Needed by the ops that write into the live editor rather
   * than only into the workspace — a caret paste is one, since the
   * block it rewrites is the block being edited.
   */
  activeTextarea: () => HTMLTextAreaElement | null;
}

export function createBlockOps(deps: JournalBlockDeps) {
  /**
   * The shape every structural op shares: page id, haptic, one command,
   * fold the view back in. Extracted so indent / outdent / move cannot
   * drift from each other in error handling or feedback — they did not,
   * but four copies of five lines is four places to forget `withError`.
   */
  async function structural(
    id: string,
    op: (pid: string, id: string) => Promise<PageView>,
  ): Promise<void> {
    const pid = deps.pageId();
    if (!pid) return;
    haptic("light");
    const next = await deps.withError(() => op(pid, id));
    if (next) deps.applyView(next);
  }

  /**
   * A named function rather than a method on the returned literal only
   * because `requestDelete` calls it directly.
   *
   * The rule the whole module obeys is the weaker one: **no handler here
   * reads `this`.** `Journal` hands every one of them to `<BlockRow />`
   * and the context menu as props, so they are always invoked detached
   * from the object, where a `this.` is `undefined` and fails silently.
   * Method shorthand is safe under that rule, which is why most of them
   * keep it. `Journal.block-ops.test.ts` pins the detached call, not the
   * spelling, so the guarantee survives someone changing the spelling.
   */
  async function performDelete(id: string): Promise<void> {
    const pid = deps.pageId();
    if (!pid) return;
    if (deps.editingId() === id) deps.setEditingId(null);
    const next = await deps.withError(() => deleteBlock(pid, id));
    if (next) deps.applyView(next);
  }

  return {
    performDelete,

    async toggleTodo(id: string) {
      const pid = deps.pageId();
      if (!pid) return;
      haptic("medium");
      const wasEditing = deps.editingId() === id;
      if (wasEditing) {
        // Commit current draft text into the workspace so the cycle
        // operates on what the user typed, without dropping out of
        // edit mode (we want the keyboard to stay up).
        const committed = await deps.withError(() => editBlock(pid, id, deps.draft()));
        if (committed) deps.setView(committed);
      }
      const next = await deps.withError(() => toggleTodo(pid, id));
      if (!next) return;
      deps.applyView(next);
      if (wasEditing) {
        // Keep edit mode on the same block; refresh draft to the
        // backend's view, **with** the TODO/DONE prefix reattached so
        // the editor stays consistent with what the user just toggled.
        const block = findBlock(next.outline, id);
        if (block) deps.setDraft(rawTextWithTodo(block));
      }
    },

    /**
     * Delete a block. When the block has descendants we *always*
     * prompt — deleting a parent destroys the whole subtree, and while
     * the keyboard toolbar's Undo button (RFC 0254 phase 1) can revert
     * it, that is a second, less immediate tap than the confirm dialog
     * already in front of the user. Leaf blocks delete immediately (no
     * prompt) to keep the swipe gesture fast.
     */
    requestDelete(id: string) {
      const cur = deps.view();
      if (!cur) return;
      const block = findBlock(cur.outline, id);
      const descendants = block ? countDescendants(block) : 0;
      if (descendants > 0) {
        haptic("warning");
        deps.setPendingDelete({ id, descendants });
        return;
      }
      haptic("heavy");
      void performDelete(id);
    },

    /**
     * "Copy block" (long-press menu, RFC 0254 phase 2) — arm the
     * in-app block clipboard with `id`'s subtree as clean outl
     * markdown, ready for "Paste block" on another row. Distinct from
     * the existing "Copy text" action: that one writes straight to the
     * OS clipboard for pasting outside outl (desktop's `Y` /
     * `YankCurrentBlock`); this one never touches the OS clipboard —
     * it's the desktop's `Cmd/Ctrl+C` (`CopyBlock`) view-mode gesture,
     * just reached by long-press instead of a chord. `copyBlockMarkdown`
     * is read-only, so arming never mutates the workspace.
     */
    async copyBlock(id: string) {
      const markdown = await deps.withError(() => copyBlockMarkdown(id));
      if (markdown !== undefined) deps.setBlockClipboard(markdown);
    },

    /**
     * "Paste block" (long-press menu) — duplicate the armed clipboard's
     * subtree as a sibling right after `id`, minting fresh ids
     * (`paste_block_after`, same backend the desktop's `Cmd/Ctrl+V`
     * calls for a `kind: "copy"` clipboard). The clipboard persists
     * after a successful paste so it can be pasted again, mirroring the
     * desktop's non-cut branch. Only reachable when `blockClipboard()`
     * is armed — the context menu hides the action otherwise.
     */
    async pasteBlock(id: string) {
      const pid = deps.pageId();
      const markdown = deps.blockClipboard();
      if (!pid || markdown === null) return;
      const next = await deps.withError(() => pasteBlockAfter(pid, id, markdown));
      if (next) deps.applyView(next);
    },

    /**
     * "Cut block" (long-press menu, RFC 0254 phase 4b) — render `id`'s
     * subtree to markdown and delete it in one backend round-trip
     * (`cutBlock`), then arm the same `blockClipboard` "Paste block"
     * reads. Deliberately **not** identity-preserving (the paste mints
     * fresh ids, per `cutBlock`'s doc comment) — the alternative is the
     * desktop's move-based cut, which needs a `{kind, nodeId}` tagged
     * clipboard this client doesn't have and doesn't need for a
     * long-press gesture.
     */
    async cutBlock(id: string) {
      const pid = deps.pageId();
      if (!pid) return;
      const reply = await deps.withError(() => cutBlock(pid, id));
      if (!reply) return;
      if (deps.editingId() === id) deps.setEditingId(null);
      deps.setBlockClipboard(reply.markdown);
      deps.applyView(reply.view);
    },

    /**
     * "Copy block ref" (long-press menu, issue #18) — resolve `id`'s
     * `((blk-XXXXXX))` handle and put it on the OS clipboard, same
     * best-effort posture as "Copy text" above (some webviews refuse
     * `navigator.clipboard` outside a user-gesture chain).
     */
    async copyBlockRef(id: string) {
      const ref = await deps.withError(() => copyBlockRef(id));
      if (ref === undefined) return;
      try {
        await navigator.clipboard?.writeText(ref);
      } catch {
        // Best-effort, same as "Copy text" / "Copy block" above.
      }
    },

    /**
     * An external-clipboard paste that landed inside a block's
     * textarea. `<BlockTextarea />` already chose the route
     * (`choosePasteRoute`) and called `preventDefault`, so what arrives
     * is the markdown the backend should splice at `caret`.
     *
     * Three steps, and the last two are the ones easy to skip:
     *
     * 1. Flush the in-flight draft, so the backend splices at a caret
     *    measured against the text it is holding.
     * 2. Fold the fresh view in, as every op does.
     * 3. **Put the host block's new text back into the editor.** The
     *    draft is seeded once by `startEdit`, so stopping at step 2
     *    left the textarea on the pre-paste string, and `commitEdit`
     *    wrote that back over the paste on the next blur — issue #488,
     *    a pasted link gone from the block and from the `.md` with
     *    nothing said.
     *
     * The caret comes off the reply, never recomputed as "where I
     * pasted plus what I sent": the backend built the string and may
     * have inserted a separating space.
     */
    async pasteMarkdownAt(blockId: string, caret: number, text: string) {
      const pid = deps.pageId();
      if (!pid) return;
      if (deps.editingId() === blockId) {
        const committed = await deps.withError(() =>
          editBlock(pid, blockId, deps.draft()),
        );
        if (committed) deps.setView(committed);
      }
      const reply = await deps.withError(() =>
        pasteMarkdown(pid, blockId, caret, text),
      );
      if (!reply) return;
      deps.applyView(reply.view);
      const update = pasteEditorUpdate(reply);
      // The guard is the round-trip: the user can tap out of the block
      // while it runs, and the draft then belongs to another block.
      if (!update || deps.editingId() !== blockId) return;
      const el = deps.activeTextarea();
      if (!el) {
        deps.setDraft(update.text);
        return;
      }
      // `spliceText` + double `parkCaret`, the pattern `lib/textarea.ts`
      // owns: a bare `el.value =` jumps the caret to the end on
      // WKWebView, and Solid's `value={draft()}` effect fires after
      // `setDraft` and would jump it again.
      spliceText(el, 0, el.value.length, update.text);
      parkCaret(el, update.caret);
      deps.setDraft(update.text);
      parkCaret(el, update.caret);
    },

    indent: (id: string) => structural(id, indentBlock),
    outdent: (id: string) => structural(id, outdentBlock),
    moveUp: (id: string) => structural(id, moveBlockUp),
    moveDown: (id: string) => structural(id, moveBlockDown),
  };
}
