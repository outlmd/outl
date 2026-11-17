/**
 * Resolving what the page on screen pulls in from elsewhere —
 * `((blk-…))` refs, `!((blk-…))` block embeds and `![[page]]` page
 * embeds.
 *
 * `MarkdownInline` renders a resolved ref as the source block's text and
 * an unresolved one as the raw handle. It decides that purely from its
 * `embeds` prop, so a client that never fills it shows
 * `((blk-01HXYZ…))` on every ref — which is the whole of
 * [issue #16](https://github.com/outlmd/outl/issues/16). Nothing was
 * missing from the backend or the renderer; this client just never
 * called `resolveEmbeds`.
 *
 * Its own module rather than two closures in `Journal.tsx`, for the
 * reason the rest of the `Journal.*.ts` siblings exist: the skip rule
 * below is a decision with a failure mode, and a decision with a
 * failure mode deserves a test that does not need a WKWebView.
 */

import { createSignal, type Accessor } from "solid-js";

import {
  collectBlockRefHandles,
  collectPageEmbedNames,
} from "@outl/shared/outline";
import { resolveEmbeds, resolvePageEmbeds } from "@outl/shared/api/commands";
import type { BlockNode } from "@outl/shared/api/types";
import type { EmbedMap, PageEmbedMap } from "@outl/shared/markdown";

export interface EmbedResolver {
  /**
   * The resolved map, for `<BlockRow embeds={…} />`. Owned here rather
   * than by the caller: `Journal.tsx` is three times its size ceiling,
   * so state whose only reader is one prop does not need to live there.
   */
  embeds: Accessor<EmbedMap>;
  /** The same, for `![[page]]` embeds, keyed by the name as written. */
  pageEmbeds: Accessor<PageEmbedMap>;
  /**
   * Resolve every handle and page name in `outline`, unless the last
   * resolve already answered that exact question. The two kinds are
   * asked independently: a newly typed `[[ref]]` must not invalidate
   * the page-embed answer, or vice versa.
   */
  resolve: (outline: BlockNode[]) => void;
}

/**
 * `resolveEmbeds` rebuilds the workspace index off disk, so it is
 * O(workspace) — and the caller runs on every mutation, not just on
 * navigation.
 *
 * Refs do not change per keystroke, so a resolve is skipped when the
 * page and the handle set both match the previous one. A newly typed ref
 * grows the handle set and a navigation changes the slug, so both still
 * resolve; the slug check is also what refreshes source text that was
 * edited on another page.
 *
 * This mirrors the desktop's `resolveTransclusions`
 * (`outl-desktop/src/components/OutlineView.tsx`) deliberately — the two
 * clients must not disagree about when a ref is stale. It is duplicated
 * rather than shared because the desktop's copy reads and writes its
 * store directly; promoting one shape before the other has a seam would
 * freeze the wrong contract (`outl-frontend-shared/CLAUDE.md`).
 *
 * A failed resolve is swallowed on purpose. The fallback is the raw
 * handle, which is what the user saw before this existed, and a toast
 * for it would fire on every commit while a peer is mid-sync.
 *
 * **Known gap, shared with the desktop.** A ref whose *source block is
 * on the page being rendered* keeps its cached text until the user
 * navigates: editing the source changes neither the slug nor the
 * handle set, so the skip holds. Closing it needs a signal the backend
 * does not send today (the resolved text's revision), and inventing
 * one here would put it on one client only. Tracked with the rest of
 * [#16](https://github.com/outlmd/outl/issues/16).
 */
export function createEmbedResolver(slug: () => string): EmbedResolver {
  const blocks = createMapResolver(slug, collectBlockRefHandles, resolveEmbeds);
  const pages = createMapResolver(
    slug,
    collectPageEmbedNames,
    resolvePageEmbeds,
  );
  return {
    embeds: blocks.map,
    pageEmbeds: pages.map,
    resolve(outline) {
      blocks.resolve(outline);
      pages.resolve(outline);
    },
  };
}

/**
 * One kind's resolve: collect the keys out of the outline, ask the
 * backend, hold the reply.
 *
 * Generic over the kind rather than written twice, because the skip
 * rule and the race guard below are the part with a failure mode, and
 * a second copy of them is a second thing to get wrong. Each kind gets
 * its **own** key and its own generation counter: sharing them would
 * let a resolve that skipped one kind cancel that kind's in-flight
 * reply, and since the key is already recorded, nothing would re-ask —
 * issue #16 again, by a different door.
 */
function createMapResolver<T>(
  slug: () => string,
  collect: (outline: BlockNode[]) => string[],
  fetch: (keys: string[]) => Promise<Record<string, T>>,
): { map: Accessor<Record<string, T>>; resolve: (o: BlockNode[]) => void } {
  const [map, setMap] = createSignal<Record<string, T>>({});
  let lastSlug = "";
  let lastKey = "";
  // Which resolve is the current one. `resolve_embeds` rebuilds the
  // index on a blocking pool and takes no workspace lock, so two calls
  // genuinely race and can land in either order. Without this the
  // older map wins — and because the skip key already names the newer
  // page, nothing re-resolves: the new page's refs stay raw handles
  // until the user edits the page. That is issue #16's own symptom,
  // made permanent. Every other async path in this client guards the
  // same way (`queryToken`, `searchToken`, `reloadGen`).
  let generation = 0;

  return {
    map,
    resolve(outline) {
      const page = slug();
      const keys = collect(outline);
      const key = keys.join("\n");
      if (page === lastSlug && key === lastKey) return;
      lastSlug = page;
      lastKey = key;
      const token = ++generation;
      // Nothing to ask about. The key is recorded above first, so a
      // page whose last ref was just deleted does not re-resolve
      // forever; the token still moves, so an in-flight resolve from
      // the previous page cannot land on top of this empty map.
      if (keys.length === 0) {
        setMap({});
        return;
      }
      void fetch(keys)
        .then((reply) => {
          if (token !== generation) return;
          setMap(reply);
        })
        .catch(() => {});
    },
  };
}
