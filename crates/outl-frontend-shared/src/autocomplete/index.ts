/**
 * Inline auto-complete helpers for the block editor.
 *
 * Two related concerns live here:
 *
 * 1. **Pair completion**: when the user types the second `[` to form
 *    `[[`, or the second `(` to form `((`, we proactively insert the
 *    matching `]]` / `))` to the right of the caret and keep the
 *    caret in the middle. Same trick most editors use for quotes and
 *    brackets — saves the user typing the closing pair when their
 *    intent is unambiguous.
 *
 * 2. **Context detection**: while the caret sits *inside* an open
 *    `[[…]]` or `((…))` we can offer a suggestion list (pages for
 *    refs, blocks for block refs). [`detectRefContext`] reads the
 *    text around the caret and returns the kind + the current query
 *    if we're in that state.
 *
 * Both helpers are pure functions over the textarea string + caret
 * index, so they're trivial to unit-test and they make no assumption
 * about the DOM.
 */

/** Result of {@link autoClosePair}. */
// Re-exported so `@outl/shared/autocomplete` stays the single import
// path after the split (see `emoji.ts`'s header).
export {
  applyEmojiSuggestion,
  detectEmojiContext,
  type EmojiContext,
} from "./emoji";

export interface PairCompletion {
  /** Full textarea value after the completion. */
  value: string;
  /** Where to place the caret. */
  caret: number;
}

/**
 * Inspect `value` after the user has already typed the new
 * character that produced `selection`. If the two characters
 * immediately to the *left* of the caret are `[[` or `((`, append the
 * matching `]]` / `))` to the right of the caret so the user lands
 * inside an empty ref. Returns `null` when nothing should change.
 *
 * We deliberately don't try to handle the case where the user typed
 * the opening pair *next to* an existing closer (`[[]]`-already-
 * present-on-the-right) — pair completion only fires when the right
 * side starts with something other than the matching closer, so we
 * never end up with `]]]]`.
 *
 * **Never run this over a caret an IME is composing at** (issue #399).
 * Browsers report that caret at the *start* of the marked run, so `[[`
 * plus the marked text reads as an unclosed ref, and the caller's write
 * then commits the run as plain text. Callers on `input` skip
 * `isComposing` / `insertCompositionText` events; the `[[` flow does
 * not depend on them, since {@link autoPairBracket} already closed the
 * pair in `beforeinput`.
 */
export function autoClosePair(
  value: string,
  selection: number,
): PairCompletion | null {
  if (selection < 2) return null;
  const left = value.slice(selection - 2, selection);
  const right = value.slice(selection, selection + 2);
  if (left === "[[" && right !== "]]") {
    return {
      value: value.slice(0, selection) + "]]" + value.slice(selection),
      caret: selection,
    };
  }
  if (left === "((" && right !== "))") {
    return {
      value: value.slice(0, selection) + "))" + value.slice(selection),
      caret: selection,
    };
  }
  return null;
}

/** Single-character bracket pairs handled by {@link autoPairBracket}
 *  and {@link autoDeletePair}. Mirrors the TUI's Insert-mode set
 *  (`outl-tui/src/input/insert.rs`). */
const BRACKET_PAIRS: Record<string, string> = {
  "(": ")",
  "[": "]",
  "{": "}",
};

const BRACKET_CLOSERS = new Set(Object.values(BRACKET_PAIRS));

/**
 * Auto-pair a single bracket keystroke, mirroring the TUI's Insert
 * mode: typing `(`, `[` or `{` inserts the matching closer and
 * leaves the caret between the two; typing `)`, `]` or `}` when that
 * exact character already sits to the right of the caret steps over
 * it instead of doubling (the closer was almost certainly
 * auto-inserted, so the keystroke's intent is "move past it").
 *
 * `typed` is the character the user is about to insert — callers
 * intercept it in `beforeinput` (soft keyboards don't emit reliable
 * `keydown` characters) and apply the returned value themselves.
 * Returns `null` when `typed` isn't a bracket character; let the
 * default insertion through.
 *
 * Pair insertion is deliberately unconditional, like the TUI's
 * `insert_pair`: typing the second `[` next to an auto-inserted `]`
 * yields `[[|]]` directly, which is exactly the shape the `[[` ref
 * flow wants — and {@link autoClosePair} stays a no-op on it, so the
 * two layers never double a closer.
 */
