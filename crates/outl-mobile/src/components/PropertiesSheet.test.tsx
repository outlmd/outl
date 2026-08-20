/**
 * The Properties sheet's three load-bearing promises:
 *
 * 1. adding a property costs **taps, not typing** — the key chips come
 *    from the workspace's own catalogue;
 * 2. deleting is an action, not the undiscoverable "empty the field"
 *    gesture (it still writes an empty value, which is what the
 *    backend means by delete — the user never has to know);
 * 3. the page's own properties are reachable and writable, which no
 *    GUI client could do at all before issue #13.
 *
 * The value field's `[[` picker is covered too, because that path
 * carries 87% of real property values.
 */

import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const setBlockProperty = vi.fn();
const setPageProperty = vi.fn();
const knownPropertyKeys = vi.fn();
const searchPages = vi.fn();

vi.mock("@outl/shared/api/commands", () => ({
  setBlockProperty: (...a: unknown[]) => setBlockProperty(...a),
  searchPages: (...a: unknown[]) => searchPages(...a),
}));

vi.mock("../lib/api", () => ({
  knownPropertyKeys: (...a: unknown[]) => knownPropertyKeys(...a),
  setPageProperty: (...a: unknown[]) => setPageProperty(...a),
}));

import { PropertiesSheet, type PropertyScope } from "./PropertiesSheet";

const PAGE_ID = "page-1";
const BLOCK_ID = "blk-1";

function pageView() {
  return {
    page: {
      id: PAGE_ID,
      slug: "inbox",
      title: "Inbox",
      kind: "page" as const,
    },
    outline: [
      {
        id: BLOCK_ID,
        text: "a block",
        todo: null,
        tokens: [],
        collapsed: false,
        properties: [["icon", "🔥"]] as Array<[string, string]>,
        children: [],
      },
    ],
    backlinks: [],
    backlinks_order: "newest" as const,
    page_properties: [["type", "person"]] as Array<[string, string]>,
  };
}

let dispose: (() => void) | undefined;
let host: HTMLElement;

function mount(scope: PropertyScope, blockId: string | null = BLOCK_ID) {
  host = document.createElement("div");
  document.body.appendChild(host);
  const onView = vi.fn();
  const onMessage = vi.fn();
  dispose = render(
    () =>
      PropertiesSheet({
        blockId,
        scope,
        pageId: PAGE_ID,
        view: pageView() as never,
        onClose: () => {},
        onMessage,
        onView,
      }) as never,
    host,
  );
  return { onView, onMessage };
}

/** First button (anywhere in the sheet) whose visible text or
 *  `aria-label` is exactly `text`. */
function button(text: string): HTMLButtonElement {
  const hit = [...document.querySelectorAll("button")].find(
    (b) =>
      b.textContent?.trim() === text || b.getAttribute("aria-label") === text,
  );
  if (!hit) {
    throw new Error(
      `no button "${text}" — have: ${[...document.querySelectorAll("button")]
        .map((b) => `"${b.textContent?.trim()}"`)
        .join(", ")}`,
    );
  }
  return hit as HTMLButtonElement;
}

function input(label: string): HTMLInputElement {
  const el = document.querySelector(`input[aria-label="${label}"]`);
  if (!el) throw new Error(`no input labelled "${label}"`);
  return el as HTMLInputElement;
}

/** Type into an input the way the webview does, caret at the end. */
function type(el: HTMLInputElement, value: string) {
  el.value = value;
  el.selectionStart = value.length;
  el.selectionEnd = value.length;
  el.dispatchEvent(new Event("input", { bubbles: true }));
}

/**
 * Dispatch one pointer step at `clientX`. happy-dom has no
 * `PointerEvent`, and Solid does not delegate pointer events, so a
 * plain `Event` with the coordinates attached reaches the handler the
 * same way the webview's would.
 */
function pointer(el: HTMLElement, type: string, clientX: number) {
  const e = new Event(type, { bubbles: true });
  Object.assign(e, { clientX, clientY: 0, pointerId: 1, pointerType: "touch", button: 0 });
  el.dispatchEvent(e);
}

/** Let the queued promise chains (catalogue load, writes) settle. */
const settle = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  knownPropertyKeys.mockResolvedValue([
    { key: "icon", uses: 90 },
    { key: "related", uses: 40 },
    { key: "oura-date", uses: 12 },
  ]);
  setBlockProperty.mockResolvedValue(pageView());
  setPageProperty.mockResolvedValue(pageView());
  searchPages.mockResolvedValue([
    { id: "p2", slug: "avelino-outl", title: "avelino/outl", kind: "page" },
  ]);
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

