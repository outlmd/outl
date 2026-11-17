import { JSX, Show } from "solid-js";

import type { PageEmbedContent } from "../api/types";

/**
 * The `![[page]]` render surface — the page-level twin of the
 * `!((blk-X))` embed.
 *
 * Two halves, deliberately in one small file rather than inside
 * `MarkdownInline.tsx`:
 *
 * - {@link PageEmbedMap}, the resolved-content map, twin of `EmbedMap`.
 * - {@link PageEmbedTitle}, the inline token's own render.
 *
 * The *expansion* of the page's blocks is not here: it is
 * `<EmbeddedSubtree />`, the same component the block embed expands
 * through, so the two kinds of transclusion cannot start looking
 * different. What the client decides is **whether** to expand, via
 * `pageEmbedOnlyName` (`@outl/shared/outline`).
 */

/**
 * Resolved page content keyed by the name **as written in the token**
 * (`![[Projeto X]]` → `"Projeto X"`), not by slug — the renderer holds
 * `token.value` and nothing else. Reply shape of `resolvePageEmbeds`.
 *
 * A name that resolves to no page is **absent** from the map, exactly
 * like an unresolvable `((blk-…))` handle, so `undefined` covers both
 * "no such page" and "the resolve has not answered yet". Both render
 * the literal chip, which is the honest answer in either case: the
 * user wrote that name, and it is on screen.
 */
export type PageEmbedMap = Record<string, PageEmbedContent>;

interface PageEmbedTitleProps {
  /** The page name, verbatim from the token. */
  name: string;
  /** Resolved content, or `undefined` while unresolved / nonexistent. */
  content?: PageEmbedContent;
  /**
   * Follow the embed to its page. Wired to the same `onRefClick` a
   * `[[ref]]` uses, because a page embed **is** a page reference (it
   * carries a backlink) — so the argument is the name inside the
   * brackets, not the slug, and a client needs no second handler.
   * Omitted (read-only contexts) leaves the title inert text rather
   * than a dead control.
   */
  onOpenPage?: (target: string) => void;
}

/**
 * The embedding block's own line: the target page's title when it
 * resolved, the literal `![[name]]` when it did not.
 *
 * One visual style for both render variants, unlike `ref` / `tag`. A
 * page embed's title is a heading for the rows underneath it, not a
 * chip in a sentence, and the compact `"pill"` contexts (backlinks,
 * an embedded subtree) show it with no expansion at all — a second
 * style there would differ from the main outline for no reason the
 * user could name.
 *
 * **Both states are a plain inline `<span>` of text, and that is a
 * layout decision, not a styling one.** The two swap *after the first
 * paint*, in the block the user may be typing in: the resolve lands a
 * round-trip late, so whatever is drawn before it must occupy the same
 * line box as what replaces it, or every row below the caret jumps when
 * a page resolves. An `inline-flex` wrapper with a 14px `<PageIcon />`
 * — the first version of this — is an atomic inline box whose height is
 * its own content's, so it can outgrow the line's strut and do exactly
 * that. A non-replaced inline span cannot: padding and underlines add
 * no height, and the smaller `text-[13px]` of the unresolved chip stays
 * inside the inherited strut. The weight (`font-medium`) is what tells
 * a transcluded page from a plain `[[ref]]`, because weight is free.
 */
export function PageEmbedTitle(props: PageEmbedTitleProps): JSX.Element {
  const follow = (e: Event) => {
    if (!props.onOpenPage) return;
    e.stopPropagation();
    props.onOpenPage(props.name);
  };
  return (
    <Show
      when={props.content}
      fallback={
        // Neutral, matching the orphan `((blk-…))` chip: a name with no
        // page behind it is dangling, not accented. Never blank — the
        // literal is what tells the user which name did not resolve.
        <span
          title={`![[${props.name}]]`}
          class="rounded bg-(--color-outl-border)/30 px-1 font-mono text-[13px] text-(--color-outl-fg-dim)"
        >
          {`![[${props.name}]]`}
        </span>
      }
    >
      {(content) => (
        <>
          {/* No marker glyph at all, and that is the restraint the
              rail buys. The `↳` that used to sit here meant "this
              lives elsewhere", which `EmbeddedSubtree`'s rail now says
              once for the whole region; repeating it on the title said
              it twice.

              A page icon was tried in its place and reverted:
              `a_page_embed_title_is_a_plain_inline_span` refuses any
              atomic inline box here, because an `<svg>` carries its own
              height into the line box and this line *swaps* a round
              trip after first paint, inside a block the user may be
              typing in. The weight and the hue are what tell a
              transcluded page from a plain `[[ref]]`, and neither adds
              a pixel. */}
          <span
            role={props.onOpenPage ? "button" : undefined}
            tabindex={props.onOpenPage ? 0 : undefined}
            title={content().slug}
            onClick={follow}
            onKeyDown={(e) => {
              if (e.key === "Enter" || e.key === " ") {
                e.preventDefault();
                follow(e);
              }
            }}
            class="font-medium text-(--color-outl-ref-link-fg) underline decoration-(--color-outl-ref-link-fg)/40 underline-offset-2 hover:decoration-(--color-outl-ref-link-fg)"
            classList={{ "cursor-pointer": !!props.onOpenPage }}
          >
            {content().title}
          </span>
        </>
      )}
    </Show>
  );
}
