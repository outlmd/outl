import { For, Show, createSignal, onCleanup, onMount } from "solid-js";

import { getVersion } from "@tauri-apps/api/app";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { openUrl } from "@tauri-apps/plugin-opener";

import { getThemeConfig } from "@outl/shared/api/commands";
import { installTheme } from "@outl/shared/theme";

import { HELP_LINKS } from "../lib/links";

/**
 * The About window's content (`src-tauri/src/about.rs` opens it, the
 * `?view=about` branch in `index.tsx` mounts it).
 *
 * It answers the two questions the macOS standard panel could not:
 * *which build am I running* and *where do I go from here*. The links
 * are `lib/links.ts`, mirrored from the backend so this window and the
 * Help menu cannot name the same destination differently.
 *
 * Nothing here touches the workspace, which is why it can open during
 * the boot replay and on the first-run picker. `capabilities/about.json`
 * scopes the window to exactly that: read the version, close itself,
 * open an `https://` URL.
 */
export function AboutWindow() {
  // `null` while the read is in flight, and after a failure. The line
  // is simply absent then rather than showing a placeholder version,
  // which is the one number this window exists to state correctly.
  const [version, setVersion] = createSignal<string | null>(null);

  onMount(async () => {
    try {
      setVersion(await getVersion());
    } catch {
      // Outside a Tauri webview (a test, a browser). Leave it unset.
    }

    // The inline `<head>` script already painted the cached palette, so
    // this window opens in the user's theme either way; this upgrades it
    // to the live config and follows an OS appearance change while open.
    try {
      const cfg = await getThemeConfig();
      const unsubscribe = await installTheme({
        mode: cfg.mode,
        preset: cfg.preset,
        presetDark: cfg.preset_dark,
      });
      onCleanup(unsubscribe);
    } catch {
      // No backend, or a workspace that has not opened yet. The cached
      // palette and the brand defaults in `styles.css` still apply.
    }
  });

  function close() {
    try {
      void getCurrentWindow().close();
    } catch {
      // Not in a window (a test). Nothing to close.
    }
  }

  function onKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") close();
  }

  onMount(() => {
    window.addEventListener("keydown", onKeyDown);
    onCleanup(() => window.removeEventListener("keydown", onKeyDown));
  });

  return (
    <div
      class="flex h-full flex-col items-center bg-(--color-outl-bg) px-8 pt-9 pb-6 text-center select-none"
      data-testid="about-window"
    >
      {/* The brand mark (`assets/brand/mark-*.svg`), painted in the
          theme's accent so it reads on every preset instead of
          carrying a hex of its own (invariant 13). */}
      <svg
        viewBox="0 0 91 128"
        class="h-14 w-auto text-(--color-outl-accent)"
        fill="currentColor"
        role="img"
        aria-label="outl"
      >
        <path d="M0 21 A21 21 0 0 1 21 0 L70 0 A21 21 0 0 1 91 21 L91 90.2 L78 98 L78 66 L72 66 L72 104.2 L59 112 L59 66 L52 66 L52 117.2 L34 128 L34 66 L25 66 L25 91 Q25 94 22 94 L3 94 Q0 94 0 91 Z" />
      </svg>

      <h1 class="mt-4 text-2xl font-semibold tracking-tight">outl</h1>

      <Show when={version()}>
        {(v) => (
          <p class="mt-1 font-mono text-xs text-(--color-outl-fg-dim)">
            Version {v()}
          </p>
        )}
      </Show>

      {/* The README's first two sentences, which are the project's own
          description. Kept short enough to read in one pass. */}
      <p class="mt-4 text-sm leading-relaxed text-(--color-outl-fg-dim)">
        Local-first outliner. Markdown is the source of truth, and your
        devices sync straight to each other.
      </p>

      <nav class="mt-6 w-full space-y-1.5">
        <For each={HELP_LINKS}>
          {(link) => (
            <button
              type="button"
              class="w-full rounded-md border border-(--color-outl-fg)/15 bg-(--color-outl-fg)/5 px-3 py-2 text-sm transition-colors hover:border-(--color-outl-accent)/50 hover:bg-(--color-outl-fg)/10 focus:border-(--color-outl-accent)/50 focus:outline-none"
              onClick={() => {
                void openUrl(link.url).catch(() => {
                  // A browser that refuses to launch. The window stays
                  // open with the URL still on screen in the tooltip.
                });
              }}
              title={link.url}
            >
              {link.label}
            </button>
          )}
        </For>
      </nav>

      <p class="mt-auto pt-6 text-xs text-(--color-outl-fg-dimmer)">
        MIT licensed · Avelino and contributors
      </p>
    </div>
  );
}