export function autoPairBracket(
  value: string,
  selection: number,
  typed: string,
): PairCompletion | null {
  const close = BRACKET_PAIRS[typed];
  if (close) {
    return {
      value: value.slice(0, selection) + typed + close + value.slice(selection),
      caret: selection + 1,
    };
  }
  if (BRACKET_CLOSERS.has(typed) && value[selection] === typed) {
    return { value, caret: selection + 1 };
  }
  return null;
}

/**
 * Inverse of {@link autoClosePair} / {@link autoPairBracket}: when
 * the caret sits in the empty middle of a pair and the user presses
 * Backspace, delete the whole pair in one shot so they don't have to
 * mash backspace to undo an aborted ref or bracket. Returns `null`
 * when the caret isn't between an empty pair — let the default
 * backspace through.
 *
 * Doubled ref pairs (`[[]]`, `(())`) are checked first and collapse
 * all four characters; the single-character pairs (`()`, `[]`, `{}`)
 * collapse two. The moment the user has typed something in the
 * middle (`[[ave]]`, `(x)`), backspace goes back to deleting one
 * char at a time so they can fix typos.
 */
export function autoDeletePair(
  value: string,
  selection: number,
): PairCompletion | null {
  if (selection >= 2) {
    const left = value.slice(selection - 2, selection);
    const right = value.slice(selection, selection + 2);
    if (
      (left === "[[" && right === "]]") ||
      (left === "((" && right === "))")
    ) {
      return {
        value: value.slice(0, selection - 2) + value.slice(selection + 2),
        caret: selection - 2,
      };
    }
  }
  if (selection >= 1) {
    const close = BRACKET_PAIRS[value[selection - 1] ?? ""];
    if (close !== undefined && value[selection] === close) {
      return {
        value: value.slice(0, selection - 1) + value.slice(selection + 1),
        caret: selection - 1,
      };
    }
  }
  return null;
}

/**
 * Insert `open + close` at `selection`, returning the new value and
 * the caret position *between* them. Used by toolbar buttons that
 * pre-seed a delimiter pair (`[[`, `((`).
 */
export function insertPair(
  value: string,
  selection: number,
  open: string,
  close: string,
): PairCompletion {
  return {
    value: value.slice(0, selection) + open + close + value.slice(selection),
    caret: selection + open.length,
  };
}

/**
 * Insert `snippet` at `selection`. Returns the new value and the
 * caret moved past the inserted text.
 */
export function insertText(
  value: string,
  selection: number,
  snippet: string,
): PairCompletion {
  return {
    value: value.slice(0, selection) + snippet + value.slice(selection),
    caret: selection + snippet.length,
  };
}

/** Result of {@link detectRefContext}. */
export interface RefContext {
  /**
   * What kind of reference the caret is sitting inside.
   *
   * - `"page"` — caret inside an open `[[…]]` page ref.
   * - `"block"` — caret inside an open `((…))` block ref.
   * - `"mention"` — caret right after a word-initial `@`, possibly
   *   followed by a query that may include spaces (composite names
   *   like `@Thiago Avelino`). Mentions have no closing delimiter;
   *   `replaceEnd` equals the caret. On accept the popup wraps the
   *   chosen title with `[[@…]]`.
   * - `"tag"` — caret inside a word-initial `#tag`. Like a mention it
   *   has no closer, and unlike one it stops at whitespace: a tag is
   *   one word. On accept the popup writes `#slug` with no brackets.
   */
  kind: "page" | "block" | "mention" | "tag";
  /** The text between the opener and the caret — what to filter by. */
  query: string;
  /** Index of the first delimiter character (the leftmost `[` / `(` / `@`). */
  openIndex: number;
  /** Index of the character right after the closer, or the caret if
   *  no closer was found before the caret. Used to replace the whole
   *  ref when a suggestion is accepted. */
  replaceEnd: number;
}

