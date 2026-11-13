/**
 * The property attribute line owns its own type scale, and neither
 * call site here may take it back.
 *
 * `src/styles.css` carries an unlayered
 * `button, input, textarea { font-size: inherit }`. An unlayered rule
 * outranks every layered one regardless of specificity, and Tailwind's
 * utilities live in `@layer utilities` — so a `text-*` on a `<button>`
 * resolves to nothing and the element paints at whatever its ancestor
 * is. That is the whole origin of this redesign: the property pills
 * carried `text-[11px]`, it was dead, and they painted at the block
 * body's 17px, centred, wrapping to three lines each.
 *
 * `<BlockProperties />` now puts the scale on its own container and
 * lets the inner button inherit, so the trap is handled once in
 * `@outl/shared`. What this file pins is that a client does not
 * reintroduce it — by passing a size down, or by hand-rolling the row
 * out of `<button>`s again, which is what the page strip used to be.
 *
 * Source reads rather than renders: the failure is a CSS cascade
 * happy-dom does not model, so mounting and reading `getComputedStyle`
 * would pass either way.
 */
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

const BODY = readFileSync(join(import.meta.dirname, "BlockBody.tsx"), "utf8");
const JOURNAL = readFileSync(join(import.meta.dirname, "Journal.tsx"), "utf8");
const STYLES = readFileSync(
  join(import.meta.dirname, "..", "styles.css"),
  "utf8",
);

/**
 * Guard the premise every pin below rests on. If the unlayered rule
 * ever goes away, the constraint goes with it and these tests should
 * be revisited rather than silently kept passing by a comment.
 */
function assertTheTrapIsStillThere() {
  expect(
    /button,\s*input,\s*textarea\s*\{[^}]*font-size:\s*inherit/m.test(STYLES),
    "styles.css no longer forces font-size on buttons — re-read this test",
  ).toBe(true);
}

/** The `<BlockProperties … />` call in `source`, attributes and all. */
function propertiesCall(source: string, where: string): string {
  const at = source.indexOf("<BlockProperties");
  expect(at, `${where} still renders the attribute line`).toBeGreaterThan(-1);
  return source.slice(at, source.indexOf("/>", at));
}

describe("the block's attribute line", () => {
  it("lets the component own the type scale", () => {
    assertTheTrapIsStillThere();
    // `rowClass` is for padding and margins. A size passed here would
    // land on the container and *work*, which is worse than failing:
    // it puts a second owner on the one fact the component exists to
    // hold, and the two drift at the next design pass.
    const call = propertiesCall(BODY, "BlockBody");
    expect(call).not.toMatch(/text-\[\d/);
  });

  it("opens the sheet instead of editing in place", () => {
    // Editing left the outline: a field is edited in the properties
    // sheet, which is a form. `onCommit` coming back here would mean
    // the pills came back with it.
    const call = propertiesCall(BODY, "BlockBody");
    expect(call).toContain("onOpen");
    expect(call).not.toContain("onCommit");
  });
});

describe("the page's attribute line", () => {
  it("uses the shared component, not hand-rolled buttons", () => {
    // It used to be a `<For>` over `<button>`s in a scrolling strip —
    // a second implementation of the same row, which is how the page's
    // metadata and a block's came to look like different kinds of
    // thing, and how the dead `text-[11px]` got written twice.
    const call = propertiesCall(JOURNAL, "Journal");
    expect(call).toContain("page_properties");
    expect(JOURNAL).not.toContain("editableProperties(");
  });

  it("lets the component own the type scale here too", () => {
    assertTheTrapIsStillThere();
    expect(propertiesCall(JOURNAL, "Journal")).not.toMatch(/text-\[\d/);
  });
});
