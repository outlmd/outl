/**
 * The `/command` slash trigger.
 *
 * Its own file for the reason `emoji.ts` is: `index.ts` crossed the
 * 600-line ratchet, and this is a group that leaves cleanly — `/` has
 * no closer, no bracket pair and no walk-back shared with `[[`, `((`,
 * `@` or `#`. Re-exported from `index.ts`, so
 * `@outl/shared/autocomplete` stays the one import path
 * (`outl-frontend-shared/CLAUDE.md`).
 *
 * The rule here mirrors `outl_tui`'s `detect_trigger`, which is the
 * owner of where a slash command starts and ends. Keep the two in step
 * in the same commit: a GUI that fires on a narrower rule than the
 * terminal is a menu that silently does not open.
 */

import type { PairCompletion } from "./index";

/** An active `/command` slash trigger inside a block's textarea. */
export interface SlashContext {
  /** Text after the `/`, up to the caret — the command filter. */
  query: string;
  /** Index of the `/` that opened the trigger. */
  openIndex: number;
  /** Caret position (no closer — equals `selection`). */
  replaceEnd: number;
}

/**
 * May this character sit inside a `/command` name?
 *
 * One constant because two callers ask: {@link detectSlashContext}
 * walking back to the `/`, and {@link applySlashContext} walking
 * forward to the end of the token it removes. Two copies of the
 * alphabet is how an accept erases a different span than the popup
 * matched on.
 */
const SLASH_NAME_CHAR = /[A-Za-z0-9_-]/;

/**
 * Detect a `/command` slash trigger, mirroring the TUI's slash menu but
 * inline inside a block.
 *
 * **Word-initial**, exactly like `outl_tui`'s `detect_trigger`: the `/`
 * sits at the start of the block or straight after whitespace, which is
 * what keeps `http://host`, `a/b` and `see docs/api` literal while
 * `nota /` still opens the menu. The run between the `/` and the caret
 * must be command-name chars; a space or any other character closes the
 * trigger (Notion behaviour).
 *
 * This used to be block-initial only (`value[0] !== "/"`), which is
 * narrower than the TUI it claims to mirror — so the same keystrokes
 * that opened the menu in the terminal did nothing in a GUI block that
 * already had text.
 *
 * Returns `null` when no trigger is active.
 */
export function detectSlashContext(
  value: string,
  selection: number,
): SlashContext | null {
  if (selection < 1) return null;
  // Walk back over the command name to the `/` that opened it. Starting
  // from the caret rather than from index 0 is what makes `/a/b` close:
  // the live trigger is the *nearest* slash, and that one is preceded by
  // a letter.
  let open = selection;
  while (open > 0 && SLASH_NAME_CHAR.test(value[open - 1])) open -= 1;
  open -= 1;
  if (open < 0 || value[open] !== "/") return null;
  if (open > 0 && !/\s/.test(value[open - 1])) return null;
  return {
    query: value.slice(open + 1, selection),
    openIndex: open,
    replaceEnd: selection,
  };
}

/**
 * Remove the `/query` token when a slash command is accepted, keeping any
 * text after the token. The block that hosted the command is left with its
 * non-command remainder (usually empty) and the caret at the open index.
 */
export function applySlashContext(
  value: string,
  ctx: SlashContext,
): PairCompletion {
  let end = ctx.openIndex + 1;
  while (end < value.length && SLASH_NAME_CHAR.test(value[end])) end++;
  const next = value.slice(0, ctx.openIndex) + value.slice(end);
  return { value: next, caret: ctx.openIndex };
}