/**
 * Detect whether the caret is inside an open `[[…]]` or `((…))`
 * reference. When it is, return the kind + the text between the
 * opener and the caret (so the caller can use it as a fuzzy query)
 * and the span the caller should replace when the user accepts a
 * suggestion.
 *
 * The rule is: walk left from the caret looking for `[[` or `((`
 * *without* hitting the matching closer first. If we find a `]]` or
 * `))` first, the caret isn't inside that ref. We also stop on
 * newlines so a stray `[[` on a previous line doesn't capture a
 * later caret.
 *
 * **Cross-runtime mirror.** The TUI runs the same walk-back in Rust
 * at `outl_tui::actions::overlay::detect_trigger`, which also owns the
 * `/` slash trigger this file mirrors in `detectSlashContext`.
 * Frontend clients keep a local TS copy (here in `@outl/shared`) so
 * the autocomplete popup never pays a Tauri round-trip per keystroke.
 * If you extend either side (a new opener, a different closing rule),
 * update both in the same commit.
 *
 * The `#` trigger is the sharpest instance of that: the TUI gates its
 * tag query on the tokenizer's alphabet, so `isTagChar` below reads
 * from the same source rather than inventing a second rule. A GUI that
 * offered a popup for `#foo.bar` would be offering to complete a tag
 * the tokenizer ends at `foo`.
 */
/**
 * Is the character at `at` inside a fenced code block?
 *
 * Counts ``` runs before it: an odd number means the caret sits inside
 * an open fence. A shell comment (`#!/bin/sh`) is a word-initial `#`
 * with no space after it, so nothing else in the tag walk-back can
 * tell it from a tag.
 */
function insideFence(value: string, at: number): boolean {
  return (value.slice(0, at).split("```").length - 1) % 2 === 1;
}

/**
 * May this character sit inside a `#tag` name?
 *
 * Mirrors `outl_md::reference::try_tag`, which is the single owner of
 * where a tag ends — it walks `is_alphanumeric() || '-' || '_' || '/'`
 * and stops. The TUI's `detect_trigger` gates on the same set.
 *
 * This is what ends the walk below, so it subsumes the two rules that
 * used to be spelled out there: whitespace is not a tag char (a `# `
 * heading and `#one two` both stop), and neither is a bracket (`#fo[o`
 * stops before the `[`).
 */
function isTagChar(ch: string): boolean {
  return (
    /[\p{Alphabetic}\p{N}]/u.test(ch) || ch === "-" || ch === "_" || ch === "/"
  );
}

/**
 * The `[[` / `((` ref the caret sits inside, or `null`.
 *
 * One owner for "is the caret inside a bracketed ref", because three
 * callers ask it: the page/block context below, and each of the two
 * openerless pre-passes (`#`, `@`) deciding whether to stand down.
 *
 * A bracketed ref outranks both of those. `#` inside `[[Issue #42` is
 * part of a page title, and a tag pre-pass that claimed it spliced
 * from the `#` instead of the `[[` — turning `[[Issue #42` into
 * `[[Issue #deep-work]]` and making any page whose title holds a `#`
 * after a space impossible to complete.
 */
function openRefAt(
  value: string,
  selection: number,
): { kind: "page" | "block"; openIndex: number } | null {
  for (let i = selection - 1; i >= 1; i -= 1) {
    const pair = value.slice(i - 1, i + 1);
    if (pair === "]]" || pair === "))") return null;
    if (value[i] === "\n") return null;
    if (pair === "[[") return { kind: "page", openIndex: i - 1 };
    if (pair === "((") return { kind: "block", openIndex: i - 1 };
  }
  return null;
}