describe("PropertiesSheet — adding without the keyboard", () => {
  it("offers the workspace's keys as chips, minus the ones already set", async () => {
    mount("block");
    await settle();
    button("Add property").click();

    // `icon` is already on the block: tapping it would mean edit, and
    // edit is the row. Offering both makes one key look like two.
    expect(() => button("icon")).toThrow();
    expect(button("related")).toBeTruthy();
    expect(button("oura-date")).toBeTruthy();
  });

  it("a chip tap plus a value is the whole flow — two taps, no key typed", async () => {
    mount("block");
    await settle();
    button("Add property").click();
    button("oura-date").click();

    type(input("Value for oura-date"), "2026-08-20");
    button("Save").click();
    await settle();

    expect(setBlockProperty).toHaveBeenCalledWith(
      PAGE_ID,
      BLOCK_ID,
      "oura-date",
      "2026-08-20",
    );
  });

  it("`Other…` opens the keyboard and strips the `::` a user copies", async () => {
    mount("block");
    await settle();
    button("Add property").click();
    button("Other…").click();

    type(input("Property key"), "gemini-msg::");
    button("Next").click();

    // The key reached the value step normalised, so the write cannot
    // create a `gemini-msg::` key that renders as `gemini-msg:::: v`.
    type(input("Value for gemini-msg"), "hi");
    button("Save").click();
    await settle();

    expect(setBlockProperty).toHaveBeenCalledWith(
      PAGE_ID,
      BLOCK_ID,
      "gemini-msg",
      "hi",
    );
  });

  it("refuses to write a key that is only punctuation", async () => {
    const { onMessage } = mount("block");
    await settle();
    button("Add property").click();
    button("Other…").click();
    type(input("Property key"), "::");
    // `Next` stays disabled, so the only way through is the value step
    // never opening — nothing is written and nothing is lost.
    expect(button("Next").disabled).toBe(true);
    expect(setBlockProperty).not.toHaveBeenCalled();
    expect(onMessage).not.toHaveBeenCalled();
  });
});

describe("PropertiesSheet — editing and deleting", () => {
  it("tapping a row edits its value in place", async () => {
    mount("block");
    await settle();
    button("Edit icon").click();

    const field = input("Value for icon");
    expect(field.value).toBe("🔥");
    type(field, "🧊");
    button("Save").click();
    await settle();

    expect(setBlockProperty).toHaveBeenCalledWith(
      PAGE_ID,
      BLOCK_ID,
      "icon",
      "🧊",
    );
  });

  it("Delete is an explicit action that writes the backend's empty value", async () => {
    mount("block");
    await settle();
    button("Edit icon").click();
    button("Delete").click();
    await settle();

    expect(setBlockProperty).toHaveBeenCalledWith(PAGE_ID, BLOCK_ID, "icon", "");
  });

  it("swipe-left on a row deletes it — the gesture iOS already taught", async () => {
    mount("block");
    await settle();
    // The row lives inside `<SwipeRow>`; the draggable surface is the
    // button's parent. Drag it past the 96px commit threshold.
    const surface = button("Edit icon").parentElement as HTMLElement;
    pointer(surface, "pointerdown", 300);
    pointer(surface, "pointermove", 260); // past the 8px capture gate
    pointer(surface, "pointermove", 150); // past the threshold
    pointer(surface, "pointerup", 150);
    await settle();

    expect(setBlockProperty).toHaveBeenCalledWith(PAGE_ID, BLOCK_ID, "icon", "");
  });

  it("offers no Delete for a property that does not exist yet", async () => {
    mount("block");
    await settle();
    button("Add property").click();
    button("related").click();

    expect(() => button("Delete")).toThrow();
  });

  it("surfaces a rejected write instead of repainting as if it landed", async () => {
    const { onMessage, onView } = mount("block");
    await settle();
    setBlockProperty.mockRejectedValueOnce(new Error("block is not in the tree"));
    button("Edit icon").click();
    type(input("Value for icon"), "x");
    button("Save").click();
    await settle();

    expect(onMessage).toHaveBeenCalledWith("block is not in the tree");
    expect(onView).not.toHaveBeenCalled();
  });
});

describe("PropertiesSheet — page properties", () => {
  it("lists the page's own metadata and writes through set_page_property", async () => {
    mount("page", null);
    await settle();
    button("Edit type").click();
    type(input("Value for type"), "project");
    button("Save").click();
    await settle();

    expect(setPageProperty).toHaveBeenCalledWith(PAGE_ID, "type", "project");
    expect(setBlockProperty).not.toHaveBeenCalled();
  });

  it("has no Block/Page switch when there is no block behind it", async () => {
    mount("page", null);
    await settle();
    expect(() => button("block")).toThrow();
  });

  it("switches a block sheet over to the page's properties", async () => {
    mount("block");
    await settle();
    button("page").click();

    // The page's property, not the block's, is what the list shows now.
    expect(button("Edit type")).toBeTruthy();
    expect(() => button("Edit icon")).toThrow();
  });
});

describe("PropertiesSheet — the value is usually a page link", () => {
  it("`[[` opens the page picker and the chip inserts a real ref", async () => {
    mount("block");
    await settle();
    button("Add property").click();
    button("related").click();

    const field = input("Value for related");
    type(field, "[[out");
    await settle();

    expect(searchPages).toHaveBeenCalledWith("out");
    button("avelino/outl").click();
    button("Save").click();
    await settle();

    expect(setBlockProperty).toHaveBeenCalledWith(
      PAGE_ID,
      BLOCK_ID,
      "related",
      "[[avelino/outl]]",
    );
  });

  it("`Link a page…` types the `[[` and queries straight away", async () => {
    mount("block");
    await settle();
    button("Add property").click();
    button("related").click();
    button("Link a page…").click();
    await settle();

    expect(searchPages).toHaveBeenCalledWith("");
  });
});
