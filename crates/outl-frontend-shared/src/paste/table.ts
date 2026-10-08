/**
 * Tabular clipboard → outl markdown table.
 *
 * Two flavours arrive, and both have to end up as the same thing:
 *
 * - **`text/html` with a `<table>`** — a spreadsheet, a Notion
 *   database, a web page, a Google Doc. Handled here, because HTML →
 *   markdown only exists on the TypeScript side (Turndown).
 * - **`text/plain` as tab-separated lines** — the same spreadsheet's
 *   plain flavour, a SQL client, `column -t`. Handled in Rust by
 *   `outl_md::tsv_to_markdown`, which the backend's paste pipeline
 *   calls; the only thing needed here is {@link looksLikeTable} so the
 *   client routes the payload to the backend instead of letting the
 *   browser splice it into the textarea.
 *
 * Turndown ships no table rule, so without this a pasted `<table>`
 * collapsed into a run of cell text with the structure gone — the
 * clipboard's richest flavour producing the poorest result.
 *
 * ## Why the cells are converted through a second Turndown instance
 *
 * The usual way to add tables (what `turndown-plugin-gfm` does) is
 * three cooperating rules — `th`/`td`, `tr`, `table` — each reading the
 * `content` the level below produced. That works, and it spreads one
 * decision ("what does a row look like") across three replacements that
 * have to agree on where the pipes go.
 *
 * Reading the DOM in one rule keeps that decision in one place, and the
 * cost is needing a converter for each cell's own inline markdown. A
 * nested `turndown()` call on the *same* instance is reentrancy this
 * module should not have to reason about, so cells get their own
 * instance, built from the same factory. Same rules, same dialect, no
 * shared call stack.
 */

import type TurndownService from "turndown";

/** Alignment markers, matching `outl_md::ColumnAlign`'s delimiters. */
const RULES: Record<string, string> = {
  left: ":---",
  center: ":---:",
  right: "---:",
  none: "---",
};

/**
 * A cell's declared alignment, from the inline `text-align` a
 * spreadsheet writes or the legacy `align` attribute.
 */
function alignOf(cell: HTMLElement): string {
  const inline = cell.style?.textAlign?.toLowerCase().trim();
  const attr = cell.getAttribute?.("align")?.toLowerCase().trim();
  const align = inline || attr || "";
  return align in RULES && align !== "none" ? align : "none";
}

/**
 * One cell's text: inline markdown, flattened to a single line with its
 * pipes escaped.
 *
 * A markdown row cannot carry a newline (a spreadsheet cell can), so a
 * hard break becomes a space — the same lossy step, and the same
 * reasoning, as `outl_md::table::escape_cell`. Returns one entry per
 * column the element occupies, so `colspan` lands here rather than in
 * the caller's loop.
 */
function cellText(cell: HTMLElement, convert: (html: string) => string): string[] {
  const text = convert(cell.innerHTML ?? "")
    .replace(/\s*\n\s*/g, " ")
    // Count the backslashes already in front of the pipe. Turndown
    // escapes `\` in a text node but deliberately does not inside
    // `<code>`, so a `<td><code>a\|b</code></td>` arrives with one
    // backslash; escaping the pipe blindly makes an **even** run, which
    // `split_cells` reads as a real delimiter and the cell splits in
    // two. Same rule as Rust's `escape_cell`.
    .replace(/(\\*)\|/g, (_m, bs: string) =>
      bs.length % 2 === 0 ? `${bs}\\|` : `${bs}|`,
    )
    .trim();
  // A merged cell spans columns the markdown grid has no merge for.
  // Emitting the empty ones keeps every later cell under the right
  // header, which matters more than reproducing the merge.
  //
  // Clamped at the HTML spec's own colspan ceiling: `getAttribute`
  // returns the literal attribute, so `colspan="2000000000"` asked for
  // a 16 GB array and `colspan="50000000"` froze the webview for
  // seconds before throwing `RangeError: Invalid string length`.
  const span = Number.parseInt(cell.getAttribute?.("colspan") ?? "1", 10);
  const extra = Number.isFinite(span) ? Math.min(Math.max(0, span - 1), 999) : 0;
  return [text, ...Array.from({ length: extra }, () => "")];
}

/** The `<td>` / `<th>` children of a row, in order. */
function cellElements(row: HTMLElement): HTMLElement[] {
  return (Array.from(row.children) as HTMLElement[]).filter((c) =>
    ["td", "th"].includes(c.nodeName.toLowerCase()),
  );
}

function line(cells: string[]): string {
  return `| ${cells.join(" | ")} |`;
}

/**
 * Convert a `<table>` element to a markdown table, `""` when it holds
 * no cells.
 *
 * The first row becomes the header, whether or not it is a `<thead>`:
 * markdown has no headerless table, and a table whose first row is
 * data reads better with that row as the header than with an invented
 * empty one.
 */
export function tableElementToMarkdown(
  table: HTMLElement,
  convert: (html: string) => string,
): string {
  // One pass: converting a cell runs Turndown over its HTML, so asking
  // a second time to find the header row would convert the whole table
  // twice. `aligns` is read off the same row that becomes the header.
  let aligns: string[] = [];
  const grid: string[][] = [];
  // `:scope >` so a `<table>` nested inside a cell does not donate its
  // rows to this grid. Without it, `querySelectorAll` matched every
  // descendant `<tr>`, so the inner rows were appended here *and*
  // flattened inline into the cell that contains them — duplicated
  // content and an inflated column count, from an HTML email or any
  // legacy layout table.
  const rows = table.querySelectorAll?.(
    ":scope > tr, :scope > thead > tr, :scope > tbody > tr, :scope > tfoot > tr",
  );
  for (const row of Array.from(rows ?? [])) {
    const cells = cellElements(row as HTMLElement);
    if (cells.length === 0) continue;
    if (grid.length === 0) aligns = cells.map(alignOf);
    grid.push(cells.flatMap((c) => cellText(c, convert)));
  }
  if (grid.length === 0) return "";

  // `reduce`, not `Math.max(...spread)`: the spread is an argument list,
  // and a table with ~200k rows overflows the call stack.
  const columns = grid.reduce((max, cells) => Math.max(max, cells.length), 0);
  const pad = (cells: string[]): string[] =>
    cells.concat(Array.from({ length: columns - cells.length }, () => ""));
  const rule = Array.from(
    { length: columns },
    (_, col) => RULES[aligns[col] ?? "none"],
  );

  const [header, ...body] = grid;
  return [line(pad(header)), line(rule), ...body.map((r) => line(pad(r)))].join(
    "\n",
  );
}

/**
 * Install the `<table>` rule on a configured Turndown instance.
 *
 * `cellConverter` builds the second instance lazily — a clipboard
 * without a table never pays for it.
 */
export function addTableRule(
  service: TurndownService,
  cellConverter: () => TurndownService,
): void {
  service.addRule("gfmTable", {
    filter: "table",
    replacement: (_content, node) => {
      const md = tableElementToMarkdown(node as unknown as HTMLElement, (html) =>
        cellConverter().turndown(html),
      );
      // Blank lines around it so the table is its own block once the
      // backend splits the payload, never glued to the sentence above.
      return md ? `\n\n${md}\n\n` : "";
    },
  });
}
