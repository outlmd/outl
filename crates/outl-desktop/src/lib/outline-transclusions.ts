/**
 * Resolving what the open page pulls in from elsewhere: `((blk-…))`
 * refs, `!((blk-…))` block embeds and `![[page]]` page embeds.
 *
 * Its own module, like `./outline-paste`, for the reason that one is:
 * the sequence has state of its own (three skip keys) and a failure
 * mode no component test reaches, so it needs a test of its own.
 *
 * ## What this module exists to stop happening again
 *
 * There are **two** `applyView` functions on the desktop. `<AppShell />`
 * has one and `<OutlineView />` has the other, and only the second ever
 * called this. The shell's is every navigation the chrome owns — the
 * sidebar, the picker, Today, a task row, a deep link, and the page the
 * app opens at boot — so a page that was *opened* never asked for its
 * transclusions. Every one of them rendered its unresolved fallback
 * until some later mutation happened to run the other `applyView`.
 *
 * The literal `![[name]]` chip and the un-expanded `((blk-…))` handle
 * are the same pixels as "that page does not exist", so the failure
 * read as a missing feature rather than a missing call.
 *
 * It is also why the block embed appeared to work "when the map was
 * already warm": warm meant an earlier commit had resolved it. A fresh
 * page was never warm, and a page the user only read never became warm.
 *
 * So the trigger is the **outline**, not whichever `applyView` ran.
 * `createTransclusionOps` owns the skip keys that make that safe to fire
 * on every outline change.
 */

import type { BlockNode, PageEmbedContent, ResolvedBlock } from "@outl/shared/api/types";
// Both from the barrel: `package.json` declares `"./outline"` and no
// deeper subpath, so `@outl/shared/outline/page-embed` resolves only
// through the editor's and vitest's path aliases — not through the
// package's own exports map.
import { collectBlockRefHandles, collectPageEmbedNames } from "@outl/shared/outline";

export interface TransclusionDeps {
  /** Slug of the page the outline belongs to; `undefined` before one is open. */
  slug: () => string | undefined;
  /** Batch-resolve `((blk-…))` / `!((blk-…))` handles. */
  resolveEmbeds: (handles: string[]) => Promise<Record<string, ResolvedBlock>>;
  /** Batch-resolve `![[page]]` names. */
  resolvePageEmbeds: (
    names: string[],
  ) => Promise<Record<string, PageEmbedContent>>;
  /** Store the block-ref map (merged into what is there). */
  setEmbeds: (map: Record<string, ResolvedBlock>) => void;
  /** Store the page-embed map (merged into what is there). */
  setPageEmbeds: (map: Record<string, PageEmbedContent>) => void;
}

export interface TransclusionOps {
  /**
   * Collect what this outline pulls in and batch-resolve each kind in
   * one round-trip.
   *
   * Both commands rebuild the workspace index off disk, so each is
   * O(workspace), and this fires on every outline change — a commit
   * included. Neither kind changes per keystroke, so a kind is skipped
   * when the page and its own key both match the last resolve. That
   * keeps the scan off the commit path (the rule the backlinks fetch
   * follows), while a newly typed ref or embed (its key grows) and a
   * navigation (the slug changes) still resolve. The slug check is also
   * what refreshes source text edited on another page.
   *
   * **One key per kind, deliberately**: a newly typed `[[ref]]` must not
   * invalidate the page-embed answer, and vice versa.
   *
   * A rejected resolve leaves the previous map in place rather than
   * blanking it — the rows on screen are still the best answer anyone
   * has, and clearing them would read as "this page lost its content".
   */
  resolve: (outline: BlockNode[], scope?: TransclusionScope) => void;
}

/** What is being resolved. Each scope keeps its own skip keys, so the
 *  page outline and the "Linked from" rows cannot skip each other. */
export type TransclusionScope = "page" | "backlinks";

interface ScopeState {
  slug: string;
  refKey: string;
  pageEmbedKey: string;
}

export function createTransclusionOps(
  deps: TransclusionDeps,
): TransclusionOps {
  const scopes: Record<TransclusionScope, ScopeState> = {
    page: { slug: "", refKey: "", pageEmbedKey: "" },
    backlinks: { slug: "", refKey: "", pageEmbedKey: "" },
  };

  function resolve(outline: BlockNode[], scope: TransclusionScope = "page") {
    const last = scopes[scope];
    const slug = deps.slug() ?? "";
    const navigated = slug !== last.slug;
    last.slug = slug;

    const handles = collectBlockRefHandles(outline);
    const refKey = handles.join("\n");
    if (navigated || refKey !== last.refKey) {
      last.refKey = refKey;
      if (handles.length > 0) {
        void deps
          .resolveEmbeds(handles)
          .then(deps.setEmbeds)
          .catch(() => {});
      }
    }

    const names = collectPageEmbedNames(outline);
    const pageKey = names.join("\n");
    if (navigated || pageKey !== last.pageEmbedKey) {
      last.pageEmbedKey = pageKey;
      if (names.length > 0) {
        void deps
          .resolvePageEmbeds(names)
          .then(deps.setPageEmbeds)
          .catch(() => {});
      }
    }
  }

  return { resolve };
}
