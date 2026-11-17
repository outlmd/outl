/**
 * The two questions a client asks about `![[page]]` before it renders —
 * the page-level twins of `embedOnlyHandle` and
 * `collectBlockRefHandles` in `./index.ts`.
 *
 * Its own file for the reason `late-section.ts` is: `index.ts` is one
 * line under the 600-line ratchet, and these two belong together
 * anyway. Both are re-exported from `@outl/shared/outline`, so a caller
 * imports them from the same place as their block-embed twins.
 */
import type { BlockNode, InlineToken } from "../api/types";
import { collectTokenValues, soleTokenValue } from "./token-walk";

/** The one kind `![[page]]` collects. */
const PAGE_EMBED_KINDS: ReadonlySet<InlineToken["kind"]> = new Set(["pageembed"]);

/**
 * Return the page name when `tokens` is a **single** `![[page]]` embed
 * surrounded only by whitespace `plain` tokens; `null` otherwise. A
 * block that is *only* a page embed expands that page's top-level
 * blocks beneath it (`<EmbeddedSubtree />`); mixed prose keeps the
 * one-line title render, so `null` is returned for it.
 *
 * Twin of {@link import("./index").embedOnlyHandle}, rule for rule:
 * whitespace-only `plain` runs are skipped, and a second embed, a
 * `[[ref]]` (the same name without the `!`), or any other token
 * disqualifies the block.
 *
 * Deliberately **not** generalised into one function over both kinds.
 * A block holding `!((blk-x))` *and* `![[page]]` is mixed prose to
 * both of them, which is the answer each gives on its own; a merged
 * helper would have to return which kind it found, and every caller
 * would then branch on that — two call sites, two kinds, no shared
 * decision.
 */
export function pageEmbedOnlyName(tokens: InlineToken[]): string | null {
  return soleTokenValue(tokens, "pageembed");
}

/**
 * Every distinct `![[page]]` name in an outline, so a client can
 * batch-resolve them in one `resolvePageEmbeds` round-trip.
 */
export function collectPageEmbedNames(outline: BlockNode[]): string[] {
  return collectTokenValues(outline, PAGE_EMBED_KINDS);
}
