/**
 * Heuristic and helpers for the external-clipboard paste flow.
 *
 * The actual markdown → block-tree conversion happens in the Rust
 * backend (`outl_actions::paste_markdown`, exposed as the
 * `paste_markdown_at` Tauri command). The frontend's job is to:
 *
 * 1. Detect that the user pasted *outline-like* content (otherwise we
 *    let the browser do its default thing — splice the text into the
 *    textarea — so plain text doesn't trigger a round trip).
 * 2. Hand the raw clipboard string to the backend along with the
 *    block id and caret position; the backend mutates the workspace
 *    and returns the refreshed page view.
 *
 * The same heuristic is mirrored in
 * `outl_actions::paste::looks_like_outline`. Keep them in sync.
 *
 * Rich-clipboard (`text/html`) → outl markdown conversion lives in
 * `./html` (`htmlToOutlMarkdown`), re-exported here.
 */

import { htmlToOutlMarkdown } from "./html";

export { htmlToOutlMarkdown };
export { tableElementToMarkdown } from "./table";

/**
 * True when `text` looks like a markdown bullet list (at least one
 * non-blank line starts with `- ` or is just `-`). The detector errs
 * on the side of "outline" — false positives only cost one Tauri
 * round trip while false negatives lose the user's hierarchy.
 *
 * Mirror of `outl_actions::paste::looks_like_outline`. Keep both in
 * sync — the Rust side is the canonical contract; the JS copy exists
 * to gate the Tauri round-trip before the user sees a flash of
 * "browser default splice" while the backend runs.
 */
export function looksLikeOutline(text: string): boolean {
  if (!text) return false;
  for (const line of text.split(/\r?\n/)) {
    const trimmed = line.replace(/^[ \t]+/, "");
    if (trimmed === "-" || trimmed.startsWith("- ")) {
      return true;
    }
  }
  return false;
}

/**
 * True when `text` carries a markdown table — a header row, a delimiter
 * row under it, and any rows after that.
 *
 * The second gate on routing a paste to the backend, beside
 * {@link looksLikeOutline}. Without it a pasted table stays on the
 * native splice (a single-line payload) or lands as one block per row
 * (multi-line), and the user undoes it by hand.
 *
 * Mirror of `outl_actions::paste::looks_like_table`, which asks
 * `outl_md::table_span` at every line. **Keep both in sync** — the Rust
 * side is the canonical contract; this copy only decides whether the
 * round-trip happens. It errs towards "table" the same way the outline
 * detector errs towards "outline": a false positive costs one round
 * trip, a false negative costs the user's structure.
 *
 * A pipe alone is not enough, here or in Rust: `run \`a | b\`` is prose,
 * and the delimiter row is what declares the intent.
 */
export function looksLikeTable(text: string): boolean {
  if (!text || !text.includes("|")) return false;
  const lines = text.split(/\r?\n/);
  for (let i = 0; i + 1 < lines.length; i += 1) {
    if (isTableRow(lines[i]) && isDelimiterRow(lines[i + 1])) return true;
  }
  return false;
}

/** A line that could be a row: non-blank, carries a pipe, not a bullet. */
function isTableRow(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed || trimmed === "-" || trimmed.startsWith("- ")) return false;
  return /(^|[^\\])\|/.test(trimmed);
}

/** `| --- | :-: |` — every cell a rule. Mirror of `is_delimiter_row`. */
function isDelimiterRow(line: string): boolean {
  const trimmed = line.trim();
  if (!trimmed.includes("|")) return false;
  const cells = trimmed
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|");
  return cells.length > 0 && cells.every((c) => /^\s*:?-+:?\s*$/.test(c));
}

/**
 * True when `text` is tab-separated tabular data — what a spreadsheet,
 * a SQL client or `column -t` puts on the clipboard as `text/plain`.
 *
 * Mirror of `outl_md::from_delimited`'s gate, which does the actual
 * conversion on the backend. **Keep both in sync.** The three
 * conditions and why each one is there:
 *
 * - **Two or more lines** — one row has no header to rule off.
 * - **The same field count on every line, two or more** — a ragged
 *   count is prose or code that happens to carry tabs.
 * - **No line starts with a tab** — tab-indented code and tab-indented
 *   outlines pass the first two, and rearranging either into a grid
 *   destroys it.
 *
 * Comma-separated text is deliberately **not** tabular: prose carries
 * commas, and `a, b` on two lines is far more often two sentences than
 * a 2×2 grid.
 */
export function looksLikeTabular(text: string): boolean {
  if (!text.includes("\t")) return false;
  const lines = text.split(/\r?\n/).filter((l) => l.trim() !== "");
  if (lines.length < 2 || lines.some((l) => l.startsWith("\t"))) return false;
  const width = lines[0].split("\t").length;
  return width >= 2 && lines.every((l) => l.split("\t").length === width);
}

