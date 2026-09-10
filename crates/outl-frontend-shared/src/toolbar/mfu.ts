/**
 * Most-frequently-used ordering for the mobile edit toolbar.
 *
 * A direct port of `OutlKit/Toolbar/ToolbarMFU.swift`: persist per-device
 * tap counts and re-order the *middle* of the row by count desc on each
 * read, keeping the first/last slots pinned (`newLine` / `done`).
 *
 * The store is `localStorage`, and it is the **only** one: the iOS bar
 * is native but records its taps through `window.__outlToolbar` into
 * this same key (`Journal.tsx`'s `dispatchToolbarAction` is the single
 * counter for both bars), and reads them back through
 * `OutlKit.ToolbarStore`.
 *
 * It used to keep a second copy in `UserDefaults`, on the argument that
 * only one bar runs per device so the two could not disagree. The
 * argument was wrong: the **settings sheet** is web on iOS too, so it
 * read an always-empty store — "Lock button order" froze a cold-start
 * row instead of the user's, and "Reset button order" did nothing.
 * Counts are still per-device UI state, never a synced value.
 *
 * The pure `orderedMiddleActions(counts)` / `record(action, counts)`
 * overloads take an explicit map so they stay deterministic and testable;
 * the `*FromStore` convenience wrappers read/write `localStorage`.
 *
 * MFU decides the order usage *suggests*. Whether that suggestion is
 * applied at all is `./lock`'s question — see `resolveMiddleOrder`.
 */
import {
  DEFAULT_ORDER,
  MIDDLE_ORDER,
  PINNED_FIRST,
  PINNED_LAST,
  type ToolbarAction,
} from "./actions";
import { safeGet, safeRemove, safeSet } from "./storage";

/** Versioned so a future schema change can't misread old shapes.
 *  Same string as the Swift `ToolbarMFU.storageKey`. */
export const MFU_STORAGE_KEY = "outl.toolbar.mfu.v1";

export type ToolbarCounts = Partial<Record<ToolbarAction, number>>;

/** Is this string one of the catalog's action ids? Exported because the
 *  iOS bar ships its action as a bare string over the
 *  `window.__outlToolbar` bridge, and an id the catalog doesn't have
 *  must not reach `recordToStore` — it would be persisted forever. */
export function isToolbarAction(key: string): key is ToolbarAction {
  return (DEFAULT_ORDER as readonly string[]).includes(key);
}

/** Parse a persisted counts blob. Tolerant of a missing key, malformed
 *  JSON, non-object shapes, unknown action ids, and non-integer values —
 *  any of which yields an empty map rather than throwing. */
export function parseCounts(raw: string | null): ToolbarCounts {
  if (!raw) return {};
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return {};
  }
  if (typeof parsed !== "object" || parsed === null) return {};
  const counts: ToolbarCounts = {};
  for (const [key, value] of Object.entries(parsed as Record<string, unknown>)) {
    if (isToolbarAction(key) && typeof value === "number" && Number.isFinite(value)) {
      counts[key] = Math.trunc(value);
    }
  }
  return counts;
}

/**
 * Pure MFU ordering for the middle (scrollable) range — excludes the two
 * pinned slots. The client renders `[PINNED_FIRST, ...this, PINNED_LAST]`,
 * keeping the pinned buttons static outside the scroll. Mirrors
 * `ToolbarMFU.orderedMiddleActions(counts:)`.
 */
export function orderedMiddleActions(counts: ToolbarCounts): ToolbarAction[] {
  return [...MIDDLE_ORDER].sort((a, b) => {
    const ca = counts[a] ?? 0;
    const cb = counts[b] ?? 0;
    if (ca !== cb) return cb - ca;
    // Stable tiebreak: original `DEFAULT_ORDER` position.
    return DEFAULT_ORDER.indexOf(a) - DEFAULT_ORDER.indexOf(b);
  });
}

/**
 * Increment `action`'s count in an explicit map (pure). No-op for the
 * pinned actions — their slot is fixed by position, so counting them just
 * wastes storage. Mirrors `ToolbarMFU.record`.
 */
export function record(action: ToolbarAction, counts: ToolbarCounts): ToolbarCounts {
  if (action === PINNED_FIRST || action === PINNED_LAST) return counts;
  return { ...counts, [action]: (counts[action] ?? 0) + 1 };
}

// ── localStorage-backed convenience ──────────────────────────────────
// Reads/writes go through `./storage`, which degrades to in-memory
// defaults rather than throwing when `localStorage` is unavailable.

/** Read counts from `localStorage`. */
export function readCountsFromStore(): ToolbarCounts {
  return parseCounts(safeGet(MFU_STORAGE_KEY));
}

/** Increment `action` in `localStorage` and return the new counts. */
export function recordToStore(action: ToolbarAction): ToolbarCounts {
  const next = record(action, readCountsFromStore());
  safeSet(MFU_STORAGE_KEY, JSON.stringify(next));
  return next;
}

/** MFU-ordered middle range read straight from `localStorage`. */
export function orderedMiddleFromStore(): ToolbarAction[] {
  return orderedMiddleActions(readCountsFromStore());
}

/** Wipe every count, sending the row back to `DEFAULT_ORDER` on the
 *  next read. Backs the settings sheet's "Reset button order", for both
 *  bars: Swift's `ToolbarMFU.clearCounts` used to be the iOS half and
 *  is gone along with the `UserDefaults` store it cleared. */
export function clearCountsInStore(): void {
  safeRemove(MFU_STORAGE_KEY);
}
