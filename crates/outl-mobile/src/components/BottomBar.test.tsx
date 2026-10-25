/**
 * The bottom bar is the phone's map (issue #364): Journal, Search,
 * Tasks, Reminders, with capture in the middle. Pages is not a tab —
 * Search is the one door to pages, tasks and blocks.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { BottomBar, type BottomTab } from "./BottomBar";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

function mount(active: BottomTab | null, onTab = vi.fn()) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BottomBar active={active} hidden={false} onTab={onTab} onCapture={() => {}} />
    ),
    host,
  );
  return { host, onTab };
}

const tabLabels = (host: HTMLElement) =>
  [...host.querySelectorAll("nav button")]
    .map((b) => b.getAttribute("aria-label"))
    .filter((l) => l !== "Add to today's journal");

describe("BottomBar", () => {
  it("has four tabs and no Pages tab", () => {
    const { host } = mount(null);
    expect(tabLabels(host)).toEqual(["Journal", "Search", "Tasks", "Reminders"]);
  });

  it("lights the tab the caller says is active", () => {
    const { host } = mount("reminders");
    const current = host.querySelectorAll('[aria-current="page"]');
    expect(current).toHaveLength(1);
    expect(current[0].getAttribute("aria-label")).toBe("Reminders");
  });

  it("tells the caller which tab was tapped", () => {
    const { host, onTab } = mount(null);
    host.querySelector<HTMLButtonElement>('[aria-label="Reminders"]')!.click();
    expect(onTab).toHaveBeenCalledWith("reminders");
  });

  it("tints the Reminders icon through currentColor as the tab lights and dims", () => {
    const lit = mount("reminders");
    const litBtn = lit.host.querySelector('[aria-label="Reminders"]')!;
    expect(litBtn.querySelector("svg")!.getAttribute("stroke")).toBe("currentColor");
    expect(litBtn.className).toContain("text-(--color-outl-accent)");
    dispose?.();
    document.body.innerHTML = "";
    const dim = mount("journal");
    const dimBtn = dim.host.querySelector('[aria-label="Reminders"]')!;
    expect(dimBtn.className).toContain("text-(--color-outl-fg-dim)");
    expect(dimBtn.className).not.toContain("text-(--color-outl-accent)");
  });
});