/**
 * True when `text` has two or more non-blank lines — the gate for "paste
 * with formatting" to split plain prose into one block per line (a pasted
 * chat reply, an email). In a `text/plain` clipboard a paragraph is a
 * single line (a long sentence wraps visually, with no newline), so the
 * line count is the paragraph count; blank lines are ignored.
 *
 * Mirror of the Rust `outl_actions::paste::split_paragraphs(...).len() > 1`
 * gate (one block per non-blank line). Keep the two in sync. Used
 * alongside {@link looksLikeOutline} to decide whether a paste is routed
 * to the backend at all — a single line (a URL, one sentence) stays on
 * the native splice.
 */
export function hasMultipleParagraphs(text: string): boolean {
  let count = 0;
  for (const line of text.split(/\r?\n/)) {
    if (line.trim() !== "") {
      count += 1;
      if (count >= 2) return true;
    }
  }
  return false;
}

/**
 * Convert a UTF-16 code unit offset (what the DOM textarea reports
 * via `selectionStart`) into a Unicode-codepoint offset, which is
 * what `outl_actions::PasteAnchor::AtCaret { caret }` expects — Rust
 * `str::chars()` iterates codepoints, not UTF-16 code units.
 *
 * For text that lives entirely in the BMP (every codepoint ≤ U+FFFF)
 * the two counts are equal, so this is a no-op for ASCII / most CJK.
 * It only matters for content with characters in supplementary planes
 * — emoji, mathematical symbols, less-common CJK extensions — where
 * each codepoint takes 2 UTF-16 code units. Without this conversion,
 * pasting after such a character lands the splice one position too
 * late per high-plane char before the caret.
 */
export function utf16OffsetToCharOffset(
  text: string,
  utf16Offset: number,
): number {
  if (utf16Offset <= 0) return 0;
  let chars = 0;
  let i = 0;
  while (i < utf16Offset && i < text.length) {
    const cp = text.codePointAt(i);
    if (cp === undefined) break;
    i += cp > 0xffff ? 2 : 1;
    chars += 1;
  }
  return chars;
}

/**
 * How a "paste with formatting" should be handled, decided from the two
 * clipboard flavours (`text/html` + `text/plain`).
 *
 * - `rich` — the HTML carried formatting the plain text lacks; `text` is
 *   the converted outl markdown, send it to the backend.
 * - `structured` — no richer HTML, but the plain text is an outline or
 *   multiple paragraphs; `text` is the plain text, the backend splits /
 *   normalises it.
 * - `native` — trivial single-paragraph plain text; let the browser
 *   splice it in place (no backend round-trip).
 */
export type PasteRoute =
  | { route: "rich"; text: string }
  | { route: "structured"; text: string }
  | { route: "native" };

/**
 * Single source of truth for the paste-with-formatting routing decision,
 * shared by the desktop (`Cmd/Ctrl+V`) and mobile paste handlers so the
 * two clients can never drift.
 *
 * The rule, in order:
 * 1. Convert `text/html` to markdown ({@link htmlToOutlMarkdown}). If it
 *    is non-empty AND differs from the trimmed plain text, the clipboard
 *    is *rich* — route the markdown (`bold` / links / lists survive).
 * 2. Otherwise, if the plain text {@link looksLikeOutline},
 *    {@link looksLikeTable}, {@link looksLikeTabular} or
 *    {@link hasMultipleParagraphs}, route the plain text as
 *    *structured* — the backend splits / normalises it, and turns
 *    tab-separated rows into a table.
 * 3. Otherwise it is trivial — `native`, no round-trip.
 *
 * `html` / `plain` are the raw `clipboardData.getData(...)` strings
 * (either may be `""`).
 */
export function choosePasteRoute(html: string, plain: string): PasteRoute {
  const md = html ? htmlToOutlMarkdown(html) : "";
  // "Rich" = the HTML added formatting the plain flavour doesn't already
  // carry. Comparing against the trimmed plain avoids a needless backend
  // round-trip when the HTML is just a styled wrapper around the same text.
  if (md !== "" && md !== plain.trim()) {
    return { route: "rich", text: md };
  }
  // The two table detectors are **redundant here** and kept for
  // readability: both require two or more non-blank lines, which
  // `hasMultipleParagraphs` already answers `true` for, so neither can
  // change a route today. They are the exported mirror of
  // `outl_actions::paste::{looks_like_table, looks_like_tabular}` and
  // the thing to reach for on a surface that is *not* behind the
  // paragraph gate — the TUI's `looks_structured` shape, say. Do not
  // assume they are what protects a pasted table.
  if (
    plain &&
    (looksLikeOutline(plain) ||
      looksLikeTable(plain) ||
      looksLikeTabular(plain) ||
      hasMultipleParagraphs(plain))
  ) {
    return { route: "structured", text: plain };
  }
  return { route: "native" };
}
