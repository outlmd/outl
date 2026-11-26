import { Show, type JSX } from "solid-js";
import type { BlockNode } from "@outl/shared/api/types";
import type { EmbedMap, PageEmbedMap } from "@outl/shared/markdown";
import {
  BlockProperties,
  EmbeddedSubtree,
  MarkdownInline,
  MarkdownTable,
  splitQuote,
  stripQuoteFromTokens,
} from "@outl/shared/markdown";
import { DiagramFence, chooseFence, diagramsMode } from "@outl/shared/diagram";
import { HighlightedCode, detectFence } from "@outl/shared/highlight";
import { transformerFor } from "@outl/shared/plugins/transformer-registry";
import { pageEmbedOnlyName } from "@outl/shared/outline";
import { PluginFence } from "./PluginFence";

export interface BlockReadBodyProps {
  block: BlockNode;
  /** RFC 0254 phase 3 — while a range selection is active every
   *  handler below is withheld, so a tap extends the range instead of
   *  following a ref (see the body). */
  selectionMode?: boolean;
  onRefClick?: (target: string) => void;
  onTagClick?: (tag: string) => void;
  /** External `[label](url)` link tap — opens in the system browser. */
  onLinkClick?: (href: string) => void;
  /** Open the properties sheet on this block. Omit and the attribute
   *  line is inert text, which is what a backlink row passes. */
  onOpenProperties?: (blockId: string) => void;
  /** Resolved `((blk-…))` / `!((blk-…))` sources, keyed by handle. */
  embeds?: EmbedMap;
  /** Resolved `![[page]]` targets, keyed by the name as written. */
  pageEmbeds?: PageEmbedMap;
}

/**
 * A block in **read mode**: what the outline row shows when it is not
 * being edited, and what a "Linked from" row shows for the block it
 * cites.
 *
 * One component for both on purpose. The backlink row used to carry
 * its own "table, else inline tokens" renderer, so a fence, a quote,
 * several lines, the property line and a page embed all came out
 * differently there than on the block's own page.
 * `BacklinksSection.parity.test.ts` keeps it that way.
 *
 * The blockquote chrome is **not** here: it wraps this one level up
 * (`<QuoteWrap>` in `BlockBody` and in the backlink row), so the
 * bullet stays outside the quote. Nothing here starts an edit either —
 * a tap that no child consumed bubbles to the host's own handler.
 */
export function BlockReadBody(props: BlockReadBodyProps): JSX.Element {
  /** Top-level blocks of the page this block transcludes, when the
   *  block is nothing but a `![[page]]` embed. An accessor, not a
   *  const: both `props.block` and `props.pageEmbeds` change under us,
   *  and a prop read once at first render freezes (root `CLAUDE.md`
   *  anti-patterns). */
  const pageEmbedBlocks = (): BlockNode[] | undefined => {
    const name = pageEmbedOnlyName(props.block.tokens);
    return name ? props.pageEmbeds?.[name]?.blocks : undefined;
  };

  // Both renders ask this: the shared components only
  // `stopPropagation` a tap when the handler is set, so
  // withholding it while selecting is what turns a tap
  // into "extend the range" instead of "follow this ref".
  const tap = (h?: (v: string) => void) =>
    props.selectionMode ? undefined : h;
  const fence = detectFence(props.block.text);
  if (fence) {
    const plainFence = () => (
      <HighlightedCode
        language={fence.language}
        code={fence.body || " "}
      />
    );
    // A plugin content-transformer may claim this fence's
    // language: render its descriptor inline (text/markdown
    // or a sandboxed iframe for `rich`), falling back to the
    // plain highlighted code while it loads or if it declines.
    const transformer = transformerFor(fence.language);
    const choice = chooseFence({
      hasTransformer: transformer != null,
      language: fence.language,
      diagrams: diagramsMode(),
    });
    if (choice === "plugin" && transformer) {
      return (
        <PluginFence
          blockId={props.block.id}
          transformer={transformer}
          body={fence.body}
          fallback={plainFence}
        />
      );
    }
    if (choice === "diagram") {
      return (
        <DiagramFence
          source={fence.body}
          onOpenPage={tap(props.onRefClick)}
          onLinkClick={tap(props.onLinkClick)}
          fallback={plainFence()}
        />
      );
    }
    return plainFence();
  }
  // No `onEdit` on purpose: the tap bubbles to this
  // row's handler (focus, or extend the range), exactly
  // like a tap on prose.
  if (props.block.table) {
    return (
      <MarkdownTable
        table={props.block.table}
        onRefClick={tap(props.onRefClick)}
        onTagClick={tap(props.onTagClick)}
        onLinkClick={tap(props.onLinkClick)}
      />
    );
  }
  // Chrome lives on the wrapper one level up; here we
  // only strip `> ` from the first Plain token so the
  // marker doesn't double-paint.
  const split = splitQuote(props.block.text);
  const tokens = split.quoted
    ? stripQuoteFromTokens(props.block.tokens)
    : props.block.tokens;
  const bodyLength = split.quoted
    ? split.body.length
    : props.block.text.length;
  // `whitespace-pre-wrap`, as the desktop's prose has: a block's text
  // can hold several lines, and without it they ran together into one.
  return (
    <>
    <p
      class="whitespace-pre-wrap break-words text-[17px] leading-(--outl-block-leading)"
      classList={{
        "text-(--color-outl-fg-dimmer) line-through":
          props.block.todo === "DONE",
      }}
    >
      <Show
        when={bodyLength > 0}
        fallback={
          <span class="italic text-(--color-outl-fg-dimmer)">
            Empty block
          </span>
        }
      >
        <MarkdownInline
          tokens={tokens}
          variant="inline"
          blockAssets
          embeds={props.embeds}
          pageEmbeds={props.pageEmbeds}
          onRefClick={tap(props.onRefClick)}
          onTagClick={tap(props.onTagClick)}
          onLinkClick={tap(props.onLinkClick)}
        />
      </Show>
    </p>
    {/* `remind::` was invisible in the outline, which is
        why this row exists at all. It is a sibling of
        the `<p>` and not a child: the line is a `<div>`,
        and a `<div>` inside a `<p>` is invalid markup
        that only survives because nothing re-parses it.

        It owns its own type scale — see the component.
        A `text-*` here would be dead anyway, since
        `styles.css`'s unlayered `font-size: inherit`
        outranks it on the button inside. */}
    <BlockProperties
      properties={props.block.properties}
      label="Block properties"
      onOpen={
        props.onOpenProperties
          ? () => props.onOpenProperties?.(props.block.id)
          : undefined
      }
    />
    {/* A block that is *only* `![[page]]` expands that
        page's top-level blocks beneath its title, through
        the same read-only `<EmbeddedSubtree />` the
        desktop uses. Nothing is drawn until the resolve
        answers with rows: the title line is already on
        screen as the embed's own placeholder, so the
        page never enters a load state for it
        (`UX.md` → Never block a keystroke). */}
    <Show when={pageEmbedBlocks()?.length}>
      <EmbeddedSubtree
        nodes={pageEmbedBlocks()!}
        embeds={props.embeds}
        pageEmbeds={props.pageEmbeds}
      />
    </Show>
    </>
  );
}
