import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createRoot, createSignal } from "solid-js";

import {
  cycleRegion,
  followSelection,
  installRegionKeys,
  rovingKeydown,
} from "./keyboard-reach";

function html(s: string) {
  document.body.innerHTML = s;
}

afterEach(() => {
  document.body.innerHTML = "";
});

const key = (k: string, init: KeyboardEventInit = {}) =>
  new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init });

describe("rovingKeydown", () => {
  it("walks a toolbar with arrows, wraps, and jumps with Home/End", () => {
    html(`<div id="t"><button>a</button><button disabled>x</button><button>b</button><button>c</button></div>`);
    const t = document.getElementById("t")!;
    const [a, , b, c] = Array.from(t.querySelectorAll("button"));
    t.addEventListener("keydown", (e) => rovingKeydown(e, "horizontal"));
    a.focus();
    a.dispatchEvent(key("ArrowRight"));
    expect(document.activeElement).toBe(b);
    b.dispatchEvent(key("End"));
    expect(document.activeElement).toBe(c);
    c.dispatchEvent(key("ArrowRight"));
    expect(document.activeElement).toBe(a);
    a.dispatchEvent(key("ArrowLeft"));
    expect(document.activeElement).toBe(c);
    c.dispatchEvent(key("Home"));
    expect(document.activeElement).toBe(a);
  });

  it("uses Up/Down for a vertical list and ignores the other axis", () => {
    html(`<nav id="n"><a href="#1">1</a><a href="#2">2</a></nav>`);
    const n = document.getElementById("n")!;
    const [one, two] = Array.from(n.querySelectorAll("a"));
    n.addEventListener("keydown", (e) => rovingKeydown(e, "vertical"));
    one.focus();
    one.dispatchEvent(key("ArrowRight"));
    expect(document.activeElement).toBe(one);
    one.dispatchEvent(key("ArrowDown"));
    expect(document.activeElement).toBe(two);
  });

  it("leaves text fields alone so the caret still moves", () => {
    html(`<div id="t"><input /><button>b</button></div>`);
    const t = document.getElementById("t")!;
    const input = t.querySelector("input")!;
    t.addEventListener("keydown", (e) => rovingKeydown(e, "vertical"));
    input.focus();
    const ev = key("ArrowDown");
    input.dispatchEvent(ev);
    expect(document.activeElement).toBe(input);
    expect(ev.defaultPrevented).toBe(false);
  });

  it("walks from a control beside the item it belongs to, not from row 0", () => {
    // `TrashPanel`'s shape: the row is the stop and the Restore button
    // is its *sibling* inside a flex wrapper. Focus lands on that button
    // when it is clicked (WebKitGTK) and a refused mutation leaves it
    // there, so the walk has to resolve it back to its row — `indexOf`
    // answered -1 and `(-1 + 1) % len` sent ArrowDown to row 0.
    html(`<div id="l">
      <div class="wrap"><div data-row tabindex="0">a</div><button>ra</button></div>
      <div class="wrap"><div data-row tabindex="0">b</div><button>rb</button></div>
      <div class="wrap"><div data-row tabindex="0">c</div><button>rc</button></div>
    </div>`);
    const l = document.getElementById("l")!;
    l.addEventListener("keydown", (e) => rovingKeydown(e, "vertical", { items: "[data-row]" }));
    const rows = Array.from(l.querySelectorAll<HTMLElement>("[data-row]"));
    const buttons = Array.from(l.querySelectorAll("button"));
    const landed = () => rows.indexOf(document.activeElement as HTMLElement);

    buttons[1].focus();
    buttons[1].dispatchEvent(key("ArrowDown"));
    expect(landed()).toBe(2);
    buttons[1].focus();
    buttons[1].dispatchEvent(key("ArrowUp"));
    expect(landed()).toBe(0);
  });

  it("walks from a control nested inside the item too", () => {
    // The other containment shape, which plain `closest` already
    // covered. Both go through one resolver so a panel cannot pick the
    // arrangement that happens to work.
    html(`<div id="l">
      <div data-row tabindex="0">a<button>x</button></div>
      <div data-row tabindex="0">b<button>y</button></div>
    </div>`);
    const l = document.getElementById("l")!;
    l.addEventListener("keydown", (e) => rovingKeydown(e, "vertical", { items: "[data-row]" }));
    const rows = Array.from(l.querySelectorAll<HTMLElement>("[data-row]"));
    // ArrowDown off row **0**, deliberately: an unresolved index is -1,
    // and `(-1 - 1 + len) % len` is `len - 2`, which for several row
    // counts happens to be the right answer for ArrowUp. ArrowDown's
    // `(-1 + 1) % len` is always 0, so only row 0 moving to row 1 can
    // tell a resolved index from an unresolved one.
    const nested = l.querySelectorAll("button")[0];

    nested.focus();
    nested.dispatchEvent(key("ArrowDown"));
    expect(rows.indexOf(document.activeElement as HTMLElement)).toBe(1);
  });

  it("still enters at the top from outside every item", () => {
    // Focus on the container itself belongs to no item, and ArrowDown
    // there should enter the list rather than do nothing. That is
    // today's `-1 → 0`, kept deliberately: the resolver stops *before*
    // the container precisely so it cannot answer "the first item" for
    // focus that is simply not in one.
    html(`<div id="l" tabindex="0">
      <div data-row tabindex="0">a</div><div data-row tabindex="0">b</div>
    </div>`);
    const l = document.getElementById("l")!;
    l.addEventListener("keydown", (e) => rovingKeydown(e, "vertical", { items: "[data-row]" }));
    const rows = Array.from(l.querySelectorAll<HTMLElement>("[data-row]"));
    l.focus();
    l.dispatchEvent(key("ArrowDown"));
    expect(document.activeElement).toBe(rows[0]);
  });
});

