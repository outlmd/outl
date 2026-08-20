/**
 * `<PropertyEditor />` — the two verbs the GUI never had.
 *
 * Before issue #13 the desktop could edit a property that already
 * existed and nothing else: creating the first one meant the TUI or
 * the `.md`, and deleting was an invisible gesture (empty the value).
 * These pin create, delete, and the key completion that makes create
 * usable in a graph where a dozen keys cover almost everything.
 */

import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PropertyEditor } from "./PropertyEditor";

const knownPropertyKeys = vi.fn();
vi.mock("../lib/api", () => ({
  knownPropertyKeys: (...a: unknown[]) => knownPropertyKeys(...a),
}));

const searchPages = vi.fn();
vi.mock("@outl/shared/api/commands", () => ({
  searchPages: (...a: unknown[]) => searchPages(...a),
}));

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
  knownPropertyKeys.mockReset();
  searchPages.mockReset();
});

function mount(node: () => unknown): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const d = render(node as () => any, host);
  dispose = () => {
    d();
    host.remove();
  };
  return host;
}

/** Settle the microtask queue the catalogue fetch resolves on. */
const flush = () => new Promise((r) => setTimeout(r, 0));

function byLabel(host: HTMLElement, label: string): HTMLElement {
  const el = host.querySelector(`[aria-label="${label}"]`);
  if (!el) throw new Error(`no element labelled ${label}`);
  return el as HTMLElement;
}

