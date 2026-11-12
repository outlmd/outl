/**
 * Switch workspace — File → Switch Workspace… on macOS, the Settings
 * button everywhere.
 *
 * The native menu (`src-tauri/src/menu.rs`, macOS only) reports the click
 * as `menu://switch-workspace`; the Settings button calls
 * `switchWorkspace` directly. Everything else is here: ask for a folder
 * the way the first-run picker does, hand it to `set_workspace`, then
 * drop the screen state that still points at the workspace we left.
 *
 * `set_workspace` already swaps the live workspace (locks, watcher,
 * undo history, sync) and emits `workspace-ready`; what it cannot reach
 * is the Solid store, which would otherwise keep showing the old
 * workspace's page, selection and backlinks until the user navigated.
 */
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { platform } from "@tauri-apps/plugin-os";
import { reconcile } from "solid-js/store";

import { setWorkspace } from "./api";
import { emptyBacklinks, setAppState, setOutline } from "./store";

/** Title of the folder dialog. */
export const SWITCH_WORKSPACE_DIALOG_TITLE = "Switch outl workspace";

/**
 * Whether this build has the native File → Switch Workspace… item.
 * Tauri's default menu, and so ours, exists on macOS only; Windows and
 * Linux have no menu bar, so their copy must not point at one. Anything
 * we cannot identify (no plugin, a browser) is treated as "no menu".
 */
export function hasNativeMenu(): boolean {
  try {
    return platform() === "macos";
  } catch {
    return false;
  }
}

/**
 * Helper line under the workspace path in Settings. The wording of the
 * menu item is `SWITCH_WORKSPACE_LABEL` in `menu.rs`; it is written out
 * here because the Rust side cannot hand a string to the frontend, and
 * `menu.rs`'s test pins that side.
 */
export function switchWorkspaceHint(): string {
  return hasNativeMenu()
    ? "Use File → Switch Workspace… or the button to change."
    : "Use the button to change.";
}

export interface SwitchWorkspaceDeps {
  /** Surface a failure the way the rest of the shell does (the toast). */
  setError: (msg: string) => void;
  /** Load a page of the new workspace once the store has been reset. */
  reload: () => Promise<void>;
}

/**
 * Forget everything that belongs to the page of the workspace being
 * left: the page and its outline, the cursors into it, the slices
 * fetched behind it (backlinks, embeds, parse warnings) and the
 * per-page panel. Panels that fetch on open (tasks, trash, reminders)
 * re-read by themselves and are left alone.
 */
export function resetPageState(): void {
  setAppState({
    page: null,
    selectedPath: null,
    selectedBlockId: null,
    selectedBacklinkBlockId: null,
    editingBlockId: null,
    addPropertyBlockId: null,
    visualAnchorId: null,
    lastVisualRange: null,
    focusBlockId: null,
    mode: "normal",
    timelineOpen: false,
    parseWarnings: [],
    mdAheadOfLog: undefined,
    pageProperties: [],
    lastError: null,
    ...emptyBacklinks(),
  });
  // A store setter merges plain objects, so `{}` alone would keep the old keys.
  setAppState("embeds", reconcile({}));
  setOutline([]);
}

let inFlight = false;

/**
 * Pick a folder and switch to it. Cancelling does nothing; a failure
 * goes to `deps.setError` and leaves the current page on screen. A second
 * call while one is running is ignored, so a double click on the menu
 * item cannot stack two dialogs.
 *
 * Resolves `true` only when the workspace was actually switched, so a
 * caller with its own chrome (the Settings modal) can close it on success
 * and keep it open on cancel or failure.
 */
export async function switchWorkspace(deps: SwitchWorkspaceDeps): Promise<boolean> {
  if (inFlight) return false;
  inFlight = true;
  try {
    const picked = await openDialog({
      directory: true,
      multiple: false,
      title: SWITCH_WORKSPACE_DIALOG_TITLE,
    });
    if (typeof picked !== "string" || !picked) return false;
    await setWorkspace(picked);
    resetPageState();
    await deps.reload();
    return true;
  } catch (e) {
    deps.setError(e instanceof Error ? e.message : String(e));
    return false;
  } finally {
    inFlight = false;
  }
}
