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
  createEffect(
    on(
      () => appState.page?.id,
      (pageId) => {
        if (!pageId) {
          setAppState("selectedBlockId", null);
          setAppState("selectedBacklinkBlockId", null);
          return;
        }
        // A new page lands the cursor on the outline (never a stale
        // backlink) and drops any zoom held on the previous page.
        setAppState("selectedBacklinkBlockId", null);
        setAppState("focusBlockId", null);
        const outline = appState.outline;
        if (outline.length === 0) {
          setAppState("selectedBlockId", null);
          return;
        }
        const current = appState.selectedBlockId;
        if (current === null || !containsBlock(outline, current)) {
          setAppState("selectedBlockId", outline[0].id);
        }
      },
    ),
  );
}
