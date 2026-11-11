/**
 * Concrete `Action -> handler` map for the desktop client.
 *
 * Built once in `<AppShell />` and handed to `installShortcuts`.
 * Each entry maps an `outl_shortcuts::Action` variant to the Tauri
 * call (or store mutation) that materialises it on the desktop.
 *
 * The map mirrors the TUI's vim bindings 1:1 — `j/k` move the
 * selection, `i/o/O` enter Insert, `Tab/Shift-Tab` indent /
 * outdent, `dd` deletes, `c` folds, `Cmd+T` cycles TODO — so a
 * user moving between clients keeps their muscle memory.
 *
 * The selection cursor lives in `appState.selectedBlockId`;
 * `<BlockRow />` highlights it. Auto-selection of the first block
 * runs from `<OutlineView />` so the dispatcher never has to worry
 * about a `null` cursor.
 *
 * This file is the assembly only. The handlers live in sibling
 * modules split by **what a keystroke changes** — the surface on
 * screen, where the cursor is, the block under it, the whole page,
 * a Visual range, a clipboard register. A new handler goes in the
 * sibling that owns its answer to that question, never back here.
 */

import { blockHandlers } from "./block-handlers";
import { chromeHandlers } from "./chrome-handlers";
import { clipboardHandlers } from "./clipboard-handlers";
import { cursorHandlers } from "./cursor-handlers";
import { createHandlerCtx, type DesktopHandlerDeps } from "./handler-context";
import { pageHandlers } from "./page-handlers";
import { type ActionHandlers } from "./shortcuts";
import { speedHandlers } from "./speed-handlers";
import { visualHandlers } from "./visual-handlers";

export type { DesktopHandlerDeps };

/**
 * Build the desktop's handler map. Closes over the `applyView` /
 * `setError` callbacks from the shell so each action can refresh
 * the rendered page or surface a problem in the status bar.
 *
 * The groups cannot collide: every `Action` appears in exactly one of
 * them, and `shortcuts.support.test.ts` fails on a key this map gains
 * or loses. So the spread order is readability, not precedence.
 */
export function buildHandlers(deps: DesktopHandlerDeps): ActionHandlers {
  const ctx = createHandlerCtx(deps);
  return {
    ...chromeHandlers(ctx),
    ...cursorHandlers(ctx),
    ...blockHandlers(ctx),
    ...pageHandlers(ctx),
    ...visualHandlers(ctx),
    ...clipboardHandlers(ctx),
    ...speedHandlers(deps),

    // ── deliberately absent ──────────────────────────────────────
    //
    // Every action with no entry here falls through without
    // `preventDefault`, so the textarea or the OS still gets the
    // key, and the dispatcher shows the user the catalog's sentence
    // for why nothing happened.
    //
    // Which ones those are is NOT a comment — comments here went
    // stale twice (this one named `EnterVisual`, `YankRange` and
    // `DeleteRange`, all three of which have handlers a few dozen
    // lines up). It is `outl_shortcuts::support`, rendered in
    // `docs/client-parity.md` and enforced by
    // `shortcuts.support.test.ts`.
  };
}
