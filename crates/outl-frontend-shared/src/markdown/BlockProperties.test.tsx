/**
 * `<BlockProperties />` is an **attribute line**, not a row of pills.
 *
 * It shipped as chips because `remind::` was invisible in the outline.
 * That fixed the visibility and introduced a category error: a pill
 * means "flat label you tap to filter", which is a tag, and a
 * `key:: value` is a field. So the outline grew a row of things that
 * looked like buttons, were not buttons, and sat on every block of a
 * Roam-imported graph.
 *
 * The shape now follows what Things, Linear, Apple Reminders and
 * GitHub mobile do with item metadata: one quiet subtitle line, key
 * dimmer than value, colour reserved for the field that will actually
 * interrupt you. Editing left the outline entirely — the line is one
 * tap target that opens the properties sheet, which is a real form.
 */

import { render } from "solid-js/web";
import { describe, expect, it, vi } from "vitest";

import { BlockProperties } from "./BlockProperties";

function mount(node: () => unknown) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(node as () => any, host);
  return {
    host,
    dispose: () => {
      dispose();
      host.remove();
    },
  };
}

const FIELDS = [
  ["priority", "high"],
  ["status", "waiting"],
] as ReadonlyArray<readonly [string, string]>;

describe("BlockProperties", () => {
  it("renders nothing when the block carries no visible field", () => {
    // An import stamp is not a visible field, so a migrated block with
    // only `created::` / `roam-uid::` must draw no line at all — not an
    // empty one taking vertical space.
    const m = mount(() => (
      <BlockProperties properties={[["roam-uid", "radartech-0611"]]} />
    ));
    expect(m.host.textContent).toBe("");
    m.dispose();
  });

  it("writes the key and the value as separate elements", () => {
    // Baked into one string they cannot be styled apart, and the whole
    // point of the line is that the key recedes behind the value.
    const m = mount(() => <BlockProperties properties={FIELDS} />);
    const text = m.host.textContent ?? "";
    expect(text).toContain("priority");
    expect(text).toContain("high");
    // Not the chip's `key: value` — no colon glued to the key.
    expect(text).not.toContain("priority:");
    m.dispose();
  });

  it("dims the key below the value", () => {
    const m = mount(() => <BlockProperties properties={FIELDS} />);
    const html = m.host.innerHTML;
    expect(html).toContain("--color-outl-fg-dimmer");
    expect(html).toContain("--color-outl-fg-dim)");
    m.dispose();
  });

  it("gives the live field accent colour and drops its key", () => {
    // `remind::` is the one field that acts on the user, so it is the
    // one that earns colour. Its key is noise next to the schedule —
    // the icon already says what it is.
    const m = mount(() => (
      <BlockProperties properties={[["remind", "9am every 1h"]]} />
    ));
    expect(m.host.innerHTML).toContain("--color-outl-accent");
    expect(m.host.textContent).toContain("9am every 1h");
    expect(m.host.textContent).not.toContain("remind");
    m.dispose();
  });

  it("draws the live field's icon as an SVG, never an emoji", () => {
    // `DESIGN.md`: no emoji in chrome. It brings its own colours,
    // renders differently per OS and reads as content. The TUI keeps
    // its glyph — a terminal has no SVG — which is why the emoji map
    // in `properties.ts` stays put for `property_glyph` to mirror.
    const m = mount(() => <BlockProperties properties={[["remind", "9am"]]} />);
    expect(m.host.querySelector("svg")).not.toBeNull();
    expect(m.host.textContent).not.toContain("⏰");
    m.dispose();
  });

  it("stays on one line", () => {
    const m = mount(() => <BlockProperties properties={FIELDS} />);
    const row = m.host.firstElementChild as HTMLElement;
    expect(row.className).toContain("whitespace-nowrap");
    expect(row.className).toContain("overflow-hidden");
    m.dispose();
  });

  it("collapses the overflow into a count rather than wrapping", () => {
    const m = mount(() => (
      <BlockProperties
        properties={[
          ["a", "1"],
          ["b", "2"],
          ["c", "3"],
          ["d", "4"],
        ]}
      />
    ));
    expect(m.host.textContent).toContain("+2 more");
    m.dispose();
  });

  it("opens the sheet when the line is tapped", () => {
    const onOpen = vi.fn();
    const m = mount(() => (
      <BlockProperties properties={FIELDS} onOpen={onOpen} />
    ));
    (m.host.querySelector("button") as HTMLButtonElement).click();
    expect(onOpen).toHaveBeenCalledOnce();
    m.dispose();
  });

  it("does not let the tap reach the row underneath", () => {
    // The row selects / edits the block. A tap on the attribute line
    // means "show me these fields", never both.
    const onRow = vi.fn();
    const m = mount(() => (
      <div onClick={onRow}>
        <BlockProperties properties={FIELDS} onOpen={() => {}} />
      </div>
    ));
    (m.host.querySelector("button") as HTMLButtonElement).click();
    expect(onRow).not.toHaveBeenCalled();
    m.dispose();
  });

  it("is inert, not a dead button, when no handler is wired", () => {
    // A button that does nothing promises a tap it cannot deliver —
    // the exact defect the pills had.
    const m = mount(() => <BlockProperties properties={FIELDS} />);
    expect(m.host.querySelector("button")).toBeNull();
    m.dispose();
  });

  it("names the action for a screen reader", () => {
    const m = mount(() => (
      <BlockProperties properties={FIELDS} onOpen={() => {}} />
    ));
    const button = m.host.querySelector("button") as HTMLButtonElement;
    expect(button.getAttribute("aria-label")).toMatch(/propert/i);
    m.dispose();
  });

  it("sizes the line on the container, never on the button", () => {
    // Mobile's `styles.css` carries an unlayered
    // `button, input, textarea { font-size: inherit }`. An unlayered
    // rule outranks every layered one, and Tailwind's utilities live
    // in `@layer utilities` — so a `text-*` on the `<button>` resolves
    // to nothing and the line paints at the block body's 17px. That is
    // the bug this whole redesign started from. The container holds
    // the scale; the button inherits it.
    const m = mount(() => (
      <BlockProperties properties={FIELDS} onOpen={() => {}} />
    ));
    const row = m.host.firstElementChild as HTMLElement;
    const button = m.host.querySelector("button") as HTMLButtonElement;
    expect(row.className).toMatch(/text-\[/);
    expect(button.className).not.toMatch(/text-\[\d/);
    m.dispose();
  });

  it("lets the host add its own classes to the line", () => {
    const m = mount(() => (
      <BlockProperties properties={FIELDS} rowClass="px-4" />
    ));
    expect((m.host.firstElementChild as HTMLElement).className).toContain(
      "px-4",
    );
    m.dispose();
  });
});
