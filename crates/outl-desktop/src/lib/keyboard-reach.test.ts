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
