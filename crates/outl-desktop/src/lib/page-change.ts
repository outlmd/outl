import { createEffect, on } from "solid-js";

import type { BlockNode } from "@outl/shared/api/types";

import { appState, setAppState } from "./store";

function containsBlock(blocks: BlockNode[], id: string): boolean {
  for (const b of blocks) {
    if (b.id === id) return true;
    if (b.children.length > 0 && containsBlock(b.children, id)) return true;
  }
  return false;
}

// `setOutline` reconciles in place, so the array identity never changes;
// walking the ids is what subscribes the effect to an outline replacement.
function outlineIds(blocks: BlockNode[], out: string[] = []): string[] {
  for (const b of blocks) {
    out.push(b.id);
    outlineIds(b.children, out);
  }
  return out;
}

/**
 * Reset per-page view state when the *page id* changes, and only then.
 *
 * Keyed with `on(() => appState.page?.id)`: `applyView` replaces the page
 * object on every commit, and `setOutline` replaces the outline, so a bare
 * effect reading them re-ran after each edit and dropped the block zoom
 * (`focusBlockId`). `on` makes the callback untracked, so `outline` and
 * `selectedBlockId` are read without becoming dependencies — which also
 * keeps j/k selection changes from re-triggering a snap to the first block.
 *
 * Not deferred: the first load still selects the first block.
 */
export function installPageChangeEffect(): void {
  // `applyView` writes the page and the outline as separate store writes, so
  // the effect also tracks the outline: the stale selection a page-first
  // write leaves behind is fixed when the outline lands. Navigation resets
  // only run when the page id differs from the last one seen.
  let lastPageId: string | null | undefined = undefined;
  createEffect(
    on([() => appState.page?.id, () => outlineIds(appState.outline)], ([pageId]) => {
      const outline = appState.outline;
      const current = appState.selectedBlockId;
      const snap = () => {
        if (outline.length === 0) {
          setAppState("selectedBlockId", null);
        } else if (current === null || !containsBlock(outline, current)) {
          setAppState("selectedBlockId", outline[0].id);
        }
      };
      const samePage = lastPageId !== undefined && (pageId ?? null) === lastPageId;
      lastPageId = pageId ?? null;
      if (samePage) {
        // Commit or late outline: keep zoom and backlink selection, only
        // repair a selection that no longer exists.
        if (pageId && current !== null && !containsBlock(outline, current)) snap();
        return;
      }
      if (!pageId) {
        setAppState("selectedBlockId", null);
        setAppState("selectedBacklinkBlockId", null);
        return;
      }
      // A new page lands the cursor on the outline (never a stale
      // backlink) and drops any zoom held on the previous page.
      setAppState("selectedBacklinkBlockId", null);
      setAppState("focusBlockId", null);
      snap();
    }),
  );
}
