/**
 * The `:shortcode:` emoji trigger.
 *
 * Its own file because `index.ts` crossed the 600-line ratchet, and
 * this is the group that leaves cleanly: a `:` trigger shares no
 * walk-back, no closer rule and no delimiter with `[[`, `((`, `@`,
 * `#` or `/`. Re-exported from `index.ts`, so
 * `@outl/shared/autocomplete` stays the one import path — the same
 * answer `api/commands.ts` and `api/types.ts` gave to the same
 * ratchet (`outl-frontend-shared/CLAUDE.md`).
 */

import type { PairCompletion } from "./index";

/** Result of {@link detectEmojiContext}. */
export interface EmojiContext {
  /** Text typed after the opening `:`, used as fuzzy query. */
  query: string;
  /** Index of the opening `:`. */
  openIndex: number;
  /**
   * Index of the character right after the trigger span. Mirrors the
   * `replaceEnd` shape of {@link RefContext} so {@link applyEmojiSuggestion}
   * can splice deterministically. Emojis have no closer in this state
   * (the user hasn't typed the second `:`) so this equals the caret.
   */
  replaceEnd: number;
}

/**
 * Detect whether the caret sits inside an open `:shortcode` emoji
 * trigger. Returns the typed query (everything after the opening `:`)
 * + the span the caller will replace on commit. Returns `null` when
 * the caret isn't in that state.
 *
 * Trigger rules (pinned to gemoji syntax, identical across TUI / mobile
 * / desktop):
 *
 * - The opener `:` must be at the start of the buffer **or** preceded
 *   by whitespace. Mid-word `:` is ambiguous (`14:00`, `mailto:foo`,
 *   `key::value`) — bail.
 * - The first char after `:` must be `[a-z]`. A digit-only or
 *   symbol-only trigger (`:1`, `:+`) opens too eagerly and produces a
 *   popup on every prose `:`.
 * - The query body is `[a-z0-9_+-]*` — the same alphabet
 *   `outl_md::emoji::is_valid_shortcode_char` accepts. The first char
 *   outside that alphabet — including whitespace, `:` (closing —
 *   commit handled separately), or any URL char (`/`, `.`, `@`) —
 *   closes the trigger.
 * - Cap the scan at 32 chars left of the caret. A real shortcode is
 *   short; long misses (`:thisisaverylongthingnotanemoji`) shouldn't
 *   keep the popup open.
 *
 * The "first char after `:` must be `[a-z]`" rule keeps the popup
 * silent during URL typing — `https://` produces `:` followed by `/`
 * which fails immediately.
 */
export function detectEmojiContext(
  value: string,
  selection: number,
): EmojiContext | null {
  const CAP = 32;
  const lower = Math.max(0, selection - CAP);
  for (let i = selection - 1; i >= lower; i -= 1) {
    const ch = value[i] ?? "";
    if (ch === ":") {
      // Opener candidate. Validate the left side (start-of-buffer or
      // whitespace) and the right side (first char `[a-z]`).
      const prev = i === 0 ? "" : value[i - 1] ?? "";
      if (i !== 0 && !/\s/.test(prev)) return null;
      const first = value[i + 1] ?? "";
      if (!/[a-z]/.test(first)) return null;
      const query = value.slice(i + 1, selection);
      return { query, openIndex: i, replaceEnd: selection };
    }
    // Any non-shortcode char breaks the run.
    if (!/[a-z0-9_+\-]/.test(ch)) return null;
  }
  return null;
}

/**
 * Replace the in-flight `:query` trigger with the canonical
 * `:shortcode:` form (opening + closing `:`s included) and place the
 * caret immediately after the closing `:`. Mirrors {@link applySuggestion}
 * for refs / mentions so the call sites look the same.
 */
export function applyEmojiSuggestion(
  value: string,
  ctx: EmojiContext,
  shortcode: string,
): PairCompletion {
  const before = value.slice(0, ctx.openIndex);
  const after = value.slice(ctx.replaceEnd);
  const rebuilt = `:${shortcode}:`;
  return {
    value: before + rebuilt + after,
    caret: before.length + rebuilt.length,
  };
}

