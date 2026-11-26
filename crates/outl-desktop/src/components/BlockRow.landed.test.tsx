/**
 * The landing flash on a block a backlink breadcrumb jump selected:
 * `data-landed` is on for the `landedMarks` hold and off after it.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";

vi.mock("@outl/shared/api/commands", () => ({
  openRef: vi.fn(),
  pluginList: vi.fn().mockResolvedValue([]),
  searchEmojis: vi.fn().mockResolvedValue([]),
  searchPages: vi.fn().mockResolvedValue([]),
  searchPersons: vi.fn().mockResolvedValue([]),
  listTemplates: vi.fn().mockResolvedValue([]),
}));
vi.mock("@outl/shared/plugins/transformer-registry", () => ({
  runTransform: vi.fn(),
  transformerFor: vi.fn(() => null),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({ readText: vi.fn() }));

import { landedMarks } from "../lib/store";
import { BlockRow, type BlockCallbacks } from "./BlockRow";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  vi.useRealTimers();
  document.body.innerHTML = "";
});

const block: BlockNode = {
  id: "landed-1",
  text: "hi",
  todo: null,
  tokens: [],
  collapsed: false,
  properties: [],
  children: [],
};

describe("BlockRow landing flash", () => {
  it("turns data-landed on for the hold and off after it", () => {
    vi.useFakeTimers();
    const host = document.createElement("div");
    document.body.appendChild(host);
    dispose = render(
      () => (
        <BlockRow
          block={block}
          depth={0}
          editingId={null}
          visualSet={null}
          cb={{} as BlockCallbacks}
        />
      ),
      host,
    );
    const flash = () => host.querySelector<HTMLElement>("[data-landed]");
    expect(flash()?.dataset.landed).toBe("false");
    expect(flash()?.getAttribute("aria-hidden")).toBe("true");

    landedMarks.mark("landed-1");
    expect(flash()?.dataset.landed).toBe("true");

    vi.advanceTimersByTime(1200);
    expect(flash()?.dataset.landed).toBe("false");
  });
});
