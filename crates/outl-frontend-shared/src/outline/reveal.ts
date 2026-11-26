/**
 * Where a client lands when it opens a page "at" a backlink's context,
 * and whether a fold is in the way.
 *
 * Pure walks over the **open page's outline**. The unfold itself is
 * `outl_actions::unfold_ancestors` (the `unfoldAncestors` command), which
 * writes only the blocks that are actually folded; {@link needsUnfold}
 * exists so a client skips that IPC when nothing is.
 *
 * Desktop and mobile both resolve the cascade here, so the two cannot
 * disagree about which block a breadcrumb click lands on.
 */
import type { BlockNode, PageView } from "../api/types";

/** A resolved landing. */
export interface Landing {
  /** The block to select and centre. */
  target: string;
  /** The block whose ancestors to unfold: the citing block when it is on
   *  the page, so the reference stays visible under the target, else the
   *  target itself. */
  unfoldFrom: string;
}

/** The chain from the outline root down to `id`, inclusive, or `null`
 *  when `id` is not in the outline. One DFS. */
function pathTo(blocks: BlockNode[], id: string): BlockNode[] | null {
  for (const b of blocks) {
    if (b.id === id) return [b];
    const rest = pathTo(b.children, id);
    if (rest) return [b, ...rest];
  }
  return null;
}

/**
 * Resolve where a backlink's breadcrumb lands, in the open page's outline:
 *
 * 1. `crumbId` (the immediate parent the row shipped), if it is there;
 * 2. else the parent of `citingId` in the outline (the crumb's block was
 *    moved or deleted, but the citing block still has a parent);
 * 3. else the citing block itself when it is on the page at the top
 *    level (it has no parent block, but it is not stale: the block the
 *    user asked about is right there);
 * 4. else `null`: nothing to select, so the client opens the page and
 *    says so rather than going quiet.
 */
export function landingTarget(
  outline: BlockNode[],
  crumbId: string | null | undefined,
  citingId: string,
): Landing | null {
  const citingPath = pathTo(outline, citingId);
  let target: string;
  if (crumbId && pathTo(outline, crumbId)) target = crumbId;
  else if (citingPath && citingPath.length > 1) target = citingPath[citingPath.length - 2].id;
  else if (citingPath) target = citingId;
  else return null;
  return { target, unfoldFrom: citingPath ? citingId : target };
}

/**
 * Whether any block on the way to `landing.unfoldFrom` is folded, so the
 * landing would be hidden. The blocks that matter are `unfoldFrom`'s
 * ancestors (its own fold hides only its children, which nobody asked
 * for); when `unfoldFrom` is a descendant of `target`, `target` is one of
 * those ancestors, so its own fold is counted too.
 *
 * `false` means the client skips the unfold call entirely.
 */
export function needsUnfold(outline: BlockNode[], landing: Landing): boolean {
  const path = pathTo(outline, landing.unfoldFrom);
  if (!path) return false;
  return path.slice(0, -1).some((b) => b.collapsed);
}

/**
 * The view to paint after an unfold: the unfold reply's outline, with the
 * notices of the **opened** view. `unfoldAncestors` returns a plain page
 * view that never ran the ahead-of-log check, and on a jump from another
 * page nothing else can recover the banner the opened page owes the user
 * (`stickyAheadOfLog` only carries it across a same-page reply).
 */
export function keepOpenedNotices(opened: PageView, unfolded: PageView): PageView {
  return {
    ...unfolded,
    md_ahead_of_log: opened.md_ahead_of_log,
    md_ahead_of_log_checked: opened.md_ahead_of_log_checked,
  };
}
