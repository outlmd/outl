/**
 * The switcher is one search box over pages, tasks and blocks (issue
 * #364). Ranking is `@outl/shared/palette`'s and tested there; these
 * tests pin what this sheet owns: both backends fed from one query,
 * a heading per section, "N more", and the page-row gestures.
 */

import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const deletePage = vi.fn();
const listPages = vi.fn();
const searchPages = vi.fn();
const searchBlocks = vi.fn();
const togglePin = vi.fn();

vi.mock("@outl/shared/api/commands", () => ({
  listPages: (...a: unknown[]) => listPages(...a),
  searchPages: (...a: unknown[]) => searchPages(...a),
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

const page = (slug: string, extra: object = {}) => ({
  id: `id-${slug}`,
  slug,
  title: slug,
  kind: "page",
  ...extra,
});
const hit = (text: string, source_slug = "2026-09-01") => ({
  handle: "blk-abc123",
  id: "n1",
  text,
  source_slug,
});
const headings = () =>
  [...host.querySelectorAll("p")].map((p) => p.textContent?.trim());

beforeEach(() => {
  listPages.mockResolvedValue([page("inbox", { title: "Inbox" })]);
  searchPages.mockResolvedValue([page("inbox", { title: "Inbox" })]);
  searchBlocks.mockResolvedValue([hit("buy milk")]);
});

afterEach(() => {
  vi.useRealTimers();
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("PageSwitcher: one box", () => {
  it("lists pages by default and picks via onPick", async () => {
    const { onPick, onJumpToBlock } = mount();
    await settle();

    rowContaining("Inbox").click();

    expect(onPick).toHaveBeenCalledWith("inbox", "page");
    expect(onJumpToBlock).not.toHaveBeenCalled();
  });

  it("a_query_lists_pages_and_blocks_together", async () => {
    searchPages.mockResolvedValue([page("oura")]);
    searchBlocks.mockResolvedValue([hit("sleep score 88 #oura")]);
    mount();
    await settle();

    type(searchInput(), "oura");
    await settleDebounce();

    expect(headings()).toEqual(expect.arrayContaining(["Pages", "Blocks"]));
    expect(rowContaining("2026-09-01")).toBeTruthy();
    expect(rowContaining("sleep score 88 #oura")).toBeTruthy();
    expect(searchPages).toHaveBeenCalledWith("oura");
    expect(searchBlocks).toHaveBeenCalledWith("oura");
  });

  it("renders a section heading once per section", async () => {
    searchPages.mockResolvedValue([page("oura"), page("oura-log")]);
    searchBlocks.mockResolvedValue([hit("one oura"), hit("two oura")]);
    mount();
    await settle();

    type(searchInput(), "oura");
    await settleDebounce();

    expect(headings().filter((h) => h === "Pages")).toHaveLength(1);
    expect(headings().filter((h) => h === "Blocks")).toHaveLength(1);
  });

  it("lists a task hit under Tasks, before the plain blocks", async () => {
    searchBlocks.mockResolvedValue([hit("plain oura"), hit("TODO call oura")]);
    mount();
    await settle();

    type(searchInput(), "oura");
    await settleDebounce();

    const order = headings().filter((h) => h === "Tasks" || h === "Blocks");
    expect(order).toEqual(["Tasks", "Blocks"]);
  });

  it("shows how many hits a section is hiding", async () => {
    listPages.mockResolvedValue(
      Array.from({ length: 30 }, (_, i) => page(`page-${i}`)),
    );
    mount();
    await settle();

    expect(rowContaining("5 more")).toBeTruthy();
    expect(host.querySelectorAll("button[class*='min-w-0']")).toHaveLength(25);
  });

  it("expanding a section lists the rest without a second search", async () => {
    listPages.mockResolvedValue(
      Array.from({ length: 30 }, (_, i) => page(`page-${i}`)),
    );
    mount();
    await settle();

    rowContaining("5 more").click();
    await settle();

    expect(host.querySelectorAll("button[class*='min-w-0']")).toHaveLength(30);
    expect(listPages).toHaveBeenCalledTimes(1);
  });

  it("has no Pages/Blocks switch", async () => {
    mount();
    await settle();

    const pressed = [...document.querySelectorAll("button[aria-pressed]")];
    expect(pressed.some((b) => b.textContent?.trim().toLowerCase() === "blocks")).toBe(false);
  });

  it("holds blocks back until the query is long enough", async () => {
    mount();
    await settle();

    type(searchInput(), "m");
    await settleDebounce();

    expect(searchBlocks).not.toHaveBeenCalled();
  });

  it("tapping a block hit jumps to it, never treats it as a page pick", async () => {
    const { onPick, onJumpToBlock } = mount();
    await settle();

    type(searchInput(), "milk");
    await settleDebounce();
    rowContaining("buy milk").click();

    expect(onJumpToBlock).toHaveBeenCalledWith(hit("buy milk"));
    expect(onPick).not.toHaveBeenCalled();
  });

  it("Enter opens the first row", async () => {
    const { onPick } = mount();
    await settle();

    searchInput().dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );

    expect(onPick).toHaveBeenCalledWith("inbox", "page");
  });

  it("debounces: only the query left after typing stops reaches the block search", async () => {
    mount();
    await settle();

    const el = searchInput();
    type(el, "mi");
    type(el, "mil");
    type(el, "milk");
    await settleDebounce();

    expect(searchBlocks).toHaveBeenCalledTimes(1);
    expect(searchBlocks).toHaveBeenCalledWith("milk");
  });
});

describe("PageSwitcher: page rows", () => {
  it("a_page_row_still_long_presses_to_delete", async () => {
    vi.useFakeTimers();
    const confirm = vi.fn(() => false);
    window.confirm = confirm;
    const { onPick } = mount();
    await vi.advanceTimersByTimeAsync(0);

    const row = rowContaining("Inbox");
    row.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, clientX: 1, clientY: 1 }));
    await vi.advanceTimersByTimeAsync(600);
    row.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
    row.click();

    expect(confirm).toHaveBeenCalledWith(expect.stringContaining("Inbox"));
    // The click that follows a long-press must not also open the page.
    expect(onPick).not.toHaveBeenCalled();
  });

  it("offers a pin toggle for a page but not a journal, and toggling calls togglePin then re-searches", async () => {
    listPages.mockResolvedValue([
      page("inbox", { pinned: false }),
      page("2026-09-01", { title: "Sep 1", kind: "journal" }),
    ]);
    mount();
    await settle();

    const pinButtons = [
      ...document.querySelectorAll('button[aria-label="Pin page"]'),
    ];
    expect(pinButtons).toHaveLength(1);

    (pinButtons[0] as HTMLButtonElement).click();
    await settle();

    expect(togglePin).toHaveBeenCalledWith("id-inbox");
    expect(listPages).toHaveBeenCalledTimes(2);
  });

  it("sorts pinned pages first, everything else in the backend's order", async () => {
    listPages.mockResolvedValue([
      page("alpha", { title: "Alpha", pinned: false }),
      page("zeta", { title: "Zeta", pinned: true }),
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

describe("PageSwitcher: review round 1", () => {
  it("an empty query reads the whole graph, so a pinned page past the limit is listed", async () => {
    const many = Array.from({ length: 40 }, (_, i) => page(`2019-01-${i}`));
    listPages.mockResolvedValue([...many, page("projects/outl", { title: "Outl", pinned: true })]);
    mount();
    await settle();

    expect(rowContaining("Outl")).toBeTruthy();
    expect(searchPages).not.toHaveBeenCalled();
  });

  it("a non-empty query uses the ranked search, not the whole list", async () => {
    mount();
    await settle();
    listPages.mockClear();

    type(searchInput(), "inb");
    await settleDebounce();

    expect(searchPages).toHaveBeenCalledWith("inb");
    expect(listPages).not.toHaveBeenCalled();
  });

  it("a task row shows its state, and a done one is struck through", async () => {
    searchBlocks.mockResolvedValue([hit("DONE ship oura"), hit("TODO call oura")]);
    mount();
    await settle();

    type(searchInput(), "oura");
    await settleDebounce();

    const done = host.querySelector('[aria-label="done"]');
    expect(done).not.toBeNull();
    expect(host.querySelector('[aria-label="todo"]')).not.toBeNull();
    expect(rowContaining("ship oura").innerHTML).toContain("line-through");
  });

  it("an empty workspace says so instead of rendering a blank sheet", async () => {
    listPages.mockResolvedValue([]);
    mount();
    await settle();

    expect(host.textContent).toContain("No pages yet");
  });

  it("lists no action rows when the caller gave no onAction", async () => {
    mount();
    await settle();

    type(searchInput(), "sync");
    await settle();

    expect(host.querySelector("[data-action]")).toBeNull();
  });

  it("'N more' stays live while anything is hidden, growing a step per tap", async () => {
    listPages.mockResolvedValue(Array.from({ length: 60 }, (_, i) => page(`p-${i}`)));
    mount();
    await settle();
    const rows = () => host.querySelectorAll("button[class*='min-w-0']").length;

    expect(rows()).toBe(25);
    rowContaining("35 more").click();
    await settle();
    expect(rows()).toBe(50);
    rowContaining("10 more").click();
    await settle();
    expect(rows()).toBe(60);
    expect(host.textContent).not.toMatch(/\d+ more/);
  });

  it("'N more' names its section and is a full-size tap target", async () => {
    listPages.mockResolvedValue(Array.from({ length: 30 }, (_, i) => page(`p-${i}`)));
    mount();
    await settle();

    const more = rowContaining("5 more");
    expect(more.getAttribute("aria-label")).toBe("5 more pages");
    expect(more.className).toContain("min-h-11");
  });
});

/**
 * The empty-query browse list and the per-keystroke cost of the ranked
 * one — both new in this branch, both measured against a ~2,900-page
 * workspace.
 */
describe("PageSwitcher: browsing with no query", () => {
  it("opens Pages at 25 rows, so a recent page is reachable without typing", async () => {
    // Slug order puts every ISO journal day last, so a 3-row window
    // showed three pages from 2019 and hid the rest behind "N more".
    listPages.mockResolvedValue(Array.from({ length: 40 }, (_, i) => page(`2019-01-${i}`)));
    mount();
    await settle();

    expect(host.querySelectorAll("button[class*='min-w-0']")).toHaveLength(25);
    expect(rowContaining("15 more")).toBeTruthy();
  });

  it("keeps the short window once a query ranks the list", async () => {
    searchPages.mockResolvedValue(Array.from({ length: 10 }, (_, i) => page(`oura-${i}`)));
    mount();
    await settle();

    type(searchInput(), "oura");
    await settleDebounce();

    expect(host.querySelectorAll("button[class*='min-w-0']")).toHaveLength(3);
    expect(rowContaining("7 more")).toBeTruthy();
  });

  it("loads the empty query immediately — there are no keystrokes to coalesce", async () => {
    mount();
    await settle();

    expect(listPages).toHaveBeenCalledTimes(1);
  });

  it("debounces the page search too: one `search_pages` for a burst of keystrokes", async () => {
    // Every keystroke used to rebuild every `PageMeta` in the workspace
    // under the lock, three property reads and a `block_text` each.
    mount();
    await settle();

    const el = searchInput();
    type(el, "mi");
    type(el, "mil");
    type(el, "milk");
    expect(searchPages).not.toHaveBeenCalled();

    await settleDebounce();
    expect(searchPages).toHaveBeenCalledTimes(1);
    expect(searchPages).toHaveBeenCalledWith("milk");
  });
});
