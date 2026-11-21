/**
 * What does editing a backlink row mean, and what has to refresh once
 * the edit commits?
 *
 * A backlink is a block that lives on **another** page and cites the one
 * being read. Editing it in place means the mutation is addressed to the
 * source page, not the open one, so two questions come up that neither
 * client should answer on its own: which `(page, block)` pair a gesture
 * targets ({@link backlinkEditTarget}), and what the reply of a
 * cross-page mutation is allowed to touch ({@link backlinkCommitPlan}).
 * This module is the **single owner** of both, so the desktop and the
 * mobile cannot disagree about them. A client is left with only what it
 * alone knows: how its row enters Insert and which command it invokes.
 *
 * **The plan is per commit, never per keystroke.**
 * Refreshing the backlinks section means calling `pageBacklinks`, which
 * is an `O(blocks-in-workspace)` scan (invariant 15).
 * Run once per keystroke it would be the expensive read behind the
 * cheapest gesture there is, and the typing it was meant to serve is
 * what would stall.
 * The edit buffer already holds the text the user is looking at, so
 * nothing needs refreshing until the commit lands.
 *
 * **An orphan backlink has nothing to edit.**
 * `source_page` is `null` only when the citing block lives outside any
 * page, which is what legacy and migrated workspaces hold.
 * There is no page to send a mutation to, and guessing one would write
 * to the wrong document, so the row stays read-only.
 */

import type { Backlink, PageMeta } from "../api/types";

/** The `(page, block)` pair an edit of a backlink row is addressed to. */
export interface BacklinkEditTarget {
  /** Stringified NodeId of the page the citing block lives on. */
  pageId: string;
  blockId: string;
}

/** What to do with the reply of a mutation committed from a backlink row. */
export interface BacklinkCommitPlan {
  /** Install the reply as the open page's view. */
  applyView: boolean;
  /** Re-ask the backlinks section for its rows. */
  refetchBacklinks: boolean;
}

/**
 * Which block does editing this row mutate, and on which page?
 * `null` when the row has no page to address (an orphan), which is the
 * client's cue to leave it read-only.
 */
export function backlinkEditTarget(link: Backlink): BacklinkEditTarget | null {
  if (link.source_page === null) return null;
  return { pageId: link.source_page.id, blockId: link.block_id };
}

/**
 * What a committed edit refreshes.
 *
 * The reply of a mutation describes the page it was sent to.
 * Installing it as the open page's view when that is a different page
 * replaces the outline the user is reading with someone else's, so
 * `applyView` is true only when the source page **is** the open page.
 * That case is real: a page can cite itself, and discarding the reply
 * there would drop a genuine refresh of the outline.
 *
 * `refetchBacklinks` is unconditionally true.
 * A mutation on another page can change what cites the open page
 * whether or not the reply is applied, and working out when it cannot
 * is a second opinion about the backlink index that would drift.
 */
export function backlinkCommitPlan(args: {
  sourcePageId: string;
  openPageId: string | undefined;
}): BacklinkCommitPlan {
  return {
    applyView: args.openPageId !== undefined && args.sourcePageId === args.openPageId,
    refetchBacklinks: true,
  };
}

/**
 * What the user is told when a sibling lands on a page they are not
 * looking at — the honest answer, not a nudge.
 *
 * The new block references nothing, so it is not a backlink of the open
 * page and no row will ever render it. There is therefore nowhere to
 * put the cursor and no editor to open, which is the same wall the
 * TUI's `o` / `O` hit from the other side. Saying where it went, and
 * how to reach it, beats parking the caret on a row that is still the
 * *old* block or on something nobody drew.
 *
 * **It is only true when the source page is not the open one.** When it
 * is — a page citing itself — the block is in the outline on screen and
 * this sentence would be a lie, so it returns `null` there. The owner of
 * the sentence decides whether there is one; a caller cannot forget to
 * ask. What a client does *instead* (the desktop focuses the new block)
 * stays the client's, keyed off {@link backlinkCommitPlan}.
 *
 * Lives here because both GUI clients say it and the TUI says its own
 * terminal-shaped version: one more client phrasing it is one more
 * answer to "where did my block go".
 *
 * Two things the wording is deliberately **not**:
 *
 * - It does not name the control that opens the page. The desktop's is
 *   a group header and the phone's is a card header, so a shared
 *   sentence naming either is wrong on one of them.
 * - It is kept short because it lands in a toast. The first draft ran
 *   ~220 characters, which overflows a 24rem toast with nothing but
 *   jsdom behind the test — and mobile has less room, not more.
 *
 * Takes a resolved {@link PageMeta} rather than the {@link Backlink},
 * so there is no unreachable `?? "its own page"` branch: every caller
 * reaches this past a `source_page` check it already had to make.
 * `openPageId` is `undefined` while no page is open, which nothing on
 * screen can hold the block, so the sentence stands.
 */
export function backlinkCreatedElsewhere(
  page: PageMeta,
  side: "below" | "above",
  openPageId: string | undefined,
): string | null {
  if (page.id === openPageId) return null;
  const where = side === "below" ? "after" : "before";
  return `Added an empty block ${where} this reference, on ${page.title}. It does not reference this page, so no row here shows it — open ${page.title} to type in it.`;
}
