import { For, createSignal, onMount, type JSX } from "solid-js";

import { focusMode } from "@outl/shared/polish";
import { pluginRun, pluginToolbar } from "@outl/shared/api/commands";
import type { PluginToolbarButton as ToolbarButtonEntry } from "@outl/shared/api/types";
import { rovingKeydown } from "../lib/keyboard-reach";
import { playPluginViews } from "../lib/plugin-views";
import { appState, setAppState, setOutline } from "../lib/store";
import type { ActionHandlers } from "../lib/shortcuts";
import { SyncIndicator } from "./SyncIndicator";

/**
 * Small round chrome button in the top bar.
 *
 * These surface keyboard chords that are otherwise invisible
 * (`Cmd/Ctrl+Shift+E` for the sidebar, `zM` / `zR` for folding). They carry no business logic — clicking flips the
 * same store signal the `outl-shortcuts` dispatcher flips, so the
 * button and the keyboard stay in sync automatically.
 */
/** Stroke icon in the mobile top bar's style: 24-unit viewBox, round
 *  caps, `currentColor` so it follows the button's active/hover state.
 *  The paths match the shared icons (`@outl/shared/icons`) and the
 *  phone's bottom bar, so the clients read as one app. Kept local
 *  because the shared `Svg` is stroke 1.75 and this is 16px at 2. */
function Icon(props: { children: JSX.Element }) {
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="2"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      {props.children}
    </svg>
  );
}

/** Focus mode: four corners framing the text. Same path as mobile. */
const FocusIcon = () => (
  <Icon>
    <path d="M4 9V5h4M20 9V5h-4M4 15v4h4M20 15v4h-4" />
    <circle cx="12" cy="12" r="2" />
  </Icon>
);

const FoldIcon = () => (
  <Icon>
    <path d="M7 14l5 5 5-5M7 5l5 5 5-5" />
  </Icon>
);
const UnfoldIcon = () => (
  <Icon>
    <path d="M7 9l5-5 5 5M7 19l5-5 5 5" />
  </Icon>
);
const BellIcon = () => (
  <Icon>
    <path d="M18 8A6 6 0 0 0 6 8c0 7-3 9-3 9h18s-3-2-3-9" />
    <path d="M13.73 21a2 2 0 0 1-3.46 0" />
  </Icon>
);
const PluginsIcon = () => (
  <Icon>
    <rect x="3" y="3" width="8" height="8" rx="1.5" />
    <rect x="13" y="3" width="8" height="8" rx="1.5" />
    <rect x="3" y="13" width="8" height="8" rx="1.5" />
    <rect x="13" y="13" width="8" height="8" rx="1.5" />
  </Icon>
);
const GearIcon = () => (
  <Icon>
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09a1.65 1.65 0 0 0-1.08-1.51 1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  </Icon>
);
const SidebarIcon = () => (
  <Icon>
    <rect x="3" y="4" width="18" height="16" rx="2" />
    <path d="M9 4v16" />
  </Icon>
);

function ChromeToggle(props: {
  glyph: JSX.Element;
  active: boolean;
  label: string;
  title: string;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      aria-label={props.label}
      aria-pressed={props.active}
      title={props.title}
      onClick={props.onToggle}
      class={`flex h-8 w-8 items-center justify-center rounded-full text-[14px] leading-none transition-colors ${
        props.active
          ? "bg-(--color-outl-accent) text-(--color-outl-accent-fg)"
          : "text-(--color-outl-accent) hover:bg-(--color-outl-border)/40"
      }`}
    >
      <span aria-hidden="true">{props.glyph}</span>
    </button>
  );
}

/**
 * Plugin-contributed toolbar button.
 *
 * A `toolbar-button` capability plugin contributes a glyph + command; we
 * render it next to the native chrome toggles and run the command on
 * click (same path the plugin palette / keybinding take: surface the
 * plugin's output on the status line, re-render the page if it mutated,
 * and play any `ui-render` overlays). Momentary action, never a toggle —
 * `active` is always false.
 */
