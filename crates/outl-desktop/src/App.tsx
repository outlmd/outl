import { Show, createSignal, onCleanup, onMount } from "solid-js";

import { getThemeConfig } from "@outl/shared/api/commands";
import {
  bootMark,
  onPagePreview,
  PreviewOutline,
  waitWorkspaceReady,
} from "@outl/shared/boot";
import type { PageView } from "@outl/shared/api/types";
import { setDiagramsMode } from "@outl/shared/diagram";
import { installTheme } from "@outl/shared/theme";

import { currentWorkspace, getSettings, workspaceStats } from "./lib/api";
import { onWorkspaceReady } from "./lib/events";
import { setAppState } from "./lib/store";
import { AppShell } from "./components/AppShell";
import { Onboarding } from "./components/Onboarding";

/** `localStorage` key for the first-run flag (pure UI state — never an Op). */
const ONBOARDED_KEY = "outl.onboarded";

/**
 * Whether the user has completed (or skipped) first-run onboarding.
 *
 * This is a per-install UI flag, not workspace state, so it lives in
 * `localStorage` and deliberately does NOT go through the op log — it
 * must not converge across devices (each device onboards once). A
 * returning user whose workspace already opens at boot is treated as
 * onboarded so we never re-show the flow.
 */
function hasOnboarded(): boolean {
  try {
    return localStorage.getItem(ONBOARDED_KEY) === "1";
  } catch {
    return false;
  }
}

function markOnboarded() {
  try {
    localStorage.setItem(ONBOARDED_KEY, "1");
  } catch {
    // Private mode / disabled storage — onboarding just re-shows next
    // launch. Harmless, never blocks the app.
  }
}

/**
 * Root component.
 *
 * Three states, decided in order:
 *
 *   - workspace already open (returning user) → `<AppShell />`.
 *   - never onboarded → `<Onboarding />` (pick storage, optional pairing).
 *   - onboarded but no workspace open (e.g. picked folder removed) →
 *     fall back into `<Onboarding />` so the user re-picks.
 *
 * The split is intentional: the user can re-pick a workspace at runtime
 * (File → Switch Workspace…, see `lib/switch-workspace.ts`) without remounting the
 * whole app.
 */
