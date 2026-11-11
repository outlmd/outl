/**
 * Resolving `((blk-…))` refs and `!((blk-…))` embeds for the page on
 * screen.
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

import { collectBlockRefHandles } from "@outl/shared/outline";
import { resolveEmbeds } from "@outl/shared/api/commands";
import type { BlockNode } from "@outl/shared/api/types";
import type { EmbedMap } from "@outl/shared/markdown";

export interface EmbedResolver {
  /**
   * The resolved map, for `<BlockRow embeds={…} />`. Owned here rather
   * than by the caller: `Journal.tsx` is three times its size ceiling,
   * so state whose only reader is one prop does not need to live there.
   */
  embeds: Accessor<EmbedMap>;
  /**
   * Resolve every handle in `outline`, unless the last resolve already
   * answered this exact question.
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
 * This mirrors the desktop's `resolvePageEmbeds`
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
  const [embeds, setEmbeds] = createSignal<EmbedMap>({});
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
    embeds,
    resolve(outline) {
      const page = slug();
      const handles = collectBlockRefHandles(outline);
      const key = handles.join("\n");
      if (page === lastSlug && key === lastKey) return;
      lastSlug = page;
      lastKey = key;
      const token = ++generation;
      // Nothing to ask about. The key is recorded above first, so a
      // page whose last ref was just deleted does not re-resolve
      // forever; the token still moves, so an in-flight resolve from
      // the previous page cannot land on top of this empty map.
      if (handles.length === 0) {
        setEmbeds({});
        return;
      }
      void resolveEmbeds(handles)
        .then((map) => {
          if (token !== generation) return;
          setEmbeds(map);
        })
        .catch(() => {});
    },
  };
}
