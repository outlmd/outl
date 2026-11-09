/**
 * The one answer to "what will the next keystroke do?", shared by the
 * keydown dispatcher and the status-bar badge.
 *
 * Two copies used to exist and disagreed (issue #489): the badge said
 * INSERT while the dispatcher ran Normal commands. A badge that differs
 * from the dispatcher lies about the next keystroke, so there is no
 * second copy to keep in step.
 */
import type { ShortcutMode } from "./api";
import { appState } from "./store";

export type ResolvedMode = Exclude<ShortcutMode, "global">;

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