function PluginToolbarButton(props: { entry: ToolbarButtonEntry }) {
  const [busy, setBusy] = createSignal(false);

  async function run() {
    if (busy()) return;
    setBusy(true);
    try {
      const reply = await pluginRun(
        props.entry.plugin_id,
        props.entry.command_id,
        appState.page?.id ?? null,
      );
      for (const note of reply.notifications) setAppState("lastError", note);
      for (const err of reply.errors)
        setAppState("lastError", `plugin: ${err}`);
      if (reply.view) {
        setAppState({ page: reply.view.page });
        setOutline(reply.view.outline);
      }
      playPluginViews(reply.views);
    } catch (e) {
      setAppState("lastError", e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  const label = () =>
    props.entry.title ?? `${props.entry.plugin_id} · ${props.entry.command_id}`;

  return (
    <button
      type="button"
      aria-label={label()}
      title={label()}
      disabled={busy()}
      onClick={() => void run()}
      class="flex h-8 w-8 items-center justify-center rounded-full text-[14px] leading-none text-(--color-outl-accent) transition-colors hover:bg-(--color-outl-border)/40 disabled:opacity-50"
    >
      <span aria-hidden="true">{props.entry.icon}</span>
    </button>
  );
}

/**
 * Window chrome, laid out like the mobile top bar (`JournalChrome.tsx`).
 *
 * Top-right capsule, same icons in the same order as the phone: fold
 * all, unfold all, plugin toolbar buttons, plugins, the sync dot,
 * settings. Settings sits last, the sync dot right before it, so a user
 * moving between the two clients finds every control where their hand
 * expects it. Reminders sits after the toolbar buttons here; the phone
 * moved it to its bottom bar (#364), so it has no header twin.
 *
 * The only other control without a mobile twin is the sidebar toggle (the
 * phone has no sidebar). It gets its own small capsule in the top-left
 * corner, where the mobile back capsule sits, so it stays reachable
 * after the sidebar is hidden. The shortcuts-help toggle is gone from
 * the chrome: `?` still opens it, and the palette lists it.
 *
 * Every button flips the same store signal or runs the same handler as
 * its keyboard chord, so the button and the keyboard never disagree.
 */
export function ChromeToggleBar(props: { handlers?: ActionHandlers | null }) {
  const [toolbar, setToolbar] = createSignal<ToolbarButtonEntry[]>([]);
  const focus = focusMode();

  // Plugins load lazily on the host's first request once the workspace is
  // open, so a fetch at mount may return empty on a cold boot. Best-effort:
  // a failure leaves the capsule with only the native controls.
  onMount(() => {
    void (async () => {
      try {
        setToolbar(await pluginToolbar());
      } catch {
        setToolbar([]);
      }
    })();
  });

  const capsule =
    "outl-focus-hide fixed top-3 z-20 flex items-center gap-1 rounded-full border border-(--color-outl-border) bg-(--color-outl-bg-elev)/80 px-1.5 py-1 shadow-lg backdrop-blur-xl";

  return (
    <>
      <div
        class={`${capsule} left-3`}
        data-testid="chrome-left"
        data-outl-region="chrome"
        role="toolbar"
        aria-label="Window"
        onKeyDown={(e) => rovingKeydown(e, "horizontal")}
      >
        <ChromeToggle
          glyph={<SidebarIcon />}
          active={appState.sidebarOpen}
          label={appState.sidebarOpen ? "Hide sidebar" : "Show sidebar"}
          title="Toggle sidebar (⌘⇧E)"
          onToggle={() => setAppState("sidebarOpen", !appState.sidebarOpen)}
        />
      </div>
      <div
        class={`${capsule} right-3`}
        data-testid="chrome-right"
        data-outl-region="chrome"
        role="toolbar"
        aria-label="Page and app"
        onKeyDown={(e) => rovingKeydown(e, "horizontal")}
      >
        <ChromeToggle
          glyph={<FoldIcon />}
          active={false}
          label="Fold all"
          title="Fold all (zM)"
          onToggle={() => void props.handlers?.FoldAll?.()}
        />
        <ChromeToggle
          glyph={<UnfoldIcon />}
          active={false}
          label="Unfold all"
          title="Unfold all (zR)"
          onToggle={() => void props.handlers?.UnfoldAll?.()}
        />
        {/* Plugin-contributed toolbar buttons, one per `toolbar-button`
            contribution, in the same slot as on mobile. */}
        <For each={toolbar()}>
          {(entry) => <PluginToolbarButton entry={entry} />}
        </For>
        <ChromeToggle
          glyph={<BellIcon />}
          active={appState.remindersOpen}
          label="Reminders"
          title="Reminders (⌘⇧R)"
          onToggle={() => setAppState("remindersOpen", !appState.remindersOpen)}
        />
        <ChromeToggle
          glyph={<PluginsIcon />}
          active={appState.marketplaceOpen}
          label={appState.marketplaceOpen ? "Hide plugins" : "Plugins"}
          title="Plugins"
          onToggle={() =>
            setAppState("marketplaceOpen", !appState.marketplaceOpen)
          }
        />
        {/* Click syncs now, right-click opens Settings → Sync. */}
        <SyncIndicator />
        <ChromeToggle
          glyph={<FocusIcon />}
          active={focus.on()}
          label={focus.on() ? "Leave focus mode" : "Focus mode"}
          title="Focus mode (⌘⇧F)"
          onToggle={focus.toggle}
        />
        <ChromeToggle
          glyph={<GearIcon />}
          active={appState.settingsOpen}
          label="Settings"
          title="Settings (⌘,)"
          onToggle={() => setAppState("settingsOpen", !appState.settingsOpen)}
        />
      </div>
    </>
  );
}
