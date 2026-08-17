import { For, JSX, Show } from "solid-js";

import type { BlockNode } from "../api/types";
import { MarkdownInline, type EmbedMap } from "./MarkdownInline";

/**
 * Read-only render of an embedded block's subtree — the children of a
 * block pulled in by a `!((blk-XXXXXX))` embed. Mirrors the TUI's
 * `emit_embedded_children` (`outl-tui/src/view/outline.rs`): every row
 * carries a `↳ ` prefix, nesting indents per depth, and recursion is
 * capped at depth 4 so an embed cycle can't render forever.
 *
 * Display-only, matching the embed case in {@link MarkdownInline}: no
 * `onClick`, no edit affordance, no navigation. The carrying block's
 * own single-line `↳ text` render stays in `MarkdownInline`; this
 * component draws only the descendants beneath it.
 */
const MAX_DEPTH = 4;

/** Task glyph prefix, matching the embed case in `MarkdownInline`. */
function todoMark(todo: BlockNode["todo"]): string {
  if (todo === "DONE") return "✓ ";
  if (todo === "DOING") return "◐ ";
  if (todo === "TODO") return "☐ ";
  return "";
}

interface EmbeddedSubtreeProps {
  /** Children of the embedded source block, rendered as nested rows. */
  nodes: BlockNode[];
  /** Resolved refs/embeds so nested `((…))` / `!((…))` inside the
   *  subtree render their content instead of a raw handle. */
  embeds?: EmbedMap;
  /** 1-based nesting depth; recursion stops once it reaches
   *  {@link MAX_DEPTH}. Defaults to `1` (direct children of the
   *  embedded block). */
  depth?: number;
}

export function EmbeddedSubtree(props: EmbeddedSubtreeProps): JSX.Element {
  const depth = (): number => props.depth ?? 1;
  return (
    <For each={props.nodes}>
      {(node) => (
        <div
          class="text-[13px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)"
          style={{ "margin-left": `${depth()}rem` }}
        >
          <span>
            ↳ {todoMark(node.todo)}
            <MarkdownInline
              tokens={node.tokens}
              variant="inline"
              embeds={props.embeds}
            />
          </span>
          <Show when={depth() < MAX_DEPTH && node.children.length > 0}>
            <EmbeddedSubtree
              nodes={node.children}
              embeds={props.embeds}
              depth={depth() + 1}
            />
          </Show>
        </div>
      )}
    </For>
  );
}
