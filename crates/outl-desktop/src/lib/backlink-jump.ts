/**
 * Open a backlink's source page **at its context**: the block its
 * breadcrumb names, selected, centred and briefly lit, with any fold in
 * the way opened.
 *
 * Its own module rather than a function in `outline-navigation.ts`,
 * which promises it "only navigates, never writes": opening a fold is an
 * `Op::SetCollapsed` (invariant 7), so this one writes, and only when
 * something on the path is actually folded.
 *
 * The cost is bounded (invariant 15): one `openRef`, one DFS over the
 * opened outline, and at most one `unfoldAncestors` round-trip, skipped
 * when `needsUnfold` finds nothing folded. The view is painted **once**,
 * after every await, so the outline never flashes through the folded
 * state and back.
 */

import { openRef, unfoldAncestors } from "@outl/shared/api/commands";
import type { Backlink, PageView } from "@outl/shared/api/types";
import { keepOpenedNotices, landingTarget, needsUnfold } from "@outl/shared/outline";

import { revealBlock } from "./keyboard-reach";
import { applyView } from "./outline-navigation";
import { resetForNavigation } from "./page-change";
import { appState, landedMarks, setAppState } from "./store";

/** Bumped by every jump. A jump that finds a newer number on return was
 *  overtaken, and must not paint over the one that overtook it. */
let latestJump = 0;

function report(message: string): void {
  setAppState("lastError", message);
}

/**
 * Jump from a backlink row to its source page, landing on the block the
 * row's breadcrumb names (else the citing block's parent).
 *
 * Never throws: a failure lands on the status line, and a jump that was
 * overtaken (a newer jump, or the user opening another page meanwhile)
 * simply stops.
 */
export async function openBacklinkContext(link: Backlink): Promise<void> {
  const slug = link.source_page?.slug;
  if (!slug) {
    report(
      "This reference has no page of its own, so there is nowhere to open it",
    );
    return;
  }
  const jump = ++latestJump;
  const startedOn = appState.page?.id;
  const overtaken = () => jump !== latestJump || appState.page?.id !== startedOn;

  try {
    let view: PageView = await openRef(slug);
    if (overtaken()) return;

    const landing = landingTarget(
      view.outline,
      link.ancestors.at(-1)?.id,
      link.block_id,
    );
    let hidden = false;
    if (landing && needsUnfold(view.outline, landing)) {
      try {
        view = keepOpenedNotices(
          view,
          await unfoldAncestors(view.page.id, landing.unfoldFrom),
        );
      } catch (e) {
        // The page is open and the block is on it, but it stays folded
        // out of sight: say so, and do not select a row nobody can see.
        hidden = true;
        report(
          `${e instanceof Error ? e.message : String(e)} (the block is folded on ${view.page.title}, so the page opened without selecting it)`,
        );
      }
      if (overtaken()) return;
    }

    resetForNavigation();
    applyView(view);

    if (!landing) {
      report(
        `That block is no longer on ${view.page.title}; opened the page instead.`,
      );
      return;
    }
    if (hidden) return;
    setAppState("selectedBlockId", landing.target);
    landedMarks.mark(landing.target);
    revealBlock(landing.target, "center");
  } catch (e) {
    report(e instanceof Error ? e.message : String(e));
  }
}
