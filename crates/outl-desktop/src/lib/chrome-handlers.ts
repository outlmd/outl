/**
 * Which surface is on screen: the overlays, the side panels, the
 * inline property editor, the window itself — and the `Esc` cascade
 * that closes the topmost one.
 *
 * The cascade is the reason these live together. It has to know the
 * full set of open-able surfaces and their order, so splitting the
 * toggles away from it would leave two lists of the same fact and the
 * `Esc` order would drift the first time a panel was added.
 *
 * `SearchWord{Forward,Backward}` are here, not in `cursor-handlers`:
 * vim's `*` / `#` have no character cursor to read on the desktop, so
 * what they actually do is open the picker with a seed.
 */

import { getCurrentWindow } from "@tauri-apps/api/window";

import { type HandlerCtx, lookupBlock } from "./handler-context";
import { type ActionHandlers } from "./shortcuts";
import { appState, setAppState } from "./store";
import { exitVisual } from "./visual-handlers";

/** Pre-fill the picker with the selected block's text, then open
 *  it. Powers `*` / `#` — vim's "search the word under cursor"
 *  collapses to "search for something inside this block" on the
 *  desktop because Normal mode has no character cursor.
 *
 *  We can't poke the picker's input ref from here (it lives inside
 *  `<Picker />`), so we stash the seed in a transient store field
 *  the picker reads on open. */
function seedPickerWithCurrentBlock() {
  const id = appState.selectedBlockId;
  if (!id) return;
  const block = lookupBlock(id);
  if (!block) return;
  const seed = block.text.trim().split(/\s+/).slice(0, 4).join(" ");
  setAppState("pickerSeed", seed);
  setAppState("pickerOpen", true);
}

export function chromeHandlers(ctx: HandlerCtx): ActionHandlers {
  // Plain record, not Solid props — see `HandlerCtx`.
  // No `safeCall` / `runOn`: nothing here calls the backend, which is
  // the module's boundary rather than an accident.
  const { deps } = ctx;

  /** Flip a chrome flag. Seven chords are exactly this, and writing
   *  each one out invited the next one to be written out differently
   *  (five of them had a block body, two an expression). */
  const toggle =
    (
      flag:
        | "pickerOpen"
        | "settingsOpen"
        | "sidebarOpen"
        | "backlinksOpen"
        | "helpOpen"
        | "remindersOpen"
        | "tasksOpen",
    ) =>
    () =>
      setAppState(flag, !appState[flag]);

  /** Ask the selected block's row for a blank property editor.
   *  Refusing loudly matters: a chord that silently does nothing when
   *  no block is selected gets reported as "the shortcut is broken". */
  const openPropertyEditor = () => {
    const block = appState.selectedBlockId;
    if (!block) {
      deps.setError("select a block first");
      return;
    }
    setAppState("addPropertyBlockId", block);
  };

  return {
    // ── chrome ────────────────────────────────────────────────────
    OpenPicker: toggle("pickerOpen"),
    // The same palette, opened on `>` so it lists actions only.
    OpenCommandPalette: () => setAppState({ pickerSeed: "> ", pickerOpen: true }),
    OpenSettings: toggle("settingsOpen"),
    ToggleSidebar: toggle("sidebarOpen"),
    ToggleBacklinks: toggle("backlinksOpen"),
    ToggleHelp: toggle("helpOpen"),
    // ── read-only panels ──────────────────────────────────────────
    //
    // These two only mount a list. *Authoring* a `remind::` rule is a
    // mutation on the selected block, so `InsertRemind` / the nag
    // preset / `SnoozeReminder` live in `block-handlers.ts`.
    OpenReminders: toggle("remindersOpen"),
    OpenTaskList: toggle("tasksOpen"),
    // ── properties (`key:: value`) ────────────────────────────────
    //
    // The chord has no chip to click, so it names the block and
    // `<BlockRow />` opens a blank editor on it. The editor clears the
    // signal when it closes, which is what lets a second press
    // re-open it on the same block.
    //
    // `OpenProperties` (the TUI's `g p`) resolves here too: that
    // overlay's job is to *list* a block's properties, and the desktop
    // already renders that list permanently as the chip row. The only
    // half it lacks is the blank pair, so both chords open it.
    AddProperty: openPropertyEditor,
    OpenProperties: openPropertyEditor,
    Quit: async () => {
      // `qq` chord in Normal + `Ctrl+C` Global. Close the active
      // window; Tauri tears the app down once the last window
      // closes.
      try {
        await getCurrentWindow().close();
      } catch (e) {
        deps.setError(e instanceof Error ? e.message : String(e));
      }
    },

    // ── Search the workspace for the selected block's text (* / #) ──
    //
    // Vim's `*` / `#` are "search the word under the cursor". The
    // desktop's Normal mode has no character cursor — only a
    // selected block — so the closest useful gesture is "search for
    // something in this block's text". We pre-fill the picker; the
    // user can refine the query before accepting.
    SearchWordForward: seedPickerWithCurrentBlock,
    // `#` (backward) collapses to the same gesture on the desktop —
    // the picker is bidirectional (fuzzy match, not directional).
    SearchWordBackward: seedPickerWithCurrentBlock,

    // ── overlays + insert escape ─────────────────────────────────
    //
    // Esc cascades: close the topmost overlay first (Help → Picker
    // → Settings), and if no overlay is up but a textarea is
    // focused, blur it. The blur fires `<BlockRow />`'s `onBlur`
    // handler (which commits + flips `editingBlockId` to null), so
    // the user is back in Normal mode without a second key.
    ExitInsert: () => {
      // Every closable surface, innermost first; one Esc closes one.
      // One list, because the order IS the fact this handler owns —
      // the panels as an array plus the overlays as three hand-written
      // branches was that order written in two shapes, and the module
      // doc above says why that is the thing to avoid here.
      const open = (
        [
          "timelineOpen",
          "remindersOpen",
          "tasksOpen",
          "trashOpen",
          "helpOpen",
          "pickerOpen",
          "settingsOpen",
        ] as const
      ).find((k) => appState[k]);
      if (open) {
        setAppState(open, false);
        return;
      }
      // Leaving Visual via Esc — capture the range for `gv`, drop
      // back to Normal. Must come before the textarea-blur branch
      // (Visual mode has no focused textarea, so the blur is a no-op
      // anyway; the explicit return keeps the mode flip atomic).
      if (appState.mode === "vim-visual") {
        exitVisual();
        return;
      }
      const el = document.activeElement;
      if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement) {
        el.blur();
        return;
      }
      // Normal-mode `Esc` with nothing focused: cancel a pending cut
      // so the dimmed block snaps back. A copy stays on the clipboard
      // (it's non-destructive and reusable, like the OS clipboard).
      if (appState.blockClipboard?.kind === "cut") {
        setAppState("blockClipboard", null);
      }
    },
  };
}
