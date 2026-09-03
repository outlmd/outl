/**
 * The page switcher's "Blocks" mode (issue #19, RFC 0254 phase 2) is
 * the only surface on mobile that searches block *content* across
 * the whole workspace — everything else here already had "Pages"
 * mode coverage indirectly through manual testing, so these tests
 * are scoped to the new half: switching modes searches the right
 * backend, hits render with enough context to tell them apart, and
 * tapping (or `Enter`-ing) a hit reaches `onJumpToBlock`, never
 * `onPick` — the two callers navigate differently because a
 * `BlockHit` carries no `kind`.
 */

import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const listPages = vi.fn();
const deletePage = vi.fn();
const searchBlocks = vi.fn();
const togglePin = vi.fn();

vi.mock("@outl/shared/api/commands", () => ({
  listPages: (...a: unknown[]) => listPages(...a),
  deletePage: (...a: unknown[]) => deletePage(...a),
  searchBlocks: (...a: unknown[]) => searchBlocks(...a),
  togglePin: (...a: unknown[]) => togglePin(...a),
}));

import { PageSwitcher } from "./PageSwitcher";

let dispose: (() => void) | undefined;
let host: HTMLElement;

function mount() {
  host = document.createElement("div");
  document.body.appendChild(host);
  const onPick = vi.fn();
  const onJumpToBlock = vi.fn();
  const onClose = vi.fn();
  dispose = render(
    () =>
      PageSwitcher({
        open: true,
        currentSlug: null,
        onClose,
        onPick,
        onJumpToBlock,
      }) as never,
    host,
  );
  return { onPick, onJumpToBlock, onClose };
}

/** Exact-text button lookup — for the "Pages" / "Blocks" segment
 *  control, whose rows are single text nodes. */
function segment(text: string): HTMLButtonElement {
  const hit = [...document.querySelectorAll("button")].find(
    (b) => b.textContent?.trim() === text,
  );
  if (!hit) throw new Error(`no segment "${text}"`);
  return hit as HTMLButtonElement;
}

/** Substring button lookup — a page/block row has two stacked
 *  `<span>`s (title/text + slug), so an exact match never hits. */
function rowContaining(text: string): HTMLButtonElement {
  const hit = [...document.querySelectorAll("button")].find((b) =>
    b.textContent?.includes(text),
  );
  if (!hit) {
    throw new Error(
      `no row containing "${text}" — have: ${[
        ...document.querySelectorAll("button"),
      ]
        .map((b) => `"${b.textContent?.trim()}"`)
        .join(", ")}`,
    );
  }
  return hit as HTMLButtonElement;
}

function searchInput(): HTMLInputElement {
  const el = host.querySelector('input[type="text"]');
  if (!el) throw new Error("no search input");
  return el as HTMLInputElement;
}

/** Type into the search box the way the webview does. */
function type(el: HTMLInputElement, value: string) {
  el.value = value;
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

/** Let a queued promise chain settle. */
const settle = () => new Promise((r) => setTimeout(r, 0));
/** Clear `PageSwitcher`'s 150ms block-search debounce, plus the
 *  microtask the mocked `searchBlocks` resolves on. */
const settleDebounce = () => new Promise((r) => setTimeout(r, 200));

beforeEach(() => {
  listPages.mockResolvedValue([
    { id: "p1", slug: "inbox", title: "Inbox", kind: "page" },
  ]);
  searchBlocks.mockResolvedValue([
    {
      handle: "blk-abc123",
      text: "buy milk",
      source_slug: "2026-09-01",
    },
  ]);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("PageSwitcher — Pages mode (unchanged)", () => {
  it("lists pages by default and picks via onPick", async () => {
    const { onPick, onJumpToBlock } = mount();
    await settle();

    rowContaining("Inbox").click();

    expect(onPick).toHaveBeenCalledWith("inbox", "page");
    expect(onJumpToBlock).not.toHaveBeenCalled();
  });
});

describe("PageSwitcher — pin toggle (RFC 0254 phase 4b, TogglePin)", () => {
  it("offers a pin toggle for a page but not a journal, and toggling calls togglePin then refetches", async () => {
    listPages.mockResolvedValue([
      { id: "p1", slug: "inbox", title: "Inbox", kind: "page", pinned: false },
      { id: "j1", slug: "2026-09-01", title: "Sep 1", kind: "journal" },
    ]);
    mount();
    await settle();

    const pinButtons = [
      ...document.querySelectorAll('button[aria-label="Pin page"]'),
    ];
    expect(pinButtons).toHaveLength(1);

    (pinButtons[0] as HTMLButtonElement).click();
    await settle();

    expect(togglePin).toHaveBeenCalledWith("p1");
    // Initial fetch + the refetch `handleTogglePin` triggers.
    expect(listPages).toHaveBeenCalledTimes(2);
  });

  it("sorts pinned pages first, everything else in listPages' order", async () => {
    listPages.mockResolvedValue([
      { id: "p1", slug: "alpha", title: "Alpha", kind: "page", pinned: false },
      { id: "p2", slug: "zeta", title: "Zeta", kind: "page", pinned: true },
    ]);
    mount();
    await settle();

    const rows = [...document.querySelectorAll("button")].map(
      (b) => b.textContent ?? "",
    );
    const zetaIdx = rows.findIndex((t) => t.includes("Zeta"));
    const alphaIdx = rows.findIndex((t) => t.includes("Alpha"));
    expect(zetaIdx).toBeGreaterThanOrEqual(0);
    expect(zetaIdx).toBeLessThan(alphaIdx);
  });
});

describe("PageSwitcher — Blocks mode (issue #19)", () => {
  it("switching to Blocks searches block content, not the page list", async () => {
    mount();
    await settle();

    segment("blocks").click();
    type(searchInput(), "milk");
    await settleDebounce();

    expect(searchBlocks).toHaveBeenCalledWith("milk");
    // The hit's own text and its hosting page both render — the
    // second is the only context telling two same-named blocks on
    // different pages apart.
    expect(rowContaining("buy milk")).toBeTruthy();
    expect(rowContaining("2026-09-01")).toBeTruthy();
  });

  it("tapping a block hit jumps to it, never treats it as a page pick", async () => {
    const { onPick, onJumpToBlock } = mount();
    await settle();

    segment("blocks").click();
    type(searchInput(), "milk");
    await settleDebounce();

    rowContaining("buy milk").click();

    expect(onJumpToBlock).toHaveBeenCalledWith({
      handle: "blk-abc123",
      text: "buy milk",
      source_slug: "2026-09-01",
    });
    expect(onPick).not.toHaveBeenCalled();
  });

  it("Enter opens the first block hit, same convention as Pages mode", async () => {
    const { onJumpToBlock } = mount();
    await settle();

    segment("blocks").click();
    type(searchInput(), "milk");
    await settleDebounce();

    searchInput().dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );

    expect(onJumpToBlock).toHaveBeenCalledWith({
      handle: "blk-abc123",
      text: "buy milk",
      source_slug: "2026-09-01",
    });
  });

  it("debounces: only the query left after typing stops reaches the backend", async () => {
    mount();
    await settle();

    segment("blocks").click();
    // Switching to Blocks itself fires an empty-query search
    // (backend default: most-recent blocks) — let that resolve
    // before typing, so it doesn't get counted below.
    await settleDebounce();
    searchBlocks.mockClear();

    const el = searchInput();
    type(el, "m");
    type(el, "mi");
    type(el, "milk");
    await settleDebounce();

    expect(searchBlocks).toHaveBeenCalledTimes(1);
    expect(searchBlocks).toHaveBeenCalledWith("milk");
  });
});
