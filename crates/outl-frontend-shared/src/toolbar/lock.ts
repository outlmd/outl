/**
 * Toolbar order lock — the user's opt-out from MFU reordering.
 *
 * MFU (`./mfu`) answers *"which order does usage suggest?"*. This
 * module answers *"is the user letting usage decide at all?"*, and
 * those are separate facts: a locked bar keeps counting taps, it just
 * stops applying the result. Unlocking therefore returns to a live MFU
 * order, not to the cold-start one.
 *
 * **A lock stores an order, not a boolean.** Counting keeps running
 * while locked, so a flag alone would leave the row free to jump the
 * moment it is unlocked-and-relocked, or on the next release that
 * touches the tiebreak. Freezing the concrete order is what makes the
 * promise ("these buttons stay where they are") true.
 *
 * The store is `localStorage`, which is where the tap counts live too
 * (`./mfu`). The iOS native bar reads both out of the webview through
 * `OutlKit.ToolbarStore` rather than keeping `UserDefaults` copies —
 * the settings sheet that writes them is web on both platforms, so a
 * native-side store would be a second answer the sheet cannot see.
 */
import { MIDDLE_ORDER, type ToolbarAction } from "./actions";
import {
  orderedMiddleActions,
  readCountsFromStore,
  type ToolbarCounts,
} from "./mfu";
import { safeGet, safeRemove, safeSet } from "./storage";

/** Versioned so a future schema change can't misread old shapes. Read
 *  by `OutlToolbar.swift` through `evaluateJavaScript`, so the string
 *  is a cross-language contract: renaming it here means renaming it
 *  there in the same commit. */
export const LOCK_STORAGE_KEY = "outl.toolbar.lock.v1";

/**
 * Reconcile a stored order against the current catalog.
 *
 * A frozen order is a snapshot, and the catalog moves under it: an
 * action added in a later release is in no snapshot, and one removed
 * is in every older snapshot. Locking the toolbar must not mean losing
 * a button that ships afterwards, so unknown ids (and the pinned
 * slots, and duplicates) are dropped and every middle action the
 * snapshot never saw is appended in catalog order.
 *
 * Appended rather than inserted at its catalog index on purpose: the
 * user locked the row to stop things moving, so a new button joins at
 * the end where it disturbs nothing, instead of shifting every button
 * after it.
 */
export function reconcileLockedOrder(
  saved: readonly string[],
): ToolbarAction[] {
  const known = new Set<ToolbarAction>(MIDDLE_ORDER);
  const seen = new Set<ToolbarAction>();
  const order: ToolbarAction[] = [];
  for (const id of saved) {
    const action = id as ToolbarAction;
    if (!known.has(action) || seen.has(action)) continue;
    seen.add(action);
    order.push(action);
  }
  for (const action of MIDDLE_ORDER) {
    if (!seen.has(action)) order.push(action);
  }
  return order;
}

/**
 * Parse a persisted lock blob. Tolerant of a missing key, malformed
 * JSON and non-array shapes — any of which means "not locked" rather
 * than throwing. A stored empty array still counts as locked; it
 * reconciles back to the full catalog order.
 */
export function parseLockedOrder(raw: string | null): ToolbarAction[] | null {
  if (!raw) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!Array.isArray(parsed)) return null;
  return reconcileLockedOrder(parsed.filter((v) => typeof v === "string"));
}

/**
 * The order the middle range should render in: the frozen one when
 * locked, the live MFU one otherwise. The single place that decides,
 * so a client can't answer it differently from the bar next to it.
 */
export function resolveMiddleOrder(
  counts: ToolbarCounts,
  locked: readonly ToolbarAction[] | null,
): ToolbarAction[] {
  return locked ? [...locked] : orderedMiddleActions(counts);
}

// ── localStorage-backed convenience ──────────────────────────────────

/** The frozen order, or `null` when the toolbar is not locked. */
function readLockedOrderFromStore(): ToolbarAction[] | null {
  return parseLockedOrder(safeGet(LOCK_STORAGE_KEY));
}

/** Whether the user has locked the toolbar. */
export function isOrderLocked(): boolean {
  return readLockedOrderFromStore() !== null;
}

/** Freeze `order` as the toolbar's layout until it is unlocked. */
export function lockOrderInStore(order: readonly ToolbarAction[]): void {
  safeSet(LOCK_STORAGE_KEY, JSON.stringify(reconcileLockedOrder(order)));
}

/** Drop the freeze and hand the row back to MFU. */
export function unlockOrderInStore(): void {
  safeRemove(LOCK_STORAGE_KEY);
}

/** The order to render, resolved straight from `localStorage`. What a
 *  client calls when it builds the bar. */
export function resolveMiddleFromStore(): ToolbarAction[] {
  return resolveMiddleOrder(readCountsFromStore(), readLockedOrderFromStore());
}
