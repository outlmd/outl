import { For, JSX, Show } from "solid-js";

import type { BlockNode } from "../api/types";
import { MarkdownInline, type EmbedMap } from "./MarkdownInline";
import { MarkdownTable } from "./MarkdownTable";
import type { PageEmbedMap } from "./PageEmbed";

/**
 * Read-only render of transcluded rows — the children of a block pulled
 * in by `!((blk-XXXXXX))`, or the top-level blocks of a page pulled in
 * by `![[page]]`. Both kinds expand through this one component on
 * purpose: they are the same promise ("these rows live somewhere
 * else"), so they must not come to look different.
 *
 * ## The rail is the whole design
 *
 * One continuous line down the left of the region, and nothing else.
 *
 * This replaced a `↳` on **every** row, which on a thirteen-row page
 * read as a wall of arrows. That glyph is the TUI's answer
 * (`outl-tui/src/view/embed.rs`), and it is the right answer *there*: a
 * terminal has no hairline to draw and no second axis to spend. In a
 * webview it was saying "this row is nested", which the indentation
 * already says, thirteen times, while nothing at all said "this came
 * from another page" — the one fact a reader needs and cannot infer.
 *
 * So the rail carries that fact once, for the whole region, and the
 * rows inside get the outline's ordinary rhythm back. **The two clients
 * now diverge from the TUI here deliberately**; the shared promise is
 * the semantics, not the glyph.
 *
 * It is `accent` at **25%**, not the solid `accent` of the selected
 * block's 3px rail (`DESIGN.md` → "The two odd numbers"). Same hue, and
 * that is the point — both mean "outl is marking this region" — but a
 * cursor is one row and solid, a transclusion is many rows and faint.
 * Reusing the solid weight would make the page look permanently
 * selected.
 *
 * No fill, no card, no shadow: `bg_elev` is the only elevation colour
 * in the product and it belongs to things that float (`DESIGN.md` →
 * Colors). An embed sits *in* the page, so it gets a margin, not a
 * surface.
 *
 * Display-only: no `onClick`, no edit affordance, no navigation. The
 * carrying block's own line stays in {@link MarkdownInline} /
 * `PageEmbedTitle`; this draws only what hangs beneath it.
 */

/**
 * Recursion cap. One of three copies — the TUI's `EMBED_MAX_DEPTH` and
 * `outl_actions::SUBTREE_MAX_DEPTH` (the Rust owner) are the others; the
 * latter also caps a backlink row's children. Do not add a fourth. It is also the whole cycle guard: a page that embeds itself
 * stops here rather than at a visited-set.
 */
const MAX_DEPTH = 4;

/**
 * Indent per level, in px. The product's own outline rhythm
 * (`DESIGN.md` → "Outline indentation — 22px per level"), which this
 * component used to contradict with a `1rem` margin: a transcluded
 * subtree stepped at 16px inside a page stepping at 22px, so borrowed
 * rows drifted out of phase with their neighbours the deeper they went.
 */
const INDENT_PX = 22;

/** Task glyph, matching the embed case in `MarkdownInline`. */
function todoMark(todo: BlockNode["todo"]): string {
  if (todo === "DONE") return "✓ ";
  if (todo === "DOING") return "◐ ";
  if (todo === "TODO") return "☐ ";
  return "";
}

interface EmbeddedSubtreeProps {
  /** Rows to render: a block's children, or a page's top-level blocks. */
  nodes: BlockNode[];
  /** Resolved refs/embeds so a nested `((…))` / `!((…))` renders its
   *  content instead of a raw handle. */
  embeds?: EmbedMap;
  /** Same, for a nested `![[page]]`. One inside a transcluded subtree
   *  renders its title only and never expands, which is what keeps one
   *  `![[page]]` from pulling in a graph. */
  pageEmbeds?: PageEmbedMap;
  /** 1-based nesting depth; recursion stops at {@link MAX_DEPTH}.
   *  Defaults to `1`, the outermost call — the one that draws the rail. */
  depth?: number;
}

export function EmbeddedSubtree(props: EmbeddedSubtreeProps): JSX.Element {
  const depth = (): number => props.depth ?? 1;

  const rows = (
    <For each={props.nodes}>
      {(node) => (
        <div style={{ "padding-left": `${(depth() - 1) * INDENT_PX}px` }}>
          <div class="flex gap-[6px] py-[2px] text-[13px] leading-(--outl-block-leading) text-(--color-outl-fg-dim)">
            {/* The outline's own bullet, not an arrow. It marks a row;
                the rail marks the region. Fixed width so wrapped text
                aligns under the first character, never under the
                bullet. */}
            <span
              aria-hidden="true"
              class="w-[6px] shrink-0 select-none text-(--color-outl-fg-dimmer)"
            >
              ·
            </span>
            <span class="min-w-0 flex-1">
              {todoMark(node.todo)}
              <Show
                when={node.table}
                fallback={
                  <MarkdownInline
                    tokens={node.tokens}
                    variant="inline"
                    embeds={props.embeds}
                    pageEmbeds={props.pageEmbeds}
                  />
                }
              >
                {/* An embedded table is a grid, not a wall of pipes.
                    Inert: an embed is a borrowed, read-only view. */}
                <MarkdownTable
                  table={node.table!}
                  variant="inline"
                  embeds={props.embeds}
                  pageEmbeds={props.pageEmbeds}
                />
              </Show>
            </span>
          </div>
          <Show when={depth() < MAX_DEPTH && node.children.length > 0}>
            <EmbeddedSubtree
              nodes={node.children}
              embeds={props.embeds}
              pageEmbeds={props.pageEmbeds}
              depth={depth() + 1}
            />
          </Show>
        </div>
      )}
    </For>
  );

  // Only the outermost call wraps. A nested one is already inside the
  // rail, and a second line per level would redraw the page's indent
  // guides in a colour that means something else.
  return (
    <Show when={depth() === 1} fallback={rows}>
      <div class="my-[2px] border-l-[3px] border-(--color-outl-accent)/25 pl-[10px]">
        {rows}
      </div>
    </Show>
  );
}
