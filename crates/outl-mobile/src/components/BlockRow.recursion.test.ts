/**
 * `<BlockRow />` renders itself for every child, and a prop it forgets
 * to pass down works on a top-level block and silently does nothing
 * everywhere else.
 *
 * Two props were in exactly that state, and neither TypeScript nor any
 * test could see it — both are optional, so omitting them compiles:
 *
 * - **`onPasteMarkdown`**. A paste in a nested block fell back to the
 *   browser's literal splice, so pasting a spreadsheet range or a
 *   markdown table into anything but a root block produced a wall of
 *   tabs instead of a table ([#329](https://github.com/outlmd/outl/issues/329)
 *   is where this surfaced).
 * - **`onSetProperty`**. `Journal.tsx` passes it, `BlockBody` declares
 *   it, `BlockProperties` calls it — and `BlockRow`'s own `<BlockBody>`
 *   call site never forwarded it either, so tapping a `key:: value`
 *   chip on mobile was a no-op on *every* block.
 *
 * So the invariant is checked structurally rather than by rendering:
 * every prop the component accepts is either forwarded to the recursive
 * call or listed below with a reason. That is the same shape as the
 * repo's other "declare the gap, don't discover it" pins —
 * `outl_shortcuts::support`'s exhaustive match, `wire_types.rs`'s key
 * sets, `outl-tauri-shared`'s `DECLARED_GAPS`.
 *
 * Reading the source is deliberate. Rendering this component needs
 * haptics, a Tauri bridge and a WKWebView-shaped textarea; a test that
 * mocks all three would pin the mocks, and the defect is in the JSX, so
 * the JSX is what gets read.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { describe, expect, it } from "vitest";

const SOURCE = readFileSync(
  join(import.meta.dirname, "BlockRow.tsx"),
  "utf8",
);

/**
 * Props the recursive call is right not to forward, each with the
 * reason it is not a drift.
 */
const NOT_FORWARDED: Record<string, string> = {
  block: "the child is the subject of the recursive call (`block={child}`)",
  depth: "incremented for the child (`depth={props.depth + 1}`)",
};

/** Prop names declared in `interface BlockRowProps`. */
function declaredProps(): string[] {
  const start = SOURCE.indexOf("interface BlockRowProps {");
  expect(start, "interface BlockRowProps not found").toBeGreaterThan(-1);
  // The interface ends at the first line that closes it at column 0.
  const end = SOURCE.indexOf("\n}", start);
  const body = SOURCE.slice(start, end);
  const names = new Set<string>();
  for (const m of body.matchAll(/^\s{2}(\w+)\??:/gm)) names.add(m[1]);
  return [...names];
}

/** Prop names passed to the recursive `<BlockRow … />`. */
function forwardedProps(): string[] {
  // The self-call inside the `<For each={props.block.children}>`.
  const start = SOURCE.lastIndexOf("<BlockRow");
  expect(start, "recursive <BlockRow> not found").toBeGreaterThan(-1);
  const end = SOURCE.indexOf("/>", start);
  const call = SOURCE.slice(start, end);
  const names = new Set<string>();
  for (const m of call.matchAll(/^\s+(\w+)=/gm)) names.add(m[1]);
  return [...names];
}

describe("BlockRow recursion", () => {
  it("forwards every prop it accepts to its children", () => {
    const declared = declaredProps();
    const forwarded = new Set(forwardedProps());
    const missing = declared.filter(
      (name) => !forwarded.has(name) && !(name in NOT_FORWARDED),
    );
    expect(
      missing,
      `these props stop at the top-level block: ${missing.join(", ")}. ` +
        "Forward them in the recursive <BlockRow>, or add a row to " +
        "NOT_FORWARDED saying why a child must not have them.",
    ).toEqual([]);
  });

  it("reads a non-trivial prop list", () => {
    // A guard on the guard: if the regex stops matching, the test above
    // passes vacuously, which is the state the two dropped props were
    // already in.
    expect(declaredProps().length).toBeGreaterThan(15);
    expect(forwardedProps().length).toBeGreaterThan(15);
  });

  it("does not forward the two props that identify the child", () => {
    const forwarded = new Set(forwardedProps());
    // `block` and `depth` are passed, but computed — so they must not
    // be in the list as `props.block` / `props.depth`.
    expect(SOURCE).toContain("block={child}");
    expect(SOURCE).toContain("depth={props.depth + 1}");
    expect(forwarded.has("block")).toBe(true);
    expect(forwarded.has("depth")).toBe(true);
  });

  it("forwards the two props that were dropped", () => {
    // Named explicitly so the regression has its own failure, not just
    // a set difference.
    const forwarded = new Set(forwardedProps());
    expect(forwarded.has("onPasteMarkdown")).toBe(true);
    expect(forwarded.has("onSetProperty")).toBe(true);
  });

  it("hands onSetProperty to the body that calls it", () => {
    // The other half: `BlockBody` declares and uses it, so a forward to
    // the children is not enough if the row itself withholds it.
    expect(SOURCE).toContain("onSetProperty={props.onSetProperty}");
  });
});
