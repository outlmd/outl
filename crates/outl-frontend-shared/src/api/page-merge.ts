/**
 * Wire types and command wrappers for renaming a page and merging one
 * page into another.
 *
 * Its own file because `commands.ts` and `types.ts` are at their size
 * ratchet; listed in the wire-mirror files
 * (`outl-tauri-shared/tests/ts_parser`) like the rest, so the key sets
 * here are pinned against the Rust DTOs rather than trusted.
 *
 * **Nothing here decides what a merge does.** The semantics live in
 * `outl_actions::merge_pages` and `rename_page`; these wrappers carry
 * the answer across the bridge. The flow on top of them (rename, then
 * confirm a merge when the name is taken) and the wording live in
 * `@outl/shared/pages`.
 */

import { invoke } from "@tauri-apps/api/core";

import type { PageMeta, PageView } from "./types";

/**
 * One property both pages of a merge carried with different values.
 * The target's value wins (`kept`), the source's is reported
 * (`dropped`) so the summary can say what was left behind. Mirrors
 * `outl_tauri_shared::PropertyConflictView`.
 */
export interface PropertyConflictView {
  key: string;
  kept: string;
  dropped: string;
}

/**
 * What a rename changed beyond the page itself. Mirrors
 * `outl_tauri_shared::RenamePageReport`.
 */
export interface RenamePageReport {
  old_slug: string;
  old_title: string;
  /** Blocks (on any page) whose `[[old]]` links were rewritten to the new name. */
  rewritten_blocks: number;
  /** Namespace refs (`[[old/child]]`) a rename leaves alone, counted so the summary can say so. */
  skipped_namespace_refs: number;
}

/**
 * What a merge did. Mirrors `outl_tauri_shared::MergePagesReport`.
 * `already_merged` is true when a retry found nothing left to move
 * (the source was already gone), so a summary can say so instead of
 * reporting zero blocks as if something had gone wrong.
 */
export interface MergePagesReport {
  source_slug: string;
  source_title: string;
  target_slug: string;
  target_title: string;
  moved_blocks: number;
  rewritten_blocks: number;
  skipped_namespace_refs: number;
  property_conflicts: PropertyConflictView[];
  /** Names added to the target's `alias::` so old `[[links]]` still resolve. */
  aliases_added: string[];
  already_merged: boolean;
}

/**
 * Reply of `rename_page`. `exists` is a typed outcome, not an error:
 * the new name already belongs to another page and nothing was
 * written, so the client can offer to merge instead. Every other
 * failure (journal, not found, invalid name) stays a rejected promise
 * carrying the message. Mirrors `outl_tauri_shared::RenamePageReply`.
 */
export type RenamePageReply =
  | { status: "renamed"; view: PageView; report: RenamePageReport }
  | { status: "exists"; existing: PageMeta };

/** Reply of `merge_pages`: the **target** page's view plus the report. */
export interface MergePagesReply {
  view: PageView;
  report: MergePagesReport;
}

/**
 * Rename a page to `newName` (human text: the backend derives the new
 * slug and keeps the trimmed text as the title). `page` is a slug or a
 * page id. Links to the old name are rewritten across the workspace.
 *
 * Resolves to `{ status: "renamed" }` with the renamed page's view, or
 * to `{ status: "exists" }` when the name already belongs to another
 * page. That second case is a typed outcome and **nothing was
 * written**: the caller decides whether to offer a merge (see
 * `renameOrMerge` in `@outl/shared/pages`). Any other failure (a
 * journal, an unknown page, an invalid name) rejects with the
 * backend's message.
 */
export function renamePage(page: string, newName: string): Promise<RenamePageReply> {
  return invoke<RenamePageReply>("rename_page", { page, newName });
}

/**
 * Merge `source` into `target` (both a slug or a page id): the
 * source's blocks move onto the target, links to the source are
 * rewritten, the source's name becomes an alias of the target and the
 * source page is deleted. Resolves to the **target** page's view.
 *
 * The caller MUST confirm before invoking, this call does not
 * re-prompt. The UI offers no undo for a merge: the ops stay in the
 * log, but nothing in the clients reverses them.
 */
export function mergePages(source: string, target: string): Promise<MergePagesReply> {
  return invoke<MergePagesReply>("merge_pages", { source, target });
}
