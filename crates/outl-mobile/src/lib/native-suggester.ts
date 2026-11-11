/**
 * Bridge between the Solid editor and the native UIKit ref suggester
 * (the chip strip docked above the formatting toolbar; see
 * `main.mm` → `OutlSuggestView` / `OutlAccessoryContainer`).
 *
 * The contract is **state-based, not message-based**: JS owns a
 * single global `window.__outlSuggesterState` that UIKit polls every
 * 150ms while the keyboard is up. We tried `WKScriptMessageHandler`
 * first but the Tauri-managed `WKUserContentController` did not
 * surface late-added handlers reliably — silent loss was worse than
 * a tiny polling cost.
 *
 * - JS calls [`setNativeSuggesterState`] when the caret enters or
 *   leaves an open `[[…]]` reference.
 * - UIKit reads it on the next poll tick and updates the chip strip.
 * - Tap on a chip calls back via `window.__outlSuggesterPicked(slug,
 *   kind)`.
 *
 * Desktop dev (`bun run dev` without a webview) just sets the global
 * — UIKit isn't there to read it, but Solid keeps the visual editor
 * consistent.
 */
import { createSignal } from "solid-js";
import type { EmojiHit } from "@outl/shared/api/commands";
import type { PageMeta } from "@outl/shared/api/types";
import { refReplacement } from "@outl/shared/autocomplete";
import type { RefContext } from "@outl/shared/autocomplete";

interface WindowWithBridge extends Window {
  __outlSuggesterState?: SuggesterMessage | null;
  __outlSuggesterPicked?: (slug: string, kind: string) => void;
}

/** Shape UIKit reads off `window.__outlSuggesterState`. Exported for
 *  unit tests so we can lock the wire shape down.
 *
 *  `kind: "emoji"` chips carry the shortcode in `slug` and the unicode
 *  glyph in `title` — UIKit renders the glyph as the chip label, and
 *  the picked callback receives the shortcode for the insertion. */
export interface SuggesterShowMessage {
  action: "show";
  items: Array<{
    slug: string;
    title: string;
    kind: "page" | "journal" | "emoji";
  }>;
}

export interface SuggesterHideMessage {
  action: "hide";
}

export type SuggesterMessage = SuggesterShowMessage | SuggesterHideMessage;

export function buildShowMessage(
  items: PageMeta[],
  ctx?: Pick<RefContext, "kind">,
): SuggesterShowMessage {
  return {
    action: "show",
    items: items.map((p) => ({
      // `slug` is the value handed back to JS on tap and spliced into
      // the document. The rule (journals → ISO slug, tags → slug
      // always, everything else / mentions → title; issues #88 and
      // #15) lives in the shared `refReplacement` — the same function
      // the desktop's ref popup calls, so the two accept paths can't
      // drift.
      //
      // It takes the ref *context*, not one boolean per kind: a flag
      // pair is what let this path ship passing `{ mention }` alone,
      // so a `#` pick inserted the title and `#Deep Work` tagged
      // `#Deep` with "Work" left as prose. One `kind` field cannot
      // say "mention and tag", and cannot say neither by omission.
      slug: refReplacement(p, {
        mention: ctx?.kind === "mention",
        tag: ctx?.kind === "tag",
      }),
      // Journal pages render under a human-readable title server-side
      // ("Thursday, May 28, 2026"). The mobile UI is anchored on ISO
      // slugs (`2026-05-28`), so show the slug in the chip strip
      // instead — matches the journal header format too.
      title: p.kind === "journal" ? p.slug : p.title,
      kind: p.kind,
    })),
  };
}

/**
 * Build a `SuggesterShowMessage` for the `:shortcode:` emoji
 * autocomplete. The picked slug is the shortcode (`"tada"`); UIKit
 * shows the glyph (`"🎉"`) as the chip label, which makes the strip
 * scannable without the user reading shortcode names.
 */
export function buildEmojiShowMessage(hits: EmojiHit[]): SuggesterShowMessage {
  return {
    action: "show",
    items: hits.map((h) => ({
      slug: h.shortcode,
      title: h.glyph,
      kind: "emoji" as const,
    })),
  };
}

export const HIDE_MESSAGE: SuggesterHideMessage = { action: "hide" };

/**
 * Reactive mirror of the published suggester state, for the **web** chip
 * strip (`<SuggesterStrip />`, Android). The iOS native strip polls the
 * `window` global instead; a single setter (`setNativeSuggesterState`)
 * feeds both so the two rendering paths can never disagree.
 */
const [suggesterState, setSuggesterSignal] =
  createSignal<SuggesterMessage | null>(null);

/** Accessor for the web chip strip to render reactively. */
export const nativeSuggesterState = suggesterState;

/**
 * Publish the next state. The iOS native strip picks it up off
 * `window.__outlSuggesterState` on its next poll tick; the web strip
 * reacts to the signal. Pass `null` to clear (suggester becomes inert).
 */
export function setNativeSuggesterState(state: SuggesterMessage | null): void {
  (window as WindowWithBridge).__outlSuggesterState = state;
  setSuggesterSignal(state);
}

/** Read the current published state. Mostly for tests. */
export function getNativeSuggesterState(): SuggesterMessage | null {
  return (window as WindowWithBridge).__outlSuggesterState ?? null;
}

/** Register a JS-side handler the native code calls back when the
 *  user taps a chip. Returns a cleanup function. */
export function registerPickedCallback(
  cb: (slug: string, kind: string) => void,
): () => void {
  const win = window as WindowWithBridge;
  win.__outlSuggesterPicked = cb;
  return () => {
    if (win.__outlSuggesterPicked === cb) {
      delete win.__outlSuggesterPicked;
    }
  };
}