export function detectRefContext(
  value: string,
  selection: number,
): RefContext | null {
  // Order is the whole design here, most-constrained trigger first.
  //
  // 1. A bracketed ref has an explicit opener *and* closer, so it wins
  //    outright: a `#` or `@` typed inside one belongs to the ref.
  // 2. `#` next, because a tag is one word — the walk ends at the
  //    first non-tag char, so it can only fire on the caret's own
  //    word, which makes it the closest trigger whenever it matches.
  // 3. `@` last, because mentions allow spaces (`@Thiago Avelino`), so
  //    its walk reaches across prose. Running it first let a stray `@`
  //    anywhere in the preceding 64 chars swallow the `#` the user had
  //    just typed, and the tag popup silently never opened.
  const open = openRefAt(value, selection);
  if (open) {
    const query = value.slice(open.openIndex + 2, selection);
    const closer = open.kind === "page" ? "]]" : "))";
    return {
      kind: open.kind,
      query,
      openIndex: open.openIndex,
      replaceEnd: findCloser(value, selection, closer),
    };
  }

  // Tag pre-pass: walk back looking for a word-initial `#`.
  //
  // A mid-word `#` is not a tag (`a#b`, a CSS colour, a URL fragment),
  // the same trap `@` has in an email address. The fence check is
  // separate because the alphabet cannot catch it: a shell comment is
  // `#!/bin/sh`, a word-initial `#` whose `!` ends the name.
  const TAG_CAP = 64;
  for (let j = selection - 1; j >= Math.max(0, selection - TAG_CAP); j -= 1) {
    const ch = value[j];
    if (ch === undefined) break;
    if (ch === "#") {
      const prev = j === 0 ? "" : value[j - 1];
      if (j !== 0 && !/\s/.test(prev)) break;
      if (insideFence(value, j)) break;
      return {
        kind: "tag",
        query: value.slice(j + 1, selection),
        openIndex: j,
        replaceEnd: selection, // tags have no closer
      };
    }
    if (!isTagChar(ch)) break;
  }

  // Mention pre-pass: walk back looking for a word-initial `@`.
  // Composite names carry spaces, so this one does not stop at
  // whitespace; it is capped at 64 chars so a stray `@` followed by
  // prose does not scan the whole buffer.
  const MENTION_CAP = 64;
  for (
    let i = selection - 1;
    i >= Math.max(0, selection - MENTION_CAP);
    i -= 1
  ) {
    const ch = value[i];
    if (ch === "\n" || ch === "[" || ch === "]" || ch === "(" || ch === ")") {
      break;
    }
    if (ch === "@") {
      const prev = i === 0 ? "" : value[i - 1];
      // Mid-word `@` (email, social handle) is not a mention.
      if (i !== 0 && !/\s/.test(prev)) break;
      return {
        kind: "mention",
        query: value.slice(i + 1, selection),
        openIndex: i,
        replaceEnd: selection, // mentions have no closer
      };
    }
  }

  return null;
}

/**
 * Apply a chosen suggestion to a textarea string. Replaces the span
 * between `openIndex` (the `[[` or `((`) and `replaceEnd` (just past
 * the closer or at the caret) with `delim_open + replacement +
 * delim_close`. Returns the new value plus the caret position right
 * after the closer.
 */
export function applySuggestion(
  value: string,
  ctx: RefContext,
  replacement: string,
): PairCompletion {
  const before = value.slice(0, ctx.openIndex);
  const after = value.slice(ctx.replaceEnd);
  let rebuilt: string;
  if (ctx.kind === "tag") {
    // No brackets and no closer: a tag *is* its text.
    rebuilt = `#${replacement}`;
  } else if (ctx.kind === "mention") {
    // Mention sugar: the page identity does NOT carry the `@`, so the
    // caller passes the page title verbatim. We add the `@` only on
    // the link side (where it acts as the mention affordance).
    rebuilt = `[[@${replacement}]]`;
  } else {
    const [open, close] = ctx.kind === "page" ? ["[[", "]]"] : ["((", "))"];
    rebuilt = `${open}${replacement}${close}`;
  }
  return {
    value: before + rebuilt + after,
    caret: before.length + rebuilt.length,
  };
}

/**
 * Scan forward from `from` looking for `closer`. If the closer is
 * found before the end of the line, return the index *after* it.
 * Otherwise return `from` so callers replace only up to the caret.
 */
function findCloser(value: string, from: number, closer: string): number {
  for (let i = from; i < value.length - 1; i += 1) {
    if (value[i] === "\n") return from;
    if (value.slice(i, i + 2) === closer) return i + 2;
  }
  return from;
}

