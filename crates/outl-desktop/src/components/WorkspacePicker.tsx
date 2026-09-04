import { createSignal } from "solid-js";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { setWorkspace } from "../lib/api";

/**
 * Initial screen shown when no workspace is loaded.
 *
 * The "Pick…" button calls the Tauri dialog plugin and, on accept,
 * hands the path to the backend's `set_workspace` command. The
 * backend emits `workspace-ready` after opening; `<App />` listens
 * for the event and swaps to `<AppShell />`.
 */
export function WorkspacePicker(props: { onPicked: () => void }) {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  async function pickDirectory() {
    setError(null);
    setBusy(true);
    try {
      const picked = await openDialog({
        directory: true,
        multiple: false,
        title: "Pick a folder for your outl workspace",
      });
      if (typeof picked !== "string" || !picked) {
        setBusy(false);
        return;
      }
      await setWorkspace(picked);
      props.onPicked();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="flex h-full flex-col items-center justify-center gap-6 p-8">
      <div class="text-center">
        <h1 class="text-3xl font-semibold">outl</h1>
        <p class="mt-2 opacity-70">
          Local-first outliner — pick a folder to begin.
        </p>
      </div>

      <button
        type="button"
        onClick={pickDirectory}
        disabled={busy()}
        class="rounded-md bg-(--color-outl-fg)/10 px-4 py-2 text-sm font-medium hover:bg-(--color-outl-fg)/15 disabled:opacity-50"
      >
        {busy() ? "Opening…" : "Pick workspace folder…"}
      </button>

      <div class="max-w-md text-center text-xs opacity-50">
        Suggestion: a folder inside iCloud Drive (macOS), Dropbox, or Syncthing.
        outl writes per-actor op log files that any of those file syncs
        propagate cleanly between devices.
      </div>

      {error() && (
        <div class="rounded bg-(--color-outl-destructive)/15 px-3 py-2 text-sm text-(--color-outl-destructive)">
          {error()}
        </div>
      )}
    </div>
  );
}
