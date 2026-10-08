/**
 * Rich-clipboard conversion: HTML → outl markdown.
 *
 * When a user copies formatted text (a Slack message, a Google Doc
 * paragraph, a Notion block, a Gmail draft), the clipboard carries the
 * formatting in its `text/html` flavour — the `text/plain` flavour is
 * stripped of bold/italic/links. Reading only `text/plain` (what the
 * paste path did before) threw the formatting away.
 *
 * We run the HTML through **Turndown** (the battle-tested HTML→markdown
 * engine) configured for the outl dialect, then hand the resulting
 * markdown to the same `paste_markdown_at` backend pipeline as any other
 * paste — so `<ul>` becomes a real outline, `<b>` becomes `**bold**`,
 * and a pasted chat reply lands as a formatted tree instead of a wall of
 * plain text.
 *
 * Dialect choices that MUST match the rest of outl:
 * - `emDelimiter: '*'` — outl italics are `*italic*`. Turndown defaults
 *   to `_italic_`, but outl deliberately does **not** treat intra-word
 *   `_` as emphasis (so `chamados_chat` stays literal), so emitting `_`
 *   here would round-trip wrong.
 * - `strongDelimiter: '**'`, `bulletListMarker: '-'`,
 *   `codeBlockStyle: 'fenced'` — the canonical outl markdown surface.
 *
 * Custom rules (the "extend Turndown for what we need" cases):
 * - **strikethrough** — Slack/GitHub `<del>`/`<s>`/`<strike>` → `~~…~~`
 *   (Turndown has no built-in strikethrough rule).
 * - **inline images → alt text** — Slack renders custom emoji as
 *   `<img alt=":bus:">`; the default `![alt](src)` would drop an image
 *   token into the block. We keep the `alt` (the `:shortcode:` outl
 *   already renders) and discard the URL.
 * - **tables** — Turndown ships no table rule, so a pasted `<table>`
 *   (a spreadsheet, a Notion database, a web page) collapsed into a run
 *   of cell text with the structure gone. `./table` converts it to a
 *   markdown table; the backend then reads it as one table block
 *   ([issue #329](https://github.com/outlmd/outl/issues/329)).
 */

import TurndownService from "turndown";

import { addTableRule } from "./table";

/**
 * True when a CSS `font-weight` value reads as bold — the keyword `bold`
 * / `bolder`, or a numeric weight ≥ 600. Used to decode Google-Docs-style
 * inline weights (`700`) into `**` while treating `normal` / `400` (the
 * Docs wrapper) as plain.
 */
/** The element's inline `font-weight` (`""` when unset or not an element). */
function inlineFontWeight(node: unknown): string {
  return (node as HTMLElement).style?.fontWeight ?? "";
}

function isBoldWeight(weight: string): boolean {
  const w = weight.toLowerCase().trim();
  if (w === "bold" || w === "bolder") return true;
  const n = Number.parseInt(w, 10);
  return Number.isFinite(n) && n >= 600;
}

function buildService(): TurndownService {
  const service = new TurndownService({
    headingStyle: "atx",
    bulletListMarker: "-",
    codeBlockStyle: "fenced",
    emDelimiter: "*",
    strongDelimiter: "**",
    linkStyle: "inlined",
    hr: "---",
  });

  // Rich editors (Google Docs above all) encode weight as inline CSS, not
  // `<b>`/`<strong>`: a bold run is `<span style="font-weight:700">`, and
  // Docs wraps the whole payload in `<b style="font-weight:normal">` (a
  // known quirk). Without honouring the inline weight, Docs bold vanishes
  // AND the wrapper bolds the entire block. This rule fires only when an
  // element carries an inline `font-weight`, so plain `<b>`/`<strong>`
  // (Slack, GitHub) still fall through to Turndown's built-in strong rule.
  service.addRule("inlineFontWeight", {
    filter: (node) => {
      const weight = inlineFontWeight(node);
      if (!weight) return false;
      const tag = node.nodeName.toLowerCase();
      // A weighted span, or a <b>/<strong> whose inline weight overrides
      // the tag's implicit bold (the Docs wrapper we must NOT bold).
      return isBoldWeight(weight) || tag === "b" || tag === "strong";
    },
    replacement: (content, node) =>
      !content
        ? ""
        : isBoldWeight(inlineFontWeight(node))
          ? `**${content}**`
          : content,
  });

  // Slack / GitHub strikethrough — Turndown ships no default rule.
  service.addRule("strikethrough", {
    filter: (node) => {
      const tag = node.nodeName.toLowerCase();
      return tag === "del" || tag === "s" || tag === "strike";
    },
    replacement: (content) => (content ? `~~${content}~~` : ""),
  });

  // Inline images collapse to their alt text (Slack custom emoji are
  // `<img alt=":bus:">`); a bare image with no alt is dropped rather
  // than emitting `![](url)` into an outliner block.
  service.addRule("imageAsAlt", {
    filter: "img",
    replacement: (_content, node) =>
      (node as unknown as HTMLImageElement).getAttribute?.("alt") ?? "",
  });

  return service;
}

// Turndown holds no mutable per-call state once configured, so one
// instance is reused across pastes.
let cached: TurndownService | null = null;
// A second one, for the cells of a pasted table. Same factory, so the
// same dialect — it exists only so a cell's conversion does not reenter
// the instance that is already converting the document around it. Built
// on first use: a clipboard with no table never pays for it.
let cachedCell: TurndownService | null = null;

function cellService(): TurndownService {
  cachedCell ??= buildService();
  return cachedCell;
}

/**
 * Collapse Turndown's list-marker padding to the outl list shape.
 *
 * Turndown pads every list marker to four columns (`-   item`, `1.  item`)
 * so nested content lines up under the marker. outl uses a single space
 * after the marker and encodes hierarchy as 2-space indent per level (the
 * backend's `normalize_external_syntax` folds the 4-space nesting Turndown
 * emits into 2-space). We strip the intra-marker padding so an unordered
 * item reads `- item` and an ordered item `1. item`; any leading indent is
 * preserved for the backend to normalise.
 */
function normalizeBullets(md: string): string {
  return md
    .replace(/^([ \t]*)[-*+][ \t]+/gm, "$1- ")
    .replace(/^([ \t]*\d+\.)[ \t]+/gm, "$1 ");
}

/**
 * Convert a `text/html` clipboard payload to outl markdown.
 *
 * Returns a trimmed markdown string. Empty (`""`) when the HTML carries
 * no textual content — the caller then falls back to `text/plain`.
 */
export function htmlToOutlMarkdown(html: string): string {
  if (!html.trim()) return "";
  if (cached === null) {
    cached = buildService();
    // On the document instance only: a `<table>` nested in a cell is
    // not a shape this dialect has, and installing the rule on the cell
    // converter too would let one recurse forever.
    addTableRule(cached, cellService);
  }
  return normalizeBullets(cached.turndown(html)).trim();
}
