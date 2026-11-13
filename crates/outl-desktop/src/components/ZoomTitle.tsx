import { Show } from "solid-js";

import type { BlockNode } from "@outl/shared/api/types";
import { MarkdownInline } from "@outl/shared/markdown";

import { appState } from "../lib/store";

/**
 * The words inside the zoom header's `<h1>` — the sibling of
 * `<PageTitle />`, for the other branch of the same `<Show>`.
 *
 * Zooming promotes a block to a page-like title, and a block's text is
 * markdown everywhere else in the app, so it is markdown here too:
 * `#1on1 [[@Roberta]] · [GCal](https://…)` reads as a tag, a ref and a
 * link, and both the ref and the tag navigate exactly as they do from
 * the row the user zoomed out of. The `<h1>` used to interpolate
 * `root.text` raw, which was the one surface that showed a user their
 * own source.
 *
 * Two deliberate differences from `<BlockBody />`, which renders the
 * same tokens one level down:
 *
 * - **No quote strip.** `<BlockBody />` drops the `> ` because
 *   `<BlockRow />` wraps it in `<QuoteWrap />` chrome that says the
 *   same thing. There is no chrome around a title, so stripping the
 *   marker here would *lose* the fact that the block is a quote.
 * - **No `blockAssets`.** `variant="inline"` therefore renders an
 *   asset as an inline chip; a block-sized image is a row's
 *   affordance (issue #322), not a heading's.
 */
export function ZoomTitle(props: {
  root: BlockNode;
  onRefClick: (target: string) => void;
  onTagClick: (tag: string) => void;
  onLinkClick?: (href: string) => void;
}) {
  return (
    <Show
      when={props.root.text}
      fallback={<span class="opacity-40">(empty block)</span>}
    >
      {/* A block the backend sent untokenized still shows its text —
          same fallback `<BlockBody />` keeps for the row. */}
      <Show when={props.root.tokens.length > 0} fallback={props.root.text}>
        <MarkdownInline
          tokens={props.root.tokens}
          variant="inline"
          onRefClick={props.onRefClick}
          onTagClick={props.onTagClick}
          onLinkClick={props.onLinkClick}
          embeds={appState.embeds}
        />
      </Show>
    </Show>
  );
}