function type(input: HTMLInputElement, value: string) {
  input.value = value;
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function press(el: HTMLElement, key: string) {
  el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
}

const PROPS = [["priority", "high"]] as ReadonlyArray<
  readonly [string, string]
>;

describe("creating a property", () => {
  it("writes the key and value the user typed", async () => {
    knownPropertyKeys.mockResolvedValue([]);
    const onCommit = vi.fn().mockResolvedValue(undefined);
    const host = mount(() => (
      <PropertyEditor properties={[]} onCommit={onCommit} />
    ));

    (byLabel(host, "Add property") as HTMLButtonElement).click();
    await flush();

    type(byLabel(host, "New property key") as HTMLInputElement, "status");
    const value = byLabel(host, "New property value") as HTMLInputElement;
    type(value, "draft");
    press(value, "Enter");
    await flush();

    expect(onCommit).toHaveBeenCalledWith("status", "draft");
  });

  it("treats an empty key as a cancel, not a backend error", async () => {
    knownPropertyKeys.mockResolvedValue([]);
    const onCommit = vi.fn();
    const host = mount(() => (
      <PropertyEditor properties={[]} onCommit={onCommit} />
    ));

    (byLabel(host, "Add property") as HTMLButtonElement).click();
    await flush();
    press(byLabel(host, "New property value"), "Enter");
    await flush();

    expect(onCommit).not.toHaveBeenCalled();
    expect(host.querySelector('[aria-label="New property key"]')).toBeNull();
  });

  it("opens from the host signal, and reports closing so a second press re-fires", async () => {
    knownPropertyKeys.mockResolvedValue([]);
    const onAddOpenChange = vi.fn();
    const host = mount(() => (
      <PropertyEditor
        properties={[]}
        onCommit={vi.fn()}
        addOpen
        onAddOpenChange={onAddOpenChange}
      />
    ));
    await flush();

    // The chord opened it without anybody clicking `+`.
    const key = byLabel(host, "New property key");
    press(key, "Escape");

    expect(onAddOpenChange).toHaveBeenCalledWith(false);
    expect(host.querySelector('[aria-label="New property key"]')).toBeNull();
  });

  it("surfaces a rejected write instead of repainting as if it landed", async () => {
    knownPropertyKeys.mockResolvedValue([]);
    const onError = vi.fn();
    const host = mount(() => (
      <PropertyEditor
        properties={[]}
        onCommit={() => Promise.reject(new Error("backend said no"))}
        onError={onError}
      />
    ));

    (byLabel(host, "Add property") as HTMLButtonElement).click();
    await flush();
    type(byLabel(host, "New property key") as HTMLInputElement, "status");
    press(byLabel(host, "New property value"), "Enter");
    await flush();

    expect(onError).toHaveBeenCalledWith("backend said no");
  });
});

describe("deleting a property", () => {
  it("clears the key with an empty value", async () => {
    const onCommit = vi.fn().mockResolvedValue(undefined);
    const host = mount(() => (
      <PropertyEditor properties={PROPS} onCommit={onCommit} />
    ));

    (byLabel(host, "Delete property priority") as HTMLButtonElement).click();
    await flush();

    expect(onCommit).toHaveBeenCalledWith("priority", "");
  });

  it("names the noun the host gave it, so the page row doesn't say 'property'", () => {
    const host = mount(() => (
      <PropertyEditor
        properties={PROPS}
        noun="page property"
        onCommit={vi.fn()}
      />
    ));

    expect(
      host.querySelector('[aria-label="Delete page property priority"]'),
    ).not.toBeNull();
  });
});

describe("key autocomplete", () => {
  it("offers the workspace's keys, most-used first", async () => {
    knownPropertyKeys.mockResolvedValue([
      { key: "related", uses: 40 },
      { key: "status", uses: 12 },
    ]);
    const host = mount(() => (
      <PropertyEditor properties={[]} onCommit={vi.fn()} />
    ));

    (byLabel(host, "Add property") as HTMLButtonElement).click();
    await flush();

    const options = [...host.querySelectorAll('[role="option"]')].map((o) =>
      (o.textContent ?? "").trim(),
    );
    expect(options[0]).toContain("related");
    expect(options[1]).toContain("status");
  });

  it("filters as the user types and accepts the highlight on Enter", async () => {
    knownPropertyKeys.mockResolvedValue([
      { key: "related", uses: 40 },
      { key: "status", uses: 12 },
    ]);
    const onCommit = vi.fn().mockResolvedValue(undefined);
    const host = mount(() => (
      <PropertyEditor properties={[]} onCommit={onCommit} />
    ));

    (byLabel(host, "Add property") as HTMLButtonElement).click();
    await flush();

    const key = byLabel(host, "New property key") as HTMLInputElement;
    type(key, "sta");
    const options = [...host.querySelectorAll('[role="option"]')];
    expect(options).toHaveLength(1);

    // Enter accepts the completion; it must not commit a half-typed
    // key, so the property still needs the value field's Enter.
    press(key, "Enter");
    expect(onCommit).not.toHaveBeenCalled();

    const value = byLabel(host, "New property value") as HTMLInputElement;
    type(value, "draft");
    press(value, "Enter");
    await flush();
    expect(onCommit).toHaveBeenCalledWith("status", "draft");
  });

  it("never offers a key the block already carries", async () => {
    knownPropertyKeys.mockResolvedValue([
      { key: "priority", uses: 99 },
      { key: "status", uses: 12 },
    ]);
    const host = mount(() => (
      <PropertyEditor properties={PROPS} onCommit={vi.fn()} />
    ));

    (byLabel(host, "Add property") as HTMLButtonElement).click();
    await flush();

    const options = [...host.querySelectorAll('[role="option"]')].map((o) =>
      (o.textContent ?? "").trim(),
    );
    expect(options.join(" ")).not.toContain("priority");
    expect(options.join(" ")).toContain("status");
  });

  it("still lets the user type a key when the catalogue call fails", async () => {
    knownPropertyKeys.mockRejectedValue(new Error("no workspace"));
    const onError = vi.fn();
    const onCommit = vi.fn().mockResolvedValue(undefined);
    const host = mount(() => (
      <PropertyEditor properties={[]} onCommit={onCommit} onError={onError} />
    ));

    (byLabel(host, "Add property") as HTMLButtonElement).click();
    await flush();

    type(byLabel(host, "New property key") as HTMLInputElement, "oura-date");
    const value = byLabel(host, "New property value") as HTMLInputElement;
    type(value, "2026-08-20");
    press(value, "Enter");
    await flush();

    expect(onCommit).toHaveBeenCalledWith("oura-date", "2026-08-20");
    // A missing catalogue costs completion, not the write — surfacing
    // it would be an error toast for something the user can't act on.
    expect(onError).not.toHaveBeenCalled();
  });
});

describe("value is a page ref, not free text", () => {
  it("completes `[[` in the value field into a wrapped page ref", async () => {
    // 87% of property values in a real graph are `[[page]]` or `#tag`
    // (issue #13). A plain text field means typing the page name from
    // memory with nothing checking it, which is the case that matters
    // most getting the worst treatment.
    knownPropertyKeys.mockResolvedValue([]);
    searchPages.mockResolvedValue([{ slug: "buser-cortex", title: "buser/cortex" }]);

    const writes: Array<[string, string]> = [];
    const host = mount(() => (
      <PropertyEditor
        properties={[]}
        onCommit={(k: string, v: string) => {
          writes.push([k, v]);
        }}
      />
    ));
    await flush();

    (byLabel(host, "Add property") as HTMLButtonElement).click();
    await flush();

    type(byLabel(host, "New property key") as HTMLInputElement, "related");
    const valueInput = byLabel(host, "New property value") as HTMLInputElement;

    valueInput.value = "[[cort";
    valueInput.setSelectionRange(6, 6);
    valueInput.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();

    expect(searchPages).toHaveBeenCalled();
    const option = host.querySelector('[role="option"][aria-selected="true"]');
    expect(option?.textContent).toContain("buser/cortex");

    option?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    await flush();

    // Wrapped, not pasted raw: the value has to round-trip through the
    // `.md` as a ref the parser reads back.
    expect(valueInput.value).toBe("[[buser/cortex]]");
  });

  it("stays quiet while the value is plain text", async () => {
    knownPropertyKeys.mockResolvedValue([]);
    searchPages.mockResolvedValue([{ slug: "x", title: "x" }]);

    const host = mount(() => <PropertyEditor properties={[]} onCommit={() => {}} />);
    await flush();
    (byLabel(host, "Add property") as HTMLButtonElement).click();
    await flush();

    const valueInput = byLabel(host, "New property value") as HTMLInputElement;
    valueInput.value = "high";
    valueInput.setSelectionRange(4, 4);
    valueInput.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();

    expect(host.querySelector('[role="option"]')).toBeNull();
  });
});
