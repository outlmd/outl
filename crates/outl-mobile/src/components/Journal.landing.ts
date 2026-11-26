/**
 * Opening a page "at" a backlink's context: the executor behind a tap on
 * the breadcrumb above a backlink row.
 *
 * Its own module because `Journal.tsx` is at its file-size baseline, and
 * because the sequence is a race by construction: two awaits (open, then
 * maybe unfold) sit between the tap and the paint, and the user can tap
 * again or navigate elsewhere in that window.
 *
 * The order is load-bearing:
 *
 * 1. open the page through the same door as the group header (journal
 *    kinds go through `openJournalFor`, never `openPageBySlug`, which
 *    would create a plain page named like the date);
 * 2. resolve the landing in the **opened** outline (`landingTarget`);
 * 3. unfold only when a fold is in the way (`needsUnfold`): the unfold is
 *    an op (`outl_actions::unfold_ancestors`), so it is skipped, not
 *    called and ignored, when nothing is folded;
 * 4. bail if another jump started or the page on screen changed by some
 *    other path while we awaited;
 * 5. paint **once** (`openView`), then scroll and mark on the next frame.
 */
import type { Backlink, PageMeta, PageView } from "@outl/shared/api/types";
import { openJournalFor, openPageBySlug } from "@outl/shared/api/commands";
import { keepOpenedNotices, landingTarget, needsUnfold } from "@outl/shared/outline";

import { landedMarks } from "../lib/landed";

/** The fields of a backlink this executor reads. */
export type LandingLink = Pick<Backlink, "block_id" | "ancestors" | "source_page">;

export interface LandingDeps {
  /** Runs a fetch and resolves `undefined` after surfacing its error
   *  itself (`Journal.tsx`'s `withError`). */
  withError: <T>(fn: () => Promise<T>) => Promise<T | undefined>;
  /** `unfoldAncestors` (the shared command). May reject. */
  unfold: (pageId: string, id: string) => Promise<PageView>;
  /** Show a page the user asked for (resets zoom and selection). */
  openView: (v: PageView) => void;
  /** Id of the page on screen right now. */
  currentPageId: () => string | undefined;
  /** A sentence for the status line. */
  notify: (message: string) => void;
  /** Scroll the block to the centre. Injected for tests. */
  reveal?: (id: string) => void;
  /** Next-frame scheduler. Injected for tests. */
  nextFrame?: (cb: () => void) => void;
}

function revealCentred(id: string) {
  const el = document.querySelector(`[data-block-id="${CSS.escape(id)}"]`);
  // Scrolling is motion too: honour reduced motion like the highlight does.
  const calm = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  el?.scrollIntoView({ block: "center", behavior: calm ? "auto" : "smooth" });
}

export function createBacklinkLanding(deps: LandingDeps) {
  let seq = 0;
  // The group header's door: journals through `openJournalFor`.
  const openPage = (slug: string, kind: PageMeta["kind"]) =>
    deps.withError(() => (kind === "journal" ? openJournalFor(slug) : openPageBySlug(slug)));
  const reveal = deps.reveal ?? revealCentred;
  const nextFrame = deps.nextFrame ?? ((cb) => requestAnimationFrame(() => cb()));

  async function jump(link: LandingLink): Promise<void> {
    const source = link.source_page;
    if (!source) return;
    const mine = ++seq;
    const from = deps.currentPageId();
    const stale = () => mine !== seq || deps.currentPageId() !== from;

    const opened = await openPage(source.slug, source.kind);
    if (!opened || mine !== seq) return;

    const crumbId = link.ancestors.at(-1)?.id;
    const landing = landingTarget(opened.outline, crumbId, link.block_id);
    let shown = opened;
    let hidden = false;
    if (landing && needsUnfold(opened.outline, landing)) {
      try {
        shown = keepOpenedNotices(opened, await deps.unfold(opened.page.id, landing.unfoldFrom));
      } catch {
        // The page is still worth showing, but the block stays folded
        // out of sight: say so rather than lighting an invisible row.
        hidden = true;
      }
    }
    if (stale()) return;

    deps.openView(shown);
    if (hidden) {
      deps.notify(
        `That block is folded on ${opened.page.title} and could not be opened; opened the page instead.`,
      );
      return;
    }
    if (!landing) {
      deps.notify(`That block is no longer on ${opened.page.title}; opened the page instead.`);
      return;
    }
    nextFrame(() => {
      // Re-checked here too: the user can leave the page within the frame.
      if (mine !== seq || deps.currentPageId() !== opened.page.id) return;
      reveal(landing.target);
      landedMarks.mark(landing.target);
    });
  }

  return { jump };
}
