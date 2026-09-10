import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { MIDDLE_ORDER as MIDDLE, PINNED_FIRST, PINNED_LAST } from "./actions";
import { orderedMiddleActions, recordToStore } from "./mfu";
import {
  isOrderLocked,
  LOCK_STORAGE_KEY,
  lockOrderInStore,
  parseLockedOrder,
  reconcileLockedOrder,
  resolveMiddleFromStore,
  resolveMiddleOrder,
  unlockOrderInStore,
} from "./lock";

describe("toolbar lock — reconcile against the catalog", () => {
  it("keeps a full snapshot exactly as saved", () => {
    const frozen = [...MIDDLE].reverse();
    expect(reconcileLockedOrder(frozen)).toEqual(frozen);
  });

  it("appends actions the snapshot never saw, in catalog order", () => {
    // A lock taken before `bold` / `italic` existed.
    const old = MIDDLE.filter((a) => a !== "bold" && a !== "italic");
    const order = reconcileLockedOrder(old);
    expect(order).toHaveLength(MIDDLE.length);
    expect(order.slice(0, old.length)).toEqual(old);
    expect(order.slice(old.length)).toEqual(["bold", "italic"]);
  });

  it("drops ids the catalog no longer has", () => {
    expect(reconcileLockedOrder(["bold", "sendCarrierPigeon"])).toContain("bold");
    expect(reconcileLockedOrder(["bold", "sendCarrierPigeon"])).toHaveLength(
      MIDDLE.length,
    );
  });

  it("drops the pinned slots — their position is fixed, not stored", () => {
    const order = reconcileLockedOrder([PINNED_FIRST, "bold", PINNED_LAST]);
    expect(order).not.toContain(PINNED_FIRST);
    expect(order).not.toContain(PINNED_LAST);
    expect(order[0]).toBe("bold");
  });

  it("de-dupes, keeping first appearance", () => {
    const order = reconcileLockedOrder(["bold", "italic", "bold"]);
    expect(order.slice(0, 2)).toEqual(["bold", "italic"]);
    expect(new Set(order).size).toBe(MIDDLE.length);
  });

  it("always returns the whole middle range, whatever went in", () => {
    for (const saved of [[], ["nonsense"], ["bold"], [...MIDDLE]]) {
      const order = reconcileLockedOrder(saved);
      expect(new Set(order)).toEqual(new Set(MIDDLE));
      expect(order).toHaveLength(MIDDLE.length);
    }
  });
});

describe("toolbar lock — parse tolerance", () => {
  it("reads null / malformed / non-array as not locked", () => {
    expect(parseLockedOrder(null)).toBeNull();
    expect(parseLockedOrder("")).toBeNull();
    expect(parseLockedOrder("not json")).toBeNull();
    expect(parseLockedOrder('{"bold":1}')).toBeNull();
    expect(parseLockedOrder("42")).toBeNull();
  });

  it("reads a stored empty array as locked on the full catalog order", () => {
    expect(parseLockedOrder("[]")).toEqual(MIDDLE);
  });

  it("ignores non-string entries rather than failing the whole blob", () => {
    const order = parseLockedOrder('["bold", 7, null, "italic"]');
    expect(order?.slice(0, 2)).toEqual(["bold", "italic"]);
  });
});

describe("toolbar lock — resolve", () => {
  it("returns the MFU order when unlocked", () => {
    const counts = { code: 10 };
    expect(resolveMiddleOrder(counts, null)).toEqual(orderedMiddleActions(counts));
  });

  it("returns the frozen order when locked, ignoring the counts", () => {
    const frozen = [...MIDDLE].reverse();
    expect(resolveMiddleOrder({ code: 9999 }, frozen)).toEqual(frozen);
  });

  it("does not hand out its own array (a caller can't mutate the lock)", () => {
    const frozen = [...MIDDLE];
    const resolved = resolveMiddleOrder({}, frozen);
    resolved.reverse();
    expect(frozen).toEqual(MIDDLE);
  });
});

/** happy-dom (this repo's Vitest env) ships no `localStorage`; the real
 *  Tauri webview always has one. Same in-memory stub `mfu.test.ts` uses. */
class MemStorage {
  private m = new Map<string, string>();
  get length() {
    return this.m.size;
  }
  clear() {
    this.m.clear();
  }
  getItem(k: string) {
    return this.m.has(k) ? (this.m.get(k) as string) : null;
  }
  setItem(k: string, v: string) {
    this.m.set(k, String(v));
  }
  removeItem(k: string) {
    this.m.delete(k);
  }
  key(i: number) {
    return [...this.m.keys()][i] ?? null;
  }
}

describe("toolbar lock — localStorage persistence", () => {
  beforeAll(() => {
    vi.stubGlobal("localStorage", new MemStorage());
  });
  afterAll(() => {
    vi.unstubAllGlobals();
  });
  beforeEach(() => {
    localStorage.clear();
  });

  it("starts unlocked", () => {
    expect(isOrderLocked()).toBe(false);
  });

  it("locking freezes the order against later taps", () => {
    recordToStore("bold");
    const before = resolveMiddleFromStore();
    lockOrderInStore(before);

    // Enough taps that MFU would certainly have reordered by now.
    for (let i = 0; i < 50; i++) recordToStore("delete");

    expect(resolveMiddleFromStore()).toEqual(before);
    expect(isOrderLocked()).toBe(true);
  });

  it("unlocking hands the row back to MFU, counts and all", () => {
    lockOrderInStore(orderedMiddleActions({}));
    for (let i = 0; i < 5; i++) recordToStore("delete");
    unlockOrderInStore();

    expect(isOrderLocked()).toBe(false);
    // The taps taken while locked were still counted — unlocking
    // returns to a live MFU order, not to the cold-start one.
    expect(resolveMiddleFromStore()[0]).toBe("delete");
  });

  it("writes under the versioned key", () => {
    lockOrderInStore(["bold"]);
    expect(localStorage.getItem(LOCK_STORAGE_KEY)).toContain("bold");
  });

  it("stores a reconciled order, so a partial lock still covers the row", () => {
    lockOrderInStore(["bold"]);
    const stored = JSON.parse(localStorage.getItem(LOCK_STORAGE_KEY) as string);
    expect(stored[0]).toBe("bold");
    expect(stored).toHaveLength(MIDDLE.length);
  });
});
