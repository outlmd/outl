/**
 * What state is the editor in — resolved from focus and the store, once.
 *
 * {@link detectMode} answers "what will the next keystroke do?", shared
 * by the keydown dispatcher and the status-bar badge. Two copies used to
 * exist and disagreed (issue #489): the badge said INSERT while the
 * dispatcher ran Normal commands. A badge that differs from the
 * dispatcher lies about the next keystroke, so there is no second copy
 * to keep in step.
 *
 * {@link isEditing} answers the neighbouring question every background
 * refresh asks before it runs, and it is here for the same reason.
 */
import type { ShortcutMode } from "./api";
import { backlinkRowIds } from "./cursor-region";
import { appState } from "./store";

export type ResolvedMode = Exclude<ShortcutMode, "global">;

/**
 * Is a block editor on screen with the user's hands in it?
 *
 * **The one owner**, because five background refreshes ask it before
 * they re-materialize the tree — a peer streaming ops, the reconcile
 * pass, the capture-inbox drain, the midnight jump to today, and the
 * drain that waits for the user to leave. Each used to read
 * `appState.editingBlockId` directly, and each carries a comment saying
 * why ("reloading while the user edits re-materializes the tree under
 * the cursor"). A **backlink** row's editor is just as much an editor
 * and was invisible to all five, so the feature that made that row
 * editable put a textarea inside the one list nothing protected: any
 * refetch replaces `appState.backlinks` with fresh objects, `<For>` keys
 * by reference, and the row remounts with the draft re-seeded from disk.
 * The typed text was gone.
 *
 * It asks whether the editor is **on screen**, not whether the field is
 * set, and that is load-bearing in the backlink case. The editor is
 * rendered by the row, so anything that takes the row away takes the
 * editor with it while `editingBacklink` stays behind — a reply that
 * drops the row, `Cmd+Shift+B` (a **`Global`** chord, so it fires from
 * inside the textarea), or a zoom. A guard that believed the field alone
 * would then defer every reload for the rest of the session.
 *
 * So "on screen" is `backlinkRowIds()`'s answer, not a second reading of
 * it: the same owner the cursor asks, so the editor and the cursor cannot
 * disagree about whether the section is there. Same shape as
 * `backlinkCursor()` — the rule lives in the accessor, so no writer has
 * to know it exists.
 */
export function isEditing(): boolean {
  if (appState.editingBlockId !== null) return true;
  const row = appState.editingBacklink;
  return row !== null && backlinkRowIds().includes(row.blockId);
}

export function detectMode(): ResolvedMode {
  if (appState.pickerOpen || appState.settingsOpen || appState.helpOpen)
    return "overlay";
  // Visual is real state with no textarea focused (a range of selected
  // blocks), and `SelectRange*` / the Visual handlers need it ahead of
  // the focus check below.
  if (appState.mode === "vim-visual") return "visual";
  // A focused editor outranks the resting `vim-normal` that
  // `exitVisual()` leaves behind and nothing ever clears (issue #489):
  // trusting the label first turned every later keystroke in a block
  // into a command (`c` folded, `dd` deleted the block being typed in)
  // until the app restarted. Focus is what the user can see; the label
  // is bookkeeping.
  const el = document.activeElement;
  if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) {
    return "insert";
  }
  // Nothing focused: a vim user's explicit mode still decides, and the
  // no-vim user falls through to Normal.
  if (appState.mode === "vim-insert") return "insert";
  return "normal";
}
