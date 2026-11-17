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
  let name: string | null = null;
  for (const tok of tokens) {
    if (tok.kind === "plain" && tok.value.trim() === "") continue;
    if (tok.kind === "pageembed" && name === null) {
      name = tok.value;
      continue;
    }
    return null;
  }
  return name;
}

/**
 * Collect every distinct `![[page]]` name reachable in an outline, so a
 * client can batch-resolve them in one `resolvePageEmbeds` round-trip
 * before rendering.
 *
 * Same walk as {@link import("./index").collectBlockRefHandles}: DFS
 * over `children`, descending the `inner` span of `bold` / `italic` /
 * `strike` / `highlight` tokens, because a page embed can sit inside
 * `**bold**`. Names are de-duplicated by first appearance (`Set`
 * insertion order), so the caller can slice or display them stably,
 * and an empty name is dropped — it can only ever resolve to nothing.
 */
export function collectPageEmbedNames(outline: BlockNode[]): string[] {
  const names = new Set<string>();
  const walkTokens = (tokens: InlineToken[]) => {
    for (const tok of tokens) {
      switch (tok.kind) {
        case "pageembed":
          if (tok.value) names.add(tok.value);
          break;
        case "bold":
        case "italic":
        case "strike":
        case "highlight":
          walkTokens(tok.inner);
          break;
      }
    }
  };
  const walkNodes = (nodes: BlockNode[]) => {
    for (const node of nodes) {
      walkTokens(node.tokens);
      walkNodes(node.children);
    }
  };
  walkNodes(outline);
  return [...names];
}
