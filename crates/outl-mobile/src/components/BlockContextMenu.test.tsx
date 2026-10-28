/**
 * The long-press menu has to fit on the phone.
 *
 * It is anchored to the bottom edge, and it had no ceiling: a block
 * with every action enabled grew the card upward until the first row
 * sat under the status bar, unreachable, with no way to scroll to it.
 * Reported from a device with a photo of the clipped menu.
 *
 * These pin the two structural facts that keep it on screen, because
 * neither is visible to a test that only asks which rows rendered: the
 * sheet is bounded by the viewport, and the action list scrolls.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { BlockContextMenu, type BlockContextAction } from "./BlockContextMenu";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

function action(id: string): BlockContextAction {
  return { id, label: id, iconPath: "M0 0", onSelect: vi.fn() };
}

function mount(count: number) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const actions = Array.from({ length: count }, (_, i) => action(`row ${i}`));
  dispose = render(
    () => <BlockContextMenu open actions={actions} onClose={() => {}} />,
    host,
  );
  return host;
}

/** The positioned sheet: bottom-anchored, holds the card and Cancel. */
const sheet = (host: HTMLElement) =>
  host.querySelector<HTMLElement>(".outl-sheet-up")!;

describe("the long-press menu fits the screen", () => {
  it("bounds the sheet by the viewport", () => {
    const host = mount(14);
    // Without a ceiling the card grows past the top of the screen.
    expect(sheet(host).className).toContain("max-h-dvh");
  });

  it("scrolls the actions instead of overflowing them", () => {
    const host = mount(14);
    const scroller = sheet(host).querySelector<HTMLElement>(".overflow-y-auto");
    expect(scroller).not.toBeNull();
    // Every action is inside the scrolling region, not beside it.
    expect(scroller!.querySelectorAll("button")).toHaveLength(14);
  });

  it("keeps the grab handle and Cancel out of the scrolling region", () => {
    const host = mount(14);
    const scroller = sheet(host).querySelector<HTMLElement>(".overflow-y-auto")!;
    // The handle stays under the thumb and Cancel stays reachable, so
    // neither may scroll away with the list.
    expect(scroller.querySelector('[aria-label="Drag to close"]')).toBeNull();
    expect(scroller.textContent).not.toContain("Cancel");
  });
});
