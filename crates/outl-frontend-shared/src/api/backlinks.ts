/**
 * The backlinks panel's commands: read the list, change its direction,
 * and mutate a block through one of its rows.
 *
 * Split out of `commands.ts` under the file-size ratchet, the same move
 * `paste.ts` made, and re-exported from there so
 * `@outl/shared/api/commands` stays the one import path. They belong
 * together because two of them return the same `PageBacklinks` and the
 * third is the only mutation that acts on a row of it.
 *
 * Nothing here decides anything. What counts as a backlink is
 * `outl_actions::backlinks`, the order is `outl_actions::sort_backlinks`
 * behind the backend's persisted preference, and how a cross-page
 * mutation recovers — refuse, never re-target — is
 * `outl_tauri_shared::commands::backlink_edit`, for all ten gestures.
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  BacklinksOrder,
  CreateBlockReply,
  PageBacklinks,
  PageView,
} from "./types";

/**
 * Compute a page's backlinks lazily, off the page-open path.
 *
 * `backlinks_for_page` is an O(blocks-in-workspace) scan, so it's no
 * longer bundled into {@link PageView} — computing it there blocked the
 * first journal paint on a large workspace. Call this after the outline
 * renders (keyed on the current page slug) and fill the backlinks panel
 * when it resolves. Mirrors the TUI's lazy/cached backlinks.
 */
export function pageBacklinks(slug: string): Promise<PageBacklinks> {
  return invoke<PageBacklinks>("page_backlinks", { slug });
}

/**
 * Persist the backlinks-list direction (issue #142) and get `slug`'s
 * backlinks back re-sorted under it. A pure display preference — it lives
 * in `config.toml`, never the op log, so it does not converge between
 * devices (same policy as the theme). Returns only the re-sorted
 * backlinks (not a whole `PageView`): the panel already has the outline.
 */
export function setBacklinksOrder(
  order: BacklinksOrder,
  slug: string,
): Promise<PageBacklinks> {
  return invoke<PageBacklinks>("set_backlinks_order", { order, slug });
}

/**
 * Replace the text of a block shown in a **backlink row**, on the page
 * it lives on. `pageId` is the backlink's *source* page, never the page
 * on screen.
 *
 * Not `editBlock` with a different argument: the two differ in how they
 * recover. `editBlock` tolerates a block that has left the tree by
 * appending the text at the end of the page, which keeps it visible when
 * that page is the one you are reading. A backlink's `block_id` can be a
 * transient id the tree never had, and appending there would move the
 * user's text to a page they are not looking at — so this one **rejects**
 * instead, and rejects a source page that has stopped syncing with the
 * same wording the "this page isn't syncing" banner carries.
 *
 * So a rejection here is a message to show, not a Retry to pin: nothing
 * was written, and retrying the same id cannot succeed.
 *
 * The resolved {@link PageView} describes the source page. Apply it only
 * when that page is the one on screen (a page can cite itself).
 */
export function editBacklinkBlock(
  pageId: string,
  id: string,
  text: string,
): Promise<PageView> {
  return invoke<PageView>("edit_backlink_block", { pageId, id, text });
}

/**
 * Which structural or task-state mutation a backlink row is asking for.
 *
 * Mirror of `outl_tauri_shared::commands::backlink_edit::BacklinkRowOp`,
 * pinned by that crate's `wire_enums.rs`. A create is not here — it
 * replies with a new block's id, so it is
 * {@link createBacklinkSibling}.
 */
export type BacklinkRowOp =
  | "toggle-todo"
  | "mark-done"
  | "delete"
  | "indent"
  | "outdent"
  | "move-up"
  | "move-down"
  // Explicit, never a toggle: a toggle aimed at a stale view flips the
  // wrong way. Writes `Op::SetCollapsed` on the **source** page.
  | "collapse"
  | "expand";

/** Which side of the citing block a new sibling lands on. */
export type BacklinkSide = "below" | "above";

/**
 * Mutate the block a **backlink row** shows, on the page it lives on.
 * `pageId` is the backlink's *source* page, never the page on screen.
 *
 * Not the `block.rs` twin (`toggleTodo`, `deleteBlock`, `indentBlock`,
 * …) with a different page id. Those act on the page the user is
 * looking at and may therefore *tolerate* a block id the tree no
 * longer holds; a backlink's `block_id` is routinely transient, so this
 * one **rejects** instead — and rejects in the words
 * `outl_actions::refusal` owns, rather than `ActionError`'s `Display`,
 * which is a raw id and an implementation noun.
 *
 * So a rejection is a message to show, never a Retry to pin: nothing
 * was written, and retrying the same id cannot succeed.
 *
 * The resolved `PageView` describes the source page. Apply it only when
 * that page is the one on screen (a page can cite itself) —
 * `backlinkCommitPlan` in `@outl/shared/backlink-edit` is the owner of
 * that question.
 */
export function backlinkRowOp(
  pageId: string,
  id: string,
  op: BacklinkRowOp,
): Promise<PageView> {
  return invoke<PageView>("backlink_row_op", { pageId, id, op });
}

/**
 * Create an empty sibling beside a backlink row's block, on its own
 * page.
 *
 * Not `createBlock` with the source page's id: `create_block` recovers
 * from a stale anchor by **appending at the end of the page**, which is
 * documented for the open page and is a silent move to a page the user
 * is not looking at from here. This one refuses, in the same sentence
 * every other cross-page gesture uses.
 *
 * The new block cites nothing, so it is a backlink of no page and no
 * row will render it — `backlinkCreatedElsewhere` is what tells the
 * user where it went.
 */
export function createBacklinkSibling(
  pageId: string,
  id: string,
  side: BacklinkSide,
): Promise<CreateBlockReply> {
  return invoke<CreateBlockReply>("create_backlink_sibling", {
    pageId,
    id,
    side,
  });
}

/**
 * Open every folded block between block `id` and `pageId`, and get the
 * page's `PageView` back with them open.
 *
 * The reveal half of landing on a block ("open this page at the
 * backlink's parent"). Fold is op-log state, so this **writes** (and
 * syncs) one `SetCollapsed(false)` per folded ancestor, and nothing when
 * none is folded; `needsUnfold` (`@outl/shared/outline`) lets a client
 * skip the call in that case. Pass the block that cites, so the parent
 * and everything above it open and the reference shows under the parent.
 *
 * Rejects, writing nothing, a block that is not live on `pageId`
 * (trashed, unknown, moved to another page). `outl_actions::unfold_ancestors`
 * owns which blocks open.
 */
export function unfoldAncestors(pageId: string, id: string): Promise<PageView> {
  return invoke<PageView>("unfold_ancestors", { pageId, id });
}
