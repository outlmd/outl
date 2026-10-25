/** The switcher as the phone's command palette: `>` lists actions only. */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("@outl/shared/api/commands", () => ({
  listPages: () => Promise.resolve([{ slug: "alpha", title: "Alpha", kind: "page" }]),
  searchPages: () => Promise.resolve([{ slug: "alpha", title: "Alpha", kind: "page" }]),
  deletePage: vi.fn(),
  searchBlocks: () => Promise.resolve([]),
  togglePin: vi.fn(),
}));

import { PageSwitcher } from "./PageSwitcher";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  document.body.innerHTML = "";
});

function mount() {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const onAction = vi.fn();
  const onClose = vi.fn();
  dispose = render(
    () =>
      PageSwitcher({
        open: true,
        currentSlug: null,
        onClose,
        onPick: vi.fn(),
        onJumpToBlock: vi.fn(),
        onAction,
      }) as never,
    host,
  );
  const input = host.querySelector("input") as HTMLInputElement;
  const type = (v: string) => {
    input.value = v;
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
  };
  return { host, input, type, onAction, onClose };
}

const flush = () => new Promise((r) => setTimeout(r, 0));

describe("PageSwitcher actions", () => {
  it("> hides pages and lists actions; tapping one closes and runs it", async () => {
    const { host, type, onAction, onClose } = mount();
    await flush();
    type(">remind");
    await flush();
    expect(host.textContent).not.toContain("Alpha");
    const row = host.querySelector('[data-action="OpenReminders"]') as HTMLButtonElement;
    row.click();
    expect(onClose).toHaveBeenCalled();
    expect(onAction).toHaveBeenCalledWith("OpenReminders");
  });

  it("Enter on a > query runs the first action", async () => {
    const { input, type, onAction } = mount();
    type("> settings");
    input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(onAction).toHaveBeenCalledWith("OpenSettings");
  });

  it("a plain query shows matching actions under the pages", async () => {
    const { host, type } = mount();
    type("sync");
    await flush();
    expect(host.querySelector('[data-action="SyncNow"]')).not.toBeNull();
  });

});
