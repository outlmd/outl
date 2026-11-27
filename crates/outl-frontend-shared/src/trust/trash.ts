/**
 * The trash's restore rules, shared by every GUI client (issue #287).
 *
 * **Nothing here decides what can be restored, or what to say about
 * it.** `entry.refusal` is `outl_actions::trash::refusal_for`'s own
 * verdict, and a refused row is rendered, not hidden. This module only
 * owns the *sequencing* around a restore, so desktop and mobile cannot
 * drift on it.
 *
 * ## A refusal can go stale between the list and the click
 *
 * The verdict depends on the live tree, so a peer can trash the parent a
 * row would land under after this list was fetched. The row's `refusal`
 * is therefore a filter, never a promise:
 *
 * 1. `restoreTrashEntry` is allowed to reject even on a row that looked
 *    actionable, and the rejection carries the **same verdict** the row
 *    would have shown. Not the same *sentence*: a row is worded for its
 *    surface and carries the way out, while the mutation's error is the
 *    bare diagnosis, which is why `notice` asks whether the row is
 *    refused rather than comparing the two strings (a string compare
 *    never matches, so a refused restore would print its reason twice,
 *    in two voices).
 * 2. That sentence lands *in the row* (`notice`), not in a toast: a
 *    toast scrolls away from the thing it is about.
 * 3. The listing then refetches, and **the listing alone decides whether
 *    the row is still restorable**. The attempt's notice is additive and
 *    is cleared by opening the surface again. Folding the two together
 *    gave the surface a third state the backend never returned, and since
 *    nothing cleared it, a row stayed un-restorable after a peer put its
 *    parent back, forever, across reopens.
 *
 * The cost is one extra `list_trash` after a failed restore, the same
 * read the surface already does on open.
 */

import {
  createEffect,
  createResource,
  createSignal,
  type Accessor,
  type Resource,
} from "solid-js";

import { listTrash, restoreTrashEntry, type TrashEntry } from "../api/trash";
import type { PageView } from "../api/types";

/**
 * "3d ago" for a deletion's wall clock.
 *
 * Rendered, never sorted on: `deleted_at_ms` comes from whichever device
 * did the deleting, so a peer with a skewed clock can legitimately show
 * a time that reads out of order. The row order is the backend's, which
 * cannot depend on a clock.
 */
export function deletedAgo(ms: number | null, nowMs: number = Date.now()): string {
  if (ms === null) return "";
  const seconds = Math.round((nowMs - ms) / 1000);
  if (seconds < 90) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(ms).toLocaleDateString();
}

export interface TrashRestore {
  rows: Resource<TrashEntry[] | undefined>;
  /** The id a restore is in flight for. */
  busy: Accessor<string | null>;
  /** true when the entry came back (view applied if non-null); false when refused. */
  restore(entry: TrashEntry): Promise<boolean>;
  /** The last attempt's sentence for this row, or null when the row's own refusal speaks or another row failed. */
  notice(entry: TrashEntry): string | null;
}

export function createTrashRestore(opts: {
  open: Accessor<boolean>;
  onView: (view: PageView) => void;
  onError: (message: string) => void;
}): TrashRestore {
  // Gated on being open: the surface is mounted for the app's whole
  // life, so an ungated resource would fold every deletion's parent
  // trail at boot.
  const [rows, { refetch }] = createResource(
    () => opts.open() || undefined,
    async () => {
      try {
        return await listTrash();
      } catch (e) {
        opts.onError(e instanceof Error ? e.message : String(e));
        return [] as TrashEntry[];
      }
    },
  );

  const [busy, setBusy] = createSignal<string | null>(null);
  const [failed, setFailed] = createSignal<{ id: string; why: string } | null>(null);

  // **Every opening starts clean**, so a notice nothing clears cannot
  // outlive its condition. On *open* rather than inside the fetcher,
  // because a failed restore refetches on purpose and clearing there
  // would wipe the sentence that refetch exists to accompany.
  createEffect(() => {
    if (opts.open()) setFailed(null);
  });

  async function restore(entry: TrashEntry): Promise<boolean> {
    setBusy(entry.id);
    setFailed(null);
    try {
      const reply = await restoreTrashEntry(entry.id);
      // The view *is* the landing page, so applying it both refreshes
      // and navigates.
      if (reply.view) opts.onView(reply.view);
      return true;
    } catch (e) {
      setFailed({ id: entry.id, why: e instanceof Error ? e.message : String(e) });
      void refetch();
      return false;
    } finally {
      setBusy(null);
    }
  }

  // Deliberately **not** folded into `entry.refusal`: whether a row can
  // be restored has one owner, the backend's listing (rule 3 above).
  function notice(entry: TrashEntry): string | null {
    const late = failed();
    if (late?.id !== entry.id) return null;
    return entry.refusal ? null : late.why;
  }

  return { rows, busy, restore, notice };
}
