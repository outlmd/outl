/**
 * Embed resolution: turning a handle or a page name written inside a
 * block into the content a renderer draws.
 *
 * Two surfaces, deliberately side by side, because they are twins and
 * drifted apart once already. `!((blk-XXXXXX))` embeds one block and
 * its subtree; `![[page]]` embeds a whole page's outline (issue #190).
 * Both are read-only expansions at render time, both are capped at the
 * same depth, and both key their reply by the token's text **as
 * written** rather than by anything resolved — the renderer holds the
 * token, not the slug.
 *
 * Split out of `commands.ts` / `types.ts` under the file-size ratchet,
 * the `api/paste.ts` arrangement: DTO beside its wrappers, re-exported
 * from both so `@outl/shared/api/commands` and `@outl/shared/api/types`
 * stay the one import path each. Listed in the wire-mirror files
 * (`outl-tauri-shared/tests/ts_parser`) — a declaration in a file that
 * list omits is invisible to the pins, never reported.
 */

import { invoke } from "@tauri-apps/api/core";

import type { BlockNode } from "./types";

/** A block resolved from its ref handle (`blk-XXXXXX`) — reply shape of
 *  {@link resolveEmbeds}. Serves both surfaces the handle appears in:
 *  `((…))` inline refs render `text`; `!((…))` embeds render `text` plus
 *  the `children` subtree. Mirrors the Rust `EmbedContent`. */
export interface ResolvedBlock {
  handle: string;
  text: string;
  page_slug: string;
  status: string | null;
  children: BlockNode[];
}

/** A page resolved from the name inside a `![[page]]` embed — reply shape
 *  of {@link resolvePageEmbeds}, keyed by the name **as written in the
 *  token** (not by slug), because that is what the renderer holds. A name
 *  that resolves to no page is absent from the map, exactly like an
 *  unresolvable `((blk-…))` handle. Mirrors the Rust `PageEmbedContent`. */
export interface PageEmbedContent {
  slug: string;
  title: string;
  /** The page's top-level blocks, capped at the same depth
   *  {@link ResolvedBlock.children} is capped at. */
  blocks: BlockNode[];
}

/**
 * Batch-resolve embed handles (`blk-XXXXXX`) to their source content.
 * Returns a map from handle to {@link ResolvedBlock} — the source
 * block's `text`, `page_slug`, `status`, and its `children` subtree
 * (populated for `!((…))` embeds, empty for a leaf).
 */
export function resolveEmbeds(
  handles: string[],
): Promise<Record<string, ResolvedBlock>> {
  return invoke<Record<string, ResolvedBlock>>("resolve_embeds", { handles });
}

/**
 * Batch-resolve page-embed names (`![[page]]`) to their target page's
 * title and top-level blocks — the page-level twin of
 * {@link resolveEmbeds}.
 *
 * Keyed by the name **as written in the token**, not by slug, because
 * that is what the renderer holds. A name that resolves to no page is
 * absent from the map, exactly like an unresolvable `((blk-…))` handle.
 */
export function resolvePageEmbeds(
  names: string[],
): Promise<Record<string, PageEmbedContent>> {
  return invoke<Record<string, PageEmbedContent>>("resolve_page_embeds", {
    names,
  });
}