function App() {
  const [ready, setReady] = createSignal(false);
  const [checked, setChecked] = createSignal(false);
  const [onboarded, setOnboarded] = createSignal(hasOnboarded());
  // Whether a workspace is already configured (`settings.last_workspace`).
  // Distinct from `ready` (the workspace has finished materializing): during
  // boot the config is known instantly but the background opener is still
  // replaying the op log, so `hasConfig && !ready` is the "opening" state — it
  // must render a loading screen, NOT the workspace picker. Showing the picker
  // there makes a returning user think their config was lost.
  const [hasConfig, setHasConfig] = createSignal(false);
  // Today's journal, read off disk before the workspace opened. Painted
  // in place of the spinner; replaced wholesale by `<AppShell />` the
  // moment the workspace is ready. Never editable — see
  // `@outl/shared/boot`'s `onPagePreview`.
  const [preview, setPreview] = createSignal<PageView | null>(null);

  function finishOnboarding() {
    markOnboarded();
    setOnboarded(true);
  }

  async function refresh() {
    try {
      const ws = await currentWorkspace();
      if (ws) {
        const stats = await workspaceStats();
        setAppState("workspace", stats);
        setReady(stats.ready);
        // A returning user whose workspace already opens has, by
        // definition, set up before — never re-show onboarding for them.
        if (stats.ready && !onboarded()) finishOnboarding();
      } else {
        setReady(false);
      }
    } finally {
      setChecked(true);
    }
  }

  /**
   * Pull `[theme]` (RFC 0022's light/dark pair) via `get_theme_config`
   * and install it as CSS custom properties on `<html>` + `<body>`,
   * following `prefers-color-scheme` when `mode = "auto"`. Falls back
   * to the built-in `outl-light` / `outl` pair when settings/config
   * aren't readable yet — the default `_root_` CSS already paints the
   * brand background, so a delayed palette load just upgrades the
   * colors. Returns the `installTheme` unsubscribe so the caller can
   * tear down the media-query listener.
   */
  async function hydrateTheme(): Promise<() => void> {
    try {
      const s = await getSettings();
      // A configured workspace is known from `settings.last_workspace` on disk
      // IMMEDIATELY — long before the background opener publishes the live
      // `storage_root` (that only happens after the op-log replay). This is the
      // signal the gate uses to show "opening…" instead of the picker, so read
      // it here, in the earliest boot step. `null` (or unreadable settings on a
      // true first run) leaves it false → the picker, correctly.
      setHasConfig(s.last_workspace != null);
      // Backlinks direction is a display preference read from the same
      // settings file (issue #142); hydrate it here so the first render
      // already orders correctly.
      if (s.backlinks_order === "oldest" || s.backlinks_order === "newest") {
        setAppState("backlinksOrder", s.backlinks_order);
      }
      setDiagramsMode(s.diagrams);
      const cfg = await getThemeConfig();
      const unsubscribe = await installTheme({
        mode: cfg.mode,
        preset: cfg.preset,
        presetDark: cfg.preset_dark,
      });
      // The authoritative palette is on `<html>`. The inline `<head>`
      // script usually painted the same colours already; the gap between
      // the two marks is exactly what that cache hides from the user.
      bootMark("theme_applied");
      return unsubscribe;
    } catch {
      // First boot before the workspace lock is created — install the
      // built-in default pair explicitly so we still ship the brand
      // colors. A total backend failure keeps the static boot frame.
      try {
        return await installTheme({
          mode: "auto",
          preset: "outl-light",
          presetDark: "outl",
        });
      } catch {
        // No backend at all (shouldn't happen) — keep the static
        // boot frame.
        return () => {};
      }
    }
  }

  onMount(async () => {
    // Subscribed FIRST, before any await. The backend emits this
    // before it opens the op log, so a listener armed after the theme
    // round-trip would miss the event on a fast workspace — which is
    // the one case where it would have been free.
    // No `ready()` guard: the render is the only reader of `preview()`
    // and it lives in the not-ready branch, so a late preview is simply
    // never shown. Same shape mobile uses.
    const unlistenPreview = await onPagePreview(setPreview);
    onCleanup(() => unlistenPreview());

    // Hydrate the theme BEFORE the rest of the boot routine so the
    // first painted frame already uses the user's chosen palette
    // — running it in parallel left a perceptible flash where
    // `refresh()` rendered with the static defaults from styles.css.
    const unsubscribeTheme = await hydrateTheme();
    onCleanup(unsubscribeTheme);
    await refresh();

    // Background opener (boot-time) emits `workspace-ready` when it
    // finishes opening `settings.last_workspace`. The picker flow
    // also emits the same event from `set_workspace`.
    const unlisten = await onWorkspaceReady(async () => {
      await refresh();
    });
    onCleanup(() => unlisten());

    // While the background opener is still in flight, **wait** for it.
    //
    // This was a 500 ms `setInterval` — a fallback for a missed
    // `workspace-ready` event, which is a real failure mode (the event
    // can be emitted before this listener exists, or lost to a webview
    // reload mid-boot) and therefore not something to delete. What is
    // gone is the *interval*: a workspace that became ready 10 ms into a
    // tick waited out the remaining 490 ms, on every launch, and each
    // tick was a full `workspace_stats` round-trip.
    //
    // `wait_workspace_ready` is latched in Rust, so it answers the
    // moment the opener publishes and answers immediately if it already
    // has. `false` is "not yet" and loops; a rejection means the backend
    // could not take the wait, which is where the old `refresh()` on a
    // timer is still the right answer.
    if (!ready()) {
      let cancelled = false;
      onCleanup(() => {
        cancelled = true;
      });
      void (async () => {
        // The same ~20s budget the 40 × 500 ms loop had, spent waiting.
        for (let i = 0; i < 10 && !cancelled && !ready(); i += 1) {
          try {
            await waitWorkspaceReady(2_000);
          } catch {
            // An older backend, or a refused wait. Fall back to asking.
          }
          if (cancelled) return;
          await refresh();
        }
      })();
    }
  });

  return (
    <div class="h-full">
      <Show
        when={checked()}
        fallback={<div class="p-8 opacity-50">Loading…</div>}
      >
        {/* Ready + onboarded → the app. Not ready yet, but a workspace IS
            configured → the background opener is still replaying; show a
            loading screen, never the picker (that reads as "my config was
            lost"). Only run onboarding when there's truly no config (first
            run, or the picked folder was removed). */}
        <Show
          when={ready() && onboarded()}
          fallback={
            <Show
              when={hasConfig()}
              fallback={
                <Onboarding
                  onWorkspacePicked={refresh}
                  onFinish={finishOnboarding}
                />
              }
            >
              {/* Real content beats a spinner. The preview is today's
                  journal read straight off disk, so it lands in ~10ms
                  whatever the workspace holds, while the op log is
                  still being replayed behind it. The spinner stays as
                  the fallback for a launch with nothing to preview (a
                  fresh workspace, or a day with no journal yet), and
                  the small line at the top is what stops the read-only
                  state from reading as a frozen app. */}
              <Show
                when={preview()}
                fallback={
                  <div class="flex h-full items-center justify-center">
                    <div class="flex items-center gap-3 opacity-60">
                      <div class="h-4 w-4 animate-spin rounded-full border-2 border-(--color-outl-fg)/30 border-t-(--color-outl-fg)/70" />
                      <span class="text-sm">Opening your workspace…</span>
                    </div>
                  </div>
                }
              >
                {(p) => (
                  <div class="h-full overflow-y-auto px-10 py-8">
                    <div class="mx-auto max-w-3xl">
                      <div class="mb-6 flex items-center gap-2 text-xs opacity-50">
                        <div class="h-3 w-3 animate-spin rounded-full border border-(--color-outl-fg)/30 border-t-(--color-outl-fg)/70" />
                        <span>Opening your workspace…</span>
                      </div>
                      <h1 class="mb-4 text-2xl font-semibold text-(--color-outl-fg)/90">
                        {p().page.title}
                      </h1>
                      <PreviewOutline outline={p().outline} />
                    </div>
                  </div>
                )}
              </Show>
            </Show>
          }
        >
          <AppShell />
        </Show>
      </Show>
    </div>
  );
}

export default App;
