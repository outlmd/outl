/**
 * The action panel is inset by a point, and that point is the fix.
 *
 * A row lands on a fractional y (162.4 and 223.4 are real values off a
 * journal page). A background is rasterised per element, so the panel
 * and the opaque row covering it rounded to different device pixels: a
 * 1px line of red ran along the top and bottom of every row's right
 * 96px, at rest, on every block in the outline. Found in a screenshot.
 * jsdom computes no layout, so no test in this file can see the pixels.
 *
 * What is pinned is the geometry that makes the leak impossible. The
 * first attempt moved the colour onto the clipping parent instead,
 * which measured *worse* — same rounding, now across the full row
 * width — so the parent staying unpainted is asserted too.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it } from "vitest";

import { SwipeRow } from "./SwipeRow";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

function mount() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <SwipeRow>a block</SwipeRow>, host);
  return host.firstElementChild as HTMLElement;
}

const ACTION = "bg-(--color-outl-destructive)";
const panel = (root: HTMLElement) => root.querySelector<HTMLElement>(".absolute")!;

describe("the swipe action cannot leak past the row", () => {
  it("insets the panel instead of filling the row's height", () => {
    const p = panel(mount());
    expect(p.className).toContain("inset-y-px");
    // `inset-y-0` is the spelling that leaked.
    expect(p.className).not.toMatch(/inset-y-0\b/);
  });

  it("keeps the colour on the panel, never on the clipping parent", () => {
    const root = mount();
    expect(panel(root).className).toContain(ACTION);
    expect(root.className).toContain("overflow-hidden");
    expect(root.className).not.toContain(ACTION);
  });

  it("covers the action with an opaque row at rest", () => {
    const cover = mount().querySelector<HTMLElement>(
      ".relative.bg-\\(--color-outl-bg\\)",
    );
    expect(cover).not.toBeNull();
    expect(cover!.style.transform).toBe("translateX(0px)");
  });
});
