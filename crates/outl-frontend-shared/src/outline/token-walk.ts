/**
 * The two token walks a transclusion needs, each written once.
 *
 * `((blk-…))` / `!((blk-…))` and `![[page]]` ask the same two questions
 * of an outline — *is this block nothing but one of these?* and *which
 * ones does this page pull in?* — and the answers differed only in
 * which `kind` counted. So there were two copies of each, and the
 * copied part is the subtle part: the whitespace-only skip, the
 * second-match rejection, and the list of wrapper tokens to descend.
 *
 * That last one is the drift that would have reached a user. Add a
 * wrapper kind and update one copy, and a `((ref))` inside it resolves
 * while a `![[page]]` inside it silently does not, on the same screen.
 *
 * Its own module rather than `index.ts` so `page-embed.ts` can import
 * it without an import cycle, and so `index.ts` stays under the
 * file-size ratchet it was split for.
 */

import type { BlockNode, InlineToken } from "../api/types";

/** Token kinds whose `inner` span is re-tokenized, so a ref can live
 *  inside one. The single copy of this list. */
const WRAPPERS: ReadonlySet<InlineToken["kind"]> = new Set([
  "bold",
  "italic",
  "strike",
  "highlight",
]);

/**
 * `value` when the variant carries one. The union's wrapper variants
 * (`bold` and friends) carry `inner` instead, so this is the one place
 * that narrows, rather than a cast at each read.
 */
function valueOf(tok: InlineToken): string | undefined {
  return "value" in tok ? tok.value : undefined;
}

/**
 * The value of the one token of `kind` in `tokens`, when the block is
 * *nothing but* that token.
 *
 * Whitespace-only `plain` runs are ignored, so a trailing space does
 * not disqualify a block. Anything else — prose around it, a second
 * token of the same kind, a token of a different kind — answers `null`,
 * because a block that says more than "show me this" is not a block
 * whose whole purpose is the transclusion.
 */
export function soleTokenValue(
  tokens: InlineToken[],
  kind: InlineToken["kind"],
): string | null {
  let found: string | null = null;
  for (const tok of tokens) {
    if (tok.kind === "plain" && tok.value.trim() === "") continue;
    if (tok.kind === kind && found === null) {
      found = valueOf(tok) ?? "";
      continue;
    }
    return null;
  }
  return found;
}

/**
 * Every distinct value of the given token `kinds` reachable in an
 * outline, so a client can batch-resolve them in one round-trip before
 * rendering.
 *
 * DFS over `children`, descending the `inner` span of every
 * {@link WRAPPERS} kind. De-duplicated by first appearance (`Set`
 * insertion order), so a caller can slice or display the result stably.
 * An empty value is dropped: it can only ever resolve to nothing.
 */
export function collectTokenValues(
  outline: BlockNode[],
  kinds: ReadonlySet<InlineToken["kind"]>,
): string[] {
  const values = new Set<string>();
  const walkTokens = (tokens: InlineToken[]) => {
    for (const tok of tokens) {
      if (kinds.has(tok.kind)) {
        const value = valueOf(tok);
        if (value) values.add(value);
      } else if (WRAPPERS.has(tok.kind) && "inner" in tok) {
        walkTokens(tok.inner);
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
  return [...values];
}
