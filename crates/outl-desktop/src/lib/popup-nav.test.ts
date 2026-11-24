import { describe, expect, it, vi } from "vitest";

import { handlePopupNav, type PopupNav } from "./popup-nav";

/** Build a fake keydown with the fields the handler reads plus spied
 *  preventDefault / stopPropagation. */
function key(
  k: string,
  mods: Partial<
    Record<"metaKey" | "ctrlKey" | "shiftKey" | "altKey", boolean>
  > = {},
) {
  return {
    key: k,
    metaKey: false,
    ctrlKey: false,
    shiftKey: false,
    altKey: false,
    ...mods,
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
  };
}

/** A nav over 3 string items with spied callbacks, starting at `index`.
 *  Return type is inferred so the `vi.fn()` mock types stay intact for
 *  `toHaveBeenCalledWith` assertions. */
function nav(index = 0) {
  return {
    items: ["a", "b", "c"],
    index,
    setIndex: vi.fn<(next: number) => void>(),
    onAccept: vi.fn<(item: string) => void>(),
    onClose: vi.fn<() => void>(),
  } satisfies PopupNav<string>;
}

describe("handlePopupNav", () => {
  it("returns false and does nothing when the popup is empty", () => {
    const n = { ...nav(), items: [] as string[] };
    const e = key("ArrowDown");
    expect(handlePopupNav(e, n)).toBe(false);
    expect(e.preventDefault).not.toHaveBeenCalled();
    expect(n.setIndex).not.toHaveBeenCalled();
  });

  it("ArrowDown moves to the next index and wraps at the end", () => {
    const n = nav(2); // last of 3
    const e = key("ArrowDown");
    expect(handlePopupNav(e, n)).toBe(true);
    expect(n.setIndex).toHaveBeenCalledWith(0);
    expect(e.preventDefault).toHaveBeenCalled();
    expect(e.stopPropagation).toHaveBeenCalled();
  });

  it("ArrowUp moves to the previous index and wraps at the top", () => {
    const n = nav(0);
    expect(handlePopupNav(key("ArrowUp"), n)).toBe(true);
    expect(n.setIndex).toHaveBeenCalledWith(2);
  });

  it("Enter accepts the highlighted item", () => {
    const n = nav(1);
    expect(handlePopupNav(key("Enter"), n)).toBe(true);
    expect(n.onAccept).toHaveBeenCalledWith("b");
  });

  it("Tab also accepts the highlighted item", () => {
    const n = nav(1);
    expect(handlePopupNav(key("Tab"), n)).toBe(true);
    expect(n.onAccept).toHaveBeenCalledWith("b");
  });

  it("Escape closes without accepting", () => {
    const n = nav(1);
    expect(handlePopupNav(key("Escape"), n)).toBe(true);
    expect(n.onClose).toHaveBeenCalled();
    expect(n.onAccept).not.toHaveBeenCalled();
  });

  it("Enter/Tab with any modifier falls through (returns false)", () => {
    for (const mod of ["metaKey", "ctrlKey", "shiftKey", "altKey"] as const) {
      for (const k of ["Enter", "Tab"]) {
        const n = nav(1);
        const e = key(k, { [mod]: true });
        expect(handlePopupNav(e, n)).toBe(false);
        expect(n.onAccept).not.toHaveBeenCalled();
        expect(e.preventDefault).not.toHaveBeenCalled();
      }
    }
  });

  it("an unrelated key falls through", () => {
    const n = nav(1);
    const e = key("x");
    expect(handlePopupNav(e, n)).toBe(false);
    expect(e.preventDefault).not.toHaveBeenCalled();
  });

  it("clamps a stale out-of-bounds index (list shrank) instead of accepting undefined", () => {
    // index 9 but only 3 items — the search result shrank under a stale
    // highlight. Accept must land on the last valid item, never undefined.
    const n = { ...nav(9), onAccept: vi.fn<(item: string) => void>() };
    expect(handlePopupNav(key("Enter"), n)).toBe(true);
    expect(n.onAccept).toHaveBeenCalledWith("c");
  });

  it("clamps a stale index on ArrowDown so it wraps from the clamped position", () => {
    const n = nav(9); // clamps to 2 (last), +1 wraps to 0
    expect(handlePopupNav(key("ArrowDown"), n)).toBe(true);
    expect(n.setIndex).toHaveBeenCalledWith(0);
  });
});

describe("handlePopupNav and the block-move chords", () => {
  it("a modified arrow is not consumed; a bare one still is; Shift+arrow unchanged", () => {
    for (const k of ["ArrowUp", "ArrowDown"]) {
      for (const mod of ["metaKey", "ctrlKey", "altKey"] as const) {
        const n = nav(1);
        const e = key(k, { [mod]: true });
        expect(handlePopupNav(e, n)).toBe(false);
        expect(e.preventDefault).not.toHaveBeenCalled();
        expect(n.setIndex).not.toHaveBeenCalled();
      }
      const bare = nav(1);
      expect(handlePopupNav(key(k), bare)).toBe(true);
      const shifted = nav(1);
      expect(handlePopupNav(key(k, { shiftKey: true }), shifted)).toBe(true);
      expect(shifted.setIndex).toHaveBeenCalled();
    }
  });
});