/**
 * Append a synthetic "create new person" candidate to the suggestion
 * list when the typed query doesn't match any existing person
 * exactly (case-insensitive). Matches the TUI's `candidates_for_mention`
 * policy so every client surfaces the same affordance on the same
 * gesture.
 *
 * Returns the input array verbatim when:
 * - The query is empty or whitespace-only (avoids polluting the empty
 *   popup with a useless "create ''" row).
 * - An existing person already matches the query exactly
 *   (case-insensitive on the `title`).
 *
 * Otherwise returns a new array with one extra candidate appended:
 * a `PageMeta`-shaped record with `id: ""` (synthetic, not persisted
 * yet), `slug` / `title` set to the typed query, `kind: "page"`, and
 * `page_type: "person"`. The downstream `acceptSuggestion` path
 * materialises the page via `openRef(`@${title}`)` (idempotent on the
 * resolver side) — clients do not need to know whether the candidate
 * was synthetic or real.
 */
export function withCreateNewPersonCandidate<T extends PageMetaLike>(
  list: T[],
  query: string,
): T[] {
  const trimmed = query.trim();
  if (trimmed.length === 0) return list;
  const lower = trimmed.toLowerCase();
  if (
    list.some(
      (p) =>
        p.title.toLowerCase() === lower ||
        (p.aliases ?? []).some((a) => a.toLowerCase() === lower),
    )
  ) {
    return list;
  }
  const synthetic = {
    id: "",
    slug: query,
    title: query,
    kind: "page" as const,
    page_type: "person",
  };
  return [...list, synthetic as unknown as T];
}

/**
 * Minimal shape `withCreateNewPersonCandidate` needs from a candidate.
 * Kept structural (not bound to `PageMeta`) so callers can extend the
 * shape with client-specific fields without losing the helper.
 */
export interface PageMetaLike {
  id: string;
  slug: string;
  title: string;
  kind: "page" | "journal";
  page_type?: string | null;
  aliases?: string[];
}

/**
 * The page name spliced into `[[…]]` when a ref suggestion is
 * accepted. Journals are anchored on their ISO slug (`2026-06-08`);
 * every other page inserts its **title**, which renders verbatim
 * inside `[[…]]` and still resolves through the slugified-match arm of
 * `open_or_create_by_ref` (so `[[avelino/outl]]` finds the page stored
 * as `avelino-outl`). Inserting the slug for regular pages was bug
 * #88: the suggestion chip showed the title but the accept wrote the
 * slug.
 *
 * `@` mentions (`opts.mention`) always insert the title — the page
 * identity carries no `@`; {@link applySuggestion} wraps the title as
 * `[[@<title>]]` on the link side.
 *
 * One owner for the rule: the desktop ref popup and the mobile native
 * chip strip both call this, so the two accept paths can't drift.
 */
export function refReplacement(
  page: Pick<PageMetaLike, "kind" | "slug" | "title">,
  opts: { mention?: boolean; tag?: boolean } = {},
): string {
  // A tag is always the slug. `#` has no closing delimiter, so the
  // first space ends the tag — inserting a title like "Deep Work"
  // would write `#Deep` and leave "Work" as loose prose.
  if (opts.tag) return page.slug;
  return page.kind === "journal" && !opts.mention ? page.slug : page.title;
}

/** An active `/command` slash trigger inside a block's textarea. */
export interface SlashContext {
  /** Text after the `/`, up to the caret — the command filter. */
  query: string;
  /** Index of the `/` (always 0 today: Notion-style, block-initial). */
  openIndex: number;
  /** Caret position (no closer — equals `selection`). */
  replaceEnd: number;
}

/**
 * Detect a `/command` slash trigger, mirroring the TUI's slash menu but
 * inline inside a block. Deliberately **block-initial only** (`/` is the
 * first character): a mid-text slash is a path / URL / fraction, never a
 * command, so triggering there would fire the menu constantly. The run
 * between the `/` and the caret must be command-name chars (`[A-Za-z0-9_-]`);
 * a space or any other character closes the trigger (Notion behaviour).
 *
 * Returns `null` when no trigger is active.
 */
export function detectSlashContext(
  value: string,
  selection: number,
): SlashContext | null {
  if (selection < 1 || value[0] !== "/") return null;
  const query = value.slice(1, selection);
  if (!/^[A-Za-z0-9_-]*$/.test(query)) return null;
  return { query, openIndex: 0, replaceEnd: selection };
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
  while (end < value.length && /[A-Za-z0-9_-]/.test(value[end])) end++;
  const next = value.slice(0, ctx.openIndex) + value.slice(end);
  return { value: next, caret: ctx.openIndex };
}
