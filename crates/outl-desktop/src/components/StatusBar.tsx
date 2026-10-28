import { For, Show } from "solid-js";

import { appState, setAppState } from "../lib/store";

/**
 * Bottom status bar — mirrors the TUI's footer.
 *
 * ```
 *  NORMAL · outl · 4 backlinks · i edit · o new · c fold · Enter open ref · …  · ? help · q
 * ```
 *
 * Mode badge picks up the theme tokens (`status_normal_*` etc) so
 * each preset colors it consistently with the TUI's badge.
 */

type Mode = "normal" | "insert" | "visual" | "overlay";

function detectMode(): Mode {
  if (appState.pickerOpen || appState.settingsOpen || appState.helpOpen)
    return "overlay";
  // Vim mode owns the badge when active. Check before the DOM-focus
  // fallback so `vim-visual` doesn't get swallowed by "insert" when a
  // textarea happens to be focused, and so Visual reads as VISUAL
  // instead of NORMAL (the badge used to lie during Visual sessions).
  if (appState.mode === "vim-visual") return "visual";
  if (appState.mode === "vim-insert") return "insert";
  const el = document.activeElement;
  if (el instanceof HTMLTextAreaElement || el instanceof HTMLInputElement)
    return "insert";
  return "normal";
}

interface HintEntry {
  chord: string;
  label: string;
}

/** Mode-specific footer hints — same shortcuts users see in
 *  the TUI's hint strip, tuned to the desktop conventions. */
function hintsFor(mode: Mode): HintEntry[] {
  switch (mode) {
    case "insert":
      return [
        { chord: "Esc", label: "commit" },
        { chord: "Enter", label: "newline" },
        { chord: "⌘Enter", label: "new block" },
        { chord: "Tab", label: "indent" },
        { chord: "⌘B", label: "bold" },
        { chord: "⌘I", label: "italic" },
        { chord: "⌘T", label: "todo" },
        { chord: "[[", label: "ref" },
      ];
    case "visual":
      return [
        { chord: "j/k", label: "extend" },
        { chord: "y", label: "yank" },
        { chord: "d", label: "delete" },
        { chord: "Esc", label: "exit" },
      ];
    case "overlay":
      return [
        { chord: "↑/↓", label: "move" },
        { chord: "Enter", label: "open" },
        { chord: "Esc", label: "close" },
      ];
    case "normal":
    default:
      return [
        { chord: "i", label: "edit" },
        { chord: "o", label: "new" },
        { chord: "c", label: "fold" },
        { chord: "Enter", label: "open ref" },
        { chord: "K/J", label: "move" },
        { chord: "t", label: "today" },
        { chord: "⌘T", label: "todo" },
        { chord: "u", label: "undo" },
        { chord: "?", label: "help" },
        { chord: "qq", label: "quit" },
      ];
  }
}

/** Background + foreground classes for the mode badge. Each mode
 *  pulls from the matching `--color-outl-status-*` token so theme
 *  swap propagates. */
function badgeClass(mode: Mode): string {
  switch (mode) {
    case "insert":
      return "bg-(--color-outl-status-insert-bg) text-(--color-outl-status-insert-fg)";
    case "visual":
      return "bg-(--color-outl-status-visual-bg) text-(--color-outl-status-visual-fg)";
    case "overlay":
      return "bg-(--color-outl-bg-elev) text-(--color-outl-fg)";
    case "normal":
    default:
      return "bg-(--color-outl-status-normal-bg) text-(--color-outl-status-normal-fg)";
  }
}

function modeLabel(mode: Mode): string {
  switch (mode) {
    case "insert":
      return "INSERT";
    case "visual":
      return "VISUAL";
    case "overlay":
      return "OVERLAY";
    case "normal":
    default:
      return "NORMAL";
  }
}

export function StatusBar() {
  // detectMode is recomputed on every render thanks to Solid's
  // reactivity through appState — focus changes inside the
  // textarea don't trigger by themselves, but every user-driven
  // mutation flips a Solid signal that the shell re-renders on.
  const mode = (): Mode => detectMode();
  const hints = (): HintEntry[] => hintsFor(mode());

  return (
    <footer class="flex h-7 shrink-0 items-center gap-3 overflow-hidden border-t border-(--color-outl-border) bg-(--color-outl-bg) px-2 text-[11px]">
      {/* Mode badge — bold, accent background. */}
      <span
        class={`rounded px-1.5 py-[1px] font-mono text-[10px] font-semibold uppercase tracking-wider ${badgeClass(mode())}`}
      >
        {modeLabel(mode())}
      </span>

      {/* Workspace + counts. */}
      <span class="text-(--color-outl-fg-dim)">
        <span class="font-medium">outl</span>
        <Show when={appState.workspace}>
          <span class="opacity-60">
            {" · "}
            {appState.workspace?.blocks ?? 0} blocks
          </span>
          {/* `ops` is the resident log, which after a snapshot boot is
              only the delta above the cutoff. A partial count rendered as
              a total reads as data loss, so it says which it is — the same
              rule `outl workspace info` follows. Compared against `false`
              so an older backend with no flag still reads as complete. */}
          <span
            class="opacity-60"
            title={
              appState.workspace?.ops_complete === false
                ? "Ops replayed since the last snapshot, not the whole log. `outl doctor` counts all of it."
                : undefined
            }
          >
            {" · "}
            {appState.workspace?.ops ?? 0} ops
            <Show when={appState.workspace?.ops_complete === false}>
              {" since snapshot"}
            </Show>
          </span>
        </Show>
        <Show when={appState.backlinks.length > 0}>
          <span class="opacity-60">
            {" · "}
            {appState.backlinks.length} backlinks
          </span>
        </Show>
      </span>

      {/* Hints — fill remaining space; truncate from the end. */}
      <span class="ml-auto flex min-w-0 items-center gap-3 overflow-hidden whitespace-nowrap text-(--color-outl-fg-dim)">
        <For each={hints()}>
          {(h) => (
            <span class="flex shrink-0 items-center gap-1">
              <span class="rounded bg-(--color-outl-bg-elev) px-1 py-[1px] font-mono text-[10px] text-(--color-outl-fg)">
                {h.chord}
              </span>
              <span class="opacity-70">{h.label}</span>
            </span>
          )}
        </For>
      </span>

      {/* Inline error indicator with dismiss — replaces the modal
       *  toast we had before. TUI shows status_message in the same
       *  footer row, so we follow that pattern. */}
      <Show when={appState.lastError}>
        <span class="flex shrink-0 items-center gap-1 rounded bg-(--color-outl-status-message-fg)/10 px-2 py-[1px] text-(--color-outl-status-message-fg)">
          ⚠ {appState.lastError}
          <button
            type="button"
            onClick={() => setAppState("lastError", null)}
            class="ml-1 opacity-70 hover:opacity-100"
            aria-label="Dismiss error"
          >
            ✕
          </button>
        </span>
      </Show>
    </footer>
  );
}
