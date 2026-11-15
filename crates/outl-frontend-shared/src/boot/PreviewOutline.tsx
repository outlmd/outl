/**
 * The page-first boot's visible half: real text, before the workspace.
 *
 * This is a **second** outline renderer, and that needs justifying,
 * because a second renderer is normally the drift this library exists
 * to remove.
 *
 * It is here because the clients' real outlines are not separable from
 * editing. `BlockRow` on either client binds callbacks for every
 * mutation, holds a draft signal, arms a long-press, and routes taps
 * into commands that need a `Workspace` — which is precisely the thing
 * that is not open yet. Reusing it would mean threading a
 * "nothing works yet" flag through every one of those paths, and a flag
 * like that is wrong in exactly one direction: the day it leaks, a
 * keystroke goes to a workspace that cannot take it.
 *
 * So this renderer has **no interaction at all**. No click handlers, no
 * editing, no refs that navigate. It cannot drift into being editable,
 * because there is nothing in it to make editable.
 *
 * What it must get right is the *shape*: indentation, bullets and text
 * colours close enough that swapping it for the real outline does not
 * move the user's eye. It is deliberately not pixel-identical, and
 * chasing that is how this file would grow into the thing it is
 * avoiding.
 */
import { For, Show, type JSX } from "solid-js";

import type { BlockNode } from "../api/types";
import { MarkdownInline } from "../markdown";

interface PreviewOutlineProps {
  /** The preview's blocks, straight from the `page-preview` payload. */
  outline: BlockNode[];
  /** Nesting depth, used internally by the recursion. */
  depth?: number;
}

/** Indent per level, matching the clients' outline rhythm closely enough. */
const INDENT_PX = 20;

/**
 * Read-only render of a preview's outline.
 *
 * Renders `todo` markers as plain glyphs rather than checkboxes: a
 * checkbox invites a tap that cannot do anything yet.
 */
export function PreviewOutline(props: PreviewOutlineProps): JSX.Element {
  const depth = (): number => props.depth ?? 0;
  return (
    <For each={props.outline}>
      {(node) => (
        <div>
          <div
            class="flex items-start gap-2 py-[3px]"
            style={{ "padding-left": `${depth() * INDENT_PX}px` }}
          >
            <span
              aria-hidden="true"
              class="mt-[0.55em] h-[5px] w-[5px] shrink-0 rounded-full bg-(--color-outl-fg)/35"
            />
            <span class="min-w-0 text-(--color-outl-fg)/80">
              <Show when={node.todo}>
                <span class="mr-1 opacity-60">
                  {node.todo === "DONE" ? "☑" : node.todo === "DOING" ? "◐" : "☐"}
                </span>
              </Show>
              <MarkdownInline tokens={node.tokens ?? []} variant="inline" />
            </span>
          </div>
          <Show when={node.children?.length}>
            <PreviewOutline outline={node.children} depth={depth() + 1} />
          </Show>
        </div>
      )}
    </For>
  );
}
