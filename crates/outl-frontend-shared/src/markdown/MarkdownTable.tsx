import { For, JSX } from "solid-js";

import type { ColumnAlign, TableView } from "../api/table";
import type { InlineToken } from "../api/types";
import { MarkdownInline, type EmbedMap } from "./MarkdownInline";

/**
 * Render a block whose whole text is a markdown table as an HTML
 * `<table>`.
 *
 * Why this lives in `@outl/shared`: desktop and mobile paint the same
 * grid from the same {@link TableView}, and a table is exactly the kind
 * of thing that drifts when two clients each build it — one would get
 * `align: center` right and the other wouldn't, and nothing would fail.
 * The chrome that differs between clients (how a row reacts to a tap)
 * stays with the caller, same split as {@link QuoteWrap}.
 *
 * **No parsing happens here.** `outl_md::tokenize_table` decided what
 * the cells are and tokenized each one, so a cell's `[[ref]]`,
 * `**bold**` or `((blk-…))` renders exactly like it does in a prose
 * block. A client that split the pipes itself would have no tokenizer
 * and would show that `[[ref]]` as literal text.
 *
 * ## What it deliberately does not do
 *
 * - **No editing affordances** (add row, add column). Out of scope for
 *   [issue #329](https://github.com/outlmd/outl/issues/329): the block's
 *   raw markdown is what gets edited, which is why `onEdit` drops the
 *   caller into its own editor rather than making a cell editable.
 * - **No sort / filter.** The table is the user's text, not a view over
 *   a query.
 */
export interface MarkdownTableProps {
  /** The backend's reading of the block — `BlockNode.table`. */
  table: TableView;
  /** Passed through to every cell. See `MarkdownInline`'s `variant`. */
  variant?: "pill" | "inline";
  onRefClick?: (target: string) => void;
  onTagClick?: (tag: string) => void;
  onLinkClick?: (href: string) => void;
  embeds?: EmbedMap;
  /**
   * Called when the user taps the grid, so the host can drop into its
   * raw-markdown editor — the same gesture that starts editing a prose
   * block. A ref / tag / link inside a cell stops propagation, so
   * following a link does not also open the editor.
   *
   * Omitted in read-only contexts (a backlink, an embedded subtree).
   */
  onEdit?: () => void;
}

/**
 * Tailwind class for a column's alignment.
 *
 * `"none"` renders left: every client outl ships reads an unmarked
 * column that way, and the on-disk delimiter is what preserves the
 * distinction (see `ColumnAlign`). The returns are string literals so
 * Tailwind's JIT finds them at build time.
 */
function alignClass(align: ColumnAlign | undefined): string {
  if (align === "center") return "text-center";
  if (align === "right") return "text-right";
  return "text-left";
}

export function MarkdownTable(props: MarkdownTableProps): JSX.Element {
  // Not destructured: a Solid prop read through a local binding freezes
  // at first render (root CLAUDE.md anti-patterns).
  //
  // No empty-cell branch: `MarkdownInline` maps over its tokens, so an
  // empty list already renders nothing and the padded `<td>` keeps its
  // place in the row.
  const cell = (tokens: InlineToken[]): JSX.Element => (
    <MarkdownInline
      tokens={tokens}
      variant={props.variant}
      onRefClick={props.onRefClick}
      onTagClick={props.onTagClick}
      onLinkClick={props.onLinkClick}
      embeds={props.embeds}
    />
  );

  return (
    // `overflow-x-auto` is what makes a wide table usable on a phone:
    // the grid scrolls inside the block instead of stretching the
    // outline and pushing every sibling off-screen. `w-max min-w-full`
    // lets a narrow table still fill the row.
    <div
      class="my-0.5 max-w-full overflow-x-auto"
      classList={{ "cursor-text": !!props.onEdit }}
      onClick={() => props.onEdit?.()}
    >
      <table class="w-max min-w-full border-collapse text-[14px]">
        <thead>
          <tr>
            <For each={props.table.header}>
              {(tokens, col) => (
                <th
                  class={`border border-(--color-outl-border) bg-(--color-outl-bg-elev) px-2 py-1 font-semibold ${alignClass(
                    props.table.aligns[col()],
                  )}`}
                >
                  {cell(tokens)}
                </th>
              )}
            </For>
          </tr>
        </thead>
        <tbody>
          <For each={props.table.rows}>
            {(row) => (
              <tr>
                <For each={row}>
                  {(tokens, col) => (
                    <td
                      class={`border border-(--color-outl-border) px-2 py-1 align-top ${alignClass(
                        props.table.aligns[col()],
                      )}`}
                    >
                      {cell(tokens)}
                    </td>
                  )}
                </For>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </div>
  );
}
