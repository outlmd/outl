/**
 * Wire types and command wrappers for the trash: what was deleted, and
 * putting one back (issue #287).
 *
 * Its own file because `commands.ts` and `types.ts` are at their size
 * ratchet; listed in the wire-mirror files
 * (`outl-tauri-shared/tests/ts_parser`) like the rest, so the key sets
 * here are pinned against the Rust DTOs rather than trusted.
 *
 * **Nothing here decides whether a deletion can be restored.**
 * `refusal` is `outl_actions::trash::refusal_for`'s own sentence,
 * carried per row, and `restoreTrashEntry` rejects with the identical
 * string when the verdict changed under the listing. A client renders
 * both; it never writes its own wording and never re-derives the rule.
 */

import { invoke } from "@tauri-apps/api/core";

import type { PageView } from "./types";

/**
 * One top-level deletion, as returned by `list_trash`. Mirrors the Rust
 * `TrashEntryDto` in `outl-tauri-shared`.
 *
 * "Top-level" is the node the delete named: a deleted subtree is one
 * row, and `blocks` is how many come back with it.
 */
export interface TrashEntry {
  id: string;
  text: string;
  page: string | null;
  blocks: number;
  deleted_at_ms: number | null;
  refusal: string | null;
}

/** What came back out of the trash. Mirrors `RestoreTrashReply`. */
export interface RestoreTrashResult {
  id: string;
  blocks: number;
  view: PageView | null;
}

/**
 * Every top-level deletion still in the trash, each carrying the
 * refusal a restore would return.
 *
 * The rows arrive in the backend's order, which is stable because it
 * cannot depend on a peer's clock. Sort on `deleted_at_ms` only if the
 * surface wants recency and accepts that a device with a skewed clock
 * reshuffles the list.
 */
export function listTrash(): Promise<TrashEntry[]> {
  return invoke<TrashEntry[]>("list_trash");
}

/**
 * Put one deleted block back where it was deleted from.
 *
 * Rejects with the backend's own refusal when the landing place is gone
 * — which can happen to a row the listing showed as restorable, so a
 * caller handles the rejection *and* re-lists rather than trusting the
 * row it rendered.
 *
 * `view` is the page the block landed on, refreshed. It is `null` only
 * for a block restored to directly under the tree root, which has no
 * page to show.
 */
export function restoreTrashEntry(id: string): Promise<RestoreTrashResult> {
  return invoke<RestoreTrashResult>("restore_trash_entry", { id });
}
