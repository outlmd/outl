/**
 * The backlinks card's writes: flipping its direction, and asking for
 * its rows again after a commit through one of them.
 *
 * A sibling module rather than two more closures in `Journal.tsx`, for
 * the reason `Journal.remind.ts` is one — and because both writes share
 * the rule that is easy to lose: **the rows are swapped in place.**
 *
 * `createResource`'s own `refetch` flips `loading`, and `Journal` hands
 * `undefined` down while that is true (so a navigation cannot leave the
 * previous page's rows under the new page). A refetch would therefore
 * unmount the card the user just typed into for the length of an
 * `O(blocks-in-workspace)` scan, which is the one thing a commit must
 * not do to the section it commits from. `mutate` keeps the rows up
 * until the fresh reply replaces them.
 *
 * Nothing here decides what a backlink is (`outl_actions::backlinks`),
 * what order they come in (`outl_actions::sort_backlinks`), or what a
 * commit may touch (`@outl/shared/backlink-edit`).
 */

import { pageBacklinks, setBacklinksOrder } from "@outl/shared/api/commands";
import type {
  BacklinksOrder,
  PageBacklinks,
  PageView,
} from "@outl/shared/api/types";
import {
  backlinkCommitPlan,
  backlinkCreatedElsewhere,
} from "@outl/shared/backlink-edit";

import { haptic } from "../lib/haptics";
import type { BacklinkCommit } from "./BacklinksSection.row-ops";

export interface BacklinkCardDeps {
  /** Accessor, never a value: a destructured prop freezes in Solid. */
  view: () => PageView | null;
  /** The direction the card is sorted in right now. */
  order: () => BacklinksOrder;
  /** Swap the card's rows in place (the resource's `mutate`). */
  mutate: (next: PageBacklinks) => void;
  /**
   * Install a reply as the open page's view. Reached only when
   * `backlinkCommitPlan` says the mutated page *is* the page on screen.
   */
  applyView: (view: PageView) => void;
  /** `null` clears a stale refusal, the way `withError` does before it
   *  runs — otherwise one refusal outlives every later success. */
  onError: (message: string | null) => void;
  /**
   * Something landed that the user has to be told about, but which did
   * not fail. Separate from `onError` because that one buzzes a warning
   * haptic, and a successful create is not a warning.
   */
  onNotice: (message: string) => void;
}

/**
 * The card's two message callbacks, wired to the one toast this client
 * has. A card message must not inherit the Retry pinned by an unrelated
 * failed outline save: its button would re-run that save under a message
 * about something else. Kept here, not inline in `Journal.tsx`, so the
 * order (retry cleared, then the message) is a behaviour a test can run.
 */
export function backlinkToast(
  clearRetry: () => void,
  show: (message: string | null) => void,
): Pick<BacklinkCardDeps, "onError" | "onNotice"> {
  const say = (message: string | null) => {
    clearRetry();
    show(message);
  };
  return { onError: say, onNotice: say };
}

export function createBacklinkCard(deps: BacklinkCardDeps) {
  /** A refusal the user has to read, with the buzz `withError` gives. */
  function refused(message: string) {
    haptic("warning");
    deps.onError(message);
  }

  /**
   * `true` when fresh rows are in. The answer matters because a caller
   * may have something to say afterwards, and `onError(null)` below
   * lands in the **same signal** as a notice on this client — so
   * nothing may be said before this returns.
   */
  async function reload(
    run: (slug: string) => Promise<PageBacklinks>,
  ): Promise<boolean> {
    const slug = deps.view()?.page.slug;
    if (!slug) return false;
    deps.onError(null);
    try {
      deps.mutate(await run(slug));
      return true;
    } catch (e) {
      // Stale rows with nothing said is the defect, not the fix: the
      // write landed, so the card is now describing the graph as it was
      // before it.
      refused(e instanceof Error ? e.message : String(e));
      return false;
    }
  }

  return {
    refused,
    /** Flip newest ⇄ oldest (issue #142). The backend persists the
     *  preference and answers with the re-sorted rows. */
    async toggleOrder(): Promise<void> {
      haptic("light");
      const next = deps.order() === "newest" ? "oldest" : "newest";
      // Awaited, not fired: the section holds its one-write guard for
      // exactly as long as this promise, so a `void` here releases the
      // guard while the re-ask is still in flight.
      await reload((slug) => setBacklinksOrder(next, slug));
    },
    /**
     * A mutation through one of the card's rows landed, on the page the
     * citing block lives on.
     *
     * **This is where the plan runs**, because this is the only place
     * holding both page ids — the mutated one, reported by the section,
     * and the open one, which is `view()`. The section reports the fact
     * and decides nothing, which is why it does not take `openPageId`.
     * The plan is read *before* `applyView` replaces `view()`.
     *
     * The re-ask is unconditional: a mutation on another page can
     * change what cites this one, and working out when it cannot is a
     * second opinion about the backlink index that would drift.
     *
     * Per commit, never per keystroke — the scan is the expensive read
     * this feature sits on top of (invariant 15). **The returned
     * promise is the section's cue to close its editor**, so a
     * committed row never renders its pre-edit text in the gap.
     */
    async committed(commit: BacklinkCommit) {
      const openPageId = deps.view()?.page.id;
      const plan = backlinkCommitPlan({
        sourcePageId: commit.sourcePageId,
        openPageId,
      });
      if (plan.applyView) deps.applyView(commit.view);
      const refreshed = plan.refetchBacklinks
        ? await reload(pageBacklinks)
        : true;
      // A created block cites nothing, so no row here will render it,
      // and this sentence is the entire affordance.
      //
      // **It has to be said last.** `onNotice` and `onError` land in
      // one signal on this client (`Journal.tsx` wires both to
      // `setError`), and `reload` clears a stale refusal *before* its
      // first `await` — so saying this earlier wrote the notice into a
      // channel that was cleared in the same tick, and the toast never
      // rendered. A create that reports nothing is the state shipping
      // this was meant to end.
      //
      // Skipped when the re-ask refused, because that message is the
      // more urgent one: the rows on screen are now stale, and the
      // user has to know before trusting them.
      //
      // Whether there is a sentence at all is the owner's call: on a page
      // citing itself it is `null`, the block being in the outline below.
      // `openPageId` was read before `applyView` could replace `view()`.
      if (commit.created && refreshed) {
        const notice = backlinkCreatedElsewhere(
          commit.created.page,
          commit.created.side,
          openPageId,
          commit.created.depth,
        );
        if (notice !== null) deps.onNotice(notice);
      }
    },
  };
}
