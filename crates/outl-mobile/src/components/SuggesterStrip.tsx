import { For, type JSX, Show } from "solid-js";
import { nativeSuggesterState } from "../lib/native-suggester";

/**
 * Web-rendered ref/emoji suggestion strip — the cross-platform successor
 * to the iOS-only native `OutlSuggestView`. Reads the same reactive
 * suggester state the editor publishes (`setNativeSuggesterState`) and, on
 * tap, calls back through the identical `window.__outlSuggesterPicked`
 * bridge the native strip uses, so the accept path is single-sourced.
 *
 * `kind: "emoji"` chips carry the glyph in `title` (shown as the label)
 * and the shortcode in `slug` (handed back on tap); every other kind shows
 * `title` and returns `slug`.
 */
interface PickerBridge extends Window {
  __outlSuggesterPicked?: (slug: string, kind: string) => void;
}

export function SuggesterStrip(): JSX.Element {
  // The chips to render, or null when there's nothing to suggest.
  // Deriving it once keeps the template free of union re-narrowing.
  const items = () => {
    const s = nativeSuggesterState();
    return s?.action === "show" && s.items.length > 0 ? s.items : null;
  };

  function pick(slug: string, kind: string) {
    (window as PickerBridge).__outlSuggesterPicked?.(slug, kind);
  }

  return (
    <Show when={items()}>
      {(list) => (
        <div class="outl-kb-capsule pointer-events-auto flex max-w-full items-center gap-0.5 overflow-x-auto rounded-full bg-(--color-outl-bg-elev) px-2 py-1 shadow-[var(--shadow-capsule)] [scrollbar-width:none] dark:shadow-[var(--shadow-capsule-dark)] [&::-webkit-scrollbar]:hidden">
          <For each={list()}>
            {(item) => (
              <button
                type="button"
                // Same focus-preservation trick as the toolbar: without
                // it the tap blurs the textarea and drops the keyboard.
                onPointerDown={(e) => e.preventDefault()}
                onClick={() => pick(item.slug, item.kind)}
                class="shrink-0 rounded-full px-3 py-1 text-[15px] text-(--color-outl-fg) active:bg-black/5 dark:active:bg-white/10"
              >
                {item.title}
              </button>
            )}
          </For>
        </div>
      )}
    </Show>
  );
}
