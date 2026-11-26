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

import type { Backlink, BlockNode, PageMeta } from "../api/types";

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
 *
 * The citing block itself, by `link.block_id`. A **child** of it is
 * addressed by {@link backlinkEditTargetFor}, which differs only in the
 * block id it reads.
 */
export function backlinkEditTarget(link: Backlink): BacklinkEditTarget | null {
  if (link.source_page === null) return null;
  return { pageId: link.source_page.id, blockId: link.block_id };
}

/**
 * Which block does editing `node` mutate, where `node` is the citing
 * block of `link` **or any descendant shipped with it**?
 *
 * A child lives on the same source page as the block that cites, so
 * the page id is the link's; only the block id changes. The backend's
 * cross-page family refuses an id that is not live, which is the right
 * answer for a child with a transient id (a page whose sidecar is
 * absent mints one per uncovered block).
 */
export function backlinkEditTargetFor(
  link: Backlink,
  node: BlockNode,
): BacklinkEditTarget | null {
  if (link.source_page === null) return null;
  return { pageId: link.source_page.id, blockId: node.id };
}

/** One drawn row of the "Linked from" panel: a citing block or a descendant of one. */
export interface BacklinkRow {
  /**
   * **Unique across the whole panel**, which `node.id` is not: a block
   * that cites the page *and* sits under another citing block is drawn
   * twice, once as its own row and once inside its ancestor's subtree.
   * Key a `<For>`, an editing slot or a selection on this, never on the
   * block id — two rows with one key would open one editor in two
   * places.
   */
  key: string;
  link: Backlink;
  node: BlockNode;
  /** 0 for the citing block, 1 for its children, and so on. */
  depth: number;
}

/**
 * The panel's rows in the order the user reads and navigates them: each
 * citing block, then its subtree depth-first, then the next citing
 * block.
 *
 * The single flattening both GUI clients walk for `j`/`k`, for "is this
 * id one of mine" guards and for rendering, so the three cannot disagree
 * about which rows exist. `O(rows)`; callers memoize it per fetch rather
 * than per render (invariant 15).
 *
 * A `collapsed` node keeps its own row and hides its children, the
 * outline's rule. The flag is the source page's (`Op::SetCollapsed`,
 * read back by `page_backlinks`), so a fold made here or there shows the
 * same on both, and a hidden row cannot be reached by `j`/`k` either.
 */
export function flattenBacklinkRows(links: readonly Backlink[]): BacklinkRow[] {
  const rows: BacklinkRow[] = [];
  const walk = (link: Backlink, node: BlockNode, depth: number, key: string) => {
    rows.push({ key, link, node, depth });
    if (node.collapsed) return;
    for (const child of node.children) {
      walk(link, child, depth + 1, `${key}>${child.id}`);
    }
  };
  for (const link of links) {
    walk(link, link.source_block, 0, link.block_id);
  }
  return rows;
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
 * **It is also only true for the citing block itself.** A sibling made
 * from a *child* (`rowDepth > 0`) is a block under the same citing
 * block, so it sits inside the subtree the panel draws and the next
 * refetch shows it. Saying "no row here shows it" there would be the
 * same lie, so it returns `null` for that too — again decided here, not
 * by whichever client remembers to skip it. (A sibling of a depth-4 row
 * is still depth 4, inside the cap.)
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
  rowDepth = 0,
): string | null {
  if (page.id === openPageId) return null;
  if (rowDepth > 0) return null;
  const where = side === "below" ? "after" : "before";
  return `Added an empty block ${where} this reference, on ${page.title}. It does not reference this page, so no row here shows it — open ${page.title} to type in it.`;
}
