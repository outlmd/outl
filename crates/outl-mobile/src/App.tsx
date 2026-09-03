import { Show, createSignal, onCleanup, onMount } from "solid-js";

import { getThemeConfig } from "@outl/shared/api/commands";
import { installTheme } from "@outl/shared/theme";

import { Journal } from "./components/Journal";
import { Onboarding } from "./components/Onboarding";

/** `localStorage` key for the first-run flag (pure UI state — never an Op). */
const ONBOARDED_KEY = "outl.onboarded";

/**
 * Whether the user has completed (or skipped) first-run onboarding.
 *
 * This is a per-install UI flag, not workspace state, so it lives in
 * `localStorage` and deliberately does NOT go through the op log — it
 * must not converge across devices (each device onboards once).
 *
 * Mobile has no "is a workspace chosen?" backend gate (a fresh install
 * always resolves *a* root — the local default), so the first-run flag
 * is the only signal that distinguishes a brand-new install from a
 * returning one.
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
    // Private mode / disabled storage — onboarding re-shows next launch.
    // Harmless, never blocks the app.
  }
}

function App() {
  const [onboarded, setOnboarded] = createSignal(hasOnboarded());

  function finishOnboarding() {
    markOnboarded();
    setOnboarded(true);
  }

  onMount(async () => {
    // Hydrate the theme BEFORE the rest of the boot routine so the
    // first painted frame already uses the real palette instead of
    // the static `styles.css` boot values (mirrors the desktop's
    // `App.tsx` — see its `hydrateTheme` comment).
    //
    // `get_theme_config` reads `outl_config::ThemeCfg` on the Rust
    // side and resolves `preset_dark` for us — never empty, even for
    // a config that only ever set `preset` (RFC 0022's
    // backwards-compatibility guarantee). If the read fails (no
    // config file yet, backend not up), fall back to the brand pair
    // `outl-light` / `outl` — the two presets must actually differ,
    // or `mode: "auto"` has nothing to switch between and OS-light
    // users get the dark palette every time
    // (`outl-theme::presets::outl()` is dark).
    let cfg: { mode: "light" | "dark" | "auto"; preset: string; presetDark: string };
    try {
      const themeConfig = await getThemeConfig();
      cfg = {
        mode: themeConfig.mode,
        preset: themeConfig.preset,
        presetDark: themeConfig.preset_dark,
      };
    } catch {
      cfg = { mode: "auto", preset: "outl-light", presetDark: "outl" };
    }
    const unsubscribe = await installTheme(cfg);
    onCleanup(unsubscribe);
  });

  return (
    <div class="flex h-full flex-col bg-(--color-outl-bg)">
      <Show
        when={onboarded()}
        fallback={<Onboarding onFinish={finishOnboarding} />}
      >
        <Journal />
      </Show>
    </div>
  );
}

export default App;