describe("regions", () => {
  const layout = `
    <aside data-outl-region="sidebar"><button id="s">s</button></aside>
    <main data-outl-region="outline" tabindex="-1"><p>text</p></main>
    <div data-outl-region="chrome"><button id="c">c</button></div>`;

  it("F6 cycles sidebar, outline, chrome and Shift+F6 goes back", () => {
    html(layout);
    document.getElementById("s")!.focus();
    cycleRegion(1);
    expect(document.activeElement?.getAttribute("data-outl-region")).toBe("outline");
    cycleRegion(1);
    expect(document.activeElement?.id).toBe("c");
    cycleRegion(1);
    expect(document.activeElement?.id).toBe("s");
    cycleRegion(-1);
    expect(document.activeElement?.id).toBe("c");
  });

  it("installRegionKeys wires F6 and sends Esc from chrome back to the outline", () => {
    html(layout);
    const off = installRegionKeys();
    document.getElementById("c")!.focus();
    document.activeElement!.dispatchEvent(key("Escape"));
    expect(document.activeElement?.getAttribute("data-outl-region")).toBe("outline");
    document.activeElement!.dispatchEvent(key("F6", { shiftKey: true }));
    expect(document.activeElement?.id).toBe("s");
    off();
    document.activeElement!.dispatchEvent(key("F6"));
    expect(document.activeElement?.id).toBe("s");
  });
});

describe("rovingKeydown in panels and forms", () => {
  it("moves between checkboxes and buttons, and leaves text fields alone", () => {
    html(`<div id="f"><input type="checkbox" id="c"/><input type="text" id="t"/><button id="b">b</button></div>`);
    const f = document.getElementById("f")!;
    f.addEventListener("keydown", (e) => rovingKeydown(e, "vertical"));
    const c = document.getElementById("c")!;
    c.focus();
    c.dispatchEvent(key("ArrowDown"));
    expect(document.activeElement?.id).toBe("b");
    const t = document.getElementById("t")!;
    t.focus();
    t.dispatchEvent(key("ArrowDown"));
    expect(document.activeElement).toBe(t);
  });

  it("with includeFields, arrows can reach a select in a form", () => {
    html(`<div id="f"><input type="checkbox" id="c"/><select id="s"><option>a</option></select></div>`);
    const f = document.getElementById("f")!;
    f.addEventListener("keydown", (e) => rovingKeydown(e, "vertical", { includeFields: true }));
    const c = document.getElementById("c")!;
    c.focus();
    c.dispatchEvent(key("ArrowDown"));
    expect(document.activeElement?.id).toBe("s");
  });
});

describe("followSelection", () => {
  /**
   * Moving the cursor has to move the page with it. Without this, `j`
   * walked the selection off the bottom and kept going: on an 80-row
   * page, 60 presses left the view on row 1 with the cursor ~55 rows
   * below the fold, so pressing `j` looked like it did nothing.
   *
   * `scrollIntoView` is a stub in the test DOM, so these pin the call
   * rather than the pixels. The lab battery measures the real scroll,
   * and it also covers the "row not rendered" path: a unit test for
   * that cannot fail here, because the throw would happen inside the
   * microtask and never reach an assertion.
   */
  let calls: Array<ScrollIntoViewOptions | boolean | undefined>;
  let dispose: (() => void) | undefined;

  beforeEach(() => {
    calls = [];
    Element.prototype.scrollIntoView = function (opts?) {
      calls.push(opts);
    };
  });

  afterEach(() => {
    dispose?.();
    dispose = undefined;
    document.body.innerHTML = "";
  });

  function mount(selected: () => string | null) {
    dispose = createRoot((d) => {
      followSelection(selected);
      return d;
    });
  }

  const flush = () => new Promise((r) => queueMicrotask(() => r(null)));

  it("brings the newly selected block into view", async () => {
    html(`<div data-block-id="a"></div><div data-block-id="b"></div>`);
    const [id, setId] = createSignal<string | null>("a");
    mount(id);
    await flush();
    calls = [];

    setId("b");
    await flush();
    expect(calls).toHaveLength(1);
    // `nearest`, never `center`: centring on every keypress makes the
    // page lurch under someone stepping one row at a time.
    expect(calls[0]).toEqual({ block: "nearest" });
  });

  it("follows every move, not just the first", async () => {
    html(
      `<div data-block-id="a"></div><div data-block-id="b"></div>` +
        `<div data-block-id="c"></div>`,
    );
    const [id, setId] = createSignal<string | null>("a");
    mount(id);
    await flush();
    calls = [];

    setId("b");
    await flush();
    setId("c");
    await flush();
    // One per move. A version that read the id once and never again
    // would scroll to the first row and leave the cursor behind — the
    // defect this exists to catch.
    expect(calls).toHaveLength(2);
  });
});
