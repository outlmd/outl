import { describe, expect, it } from "vitest";

import type { BlockNode } from "../api/types";
import {
  type CaretExit,
  caretExit,
  caretNavKey,
  entryCaretOffset,
  resolveCaretExit,
} from "./index";

describe("caretNavKey", () => {
  const bare = (key: string) => ({
    key,
    shiftKey: false,
    metaKey: false,
    ctrlKey: false,
    altKey: false,
  });

  it("names the four bare arrows", () => {
    for (const key of ["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"]) {
      expect(caretNavKey(bare(key))).toBe(key);
    }
  });

  it("is not any other key", () => {
    for (const key of ["Enter", "Tab", "a", "Home", "End", "Backspace"]) {
      expect(caretNavKey(bare(key))).toBeNull();
    }
  });

  it("refuses a modified arrow, because those chords mean something else", () => {
    // `Alt+↑` moves the block, `Cmd+↑` is the OS's document-edge
    // motion, `Shift+↑` extends a selection. Both clients spelled this
    // guard out before, so one gaining a modifier and the other not was
    // two clients disagreeing about the same keystroke.
    for (const mod of ["shiftKey", "metaKey", "ctrlKey", "altKey"] as const) {
      expect(caretNavKey({ ...bare("ArrowUp"), [mod]: true })).toBeNull();
    }
  });
});

describe("caretExit — ← / →", () => {
  it("steps to the previous block's end from offset 0", () => {
    expect(caretExit("ArrowLeft", "ship it", 0, 0)).toEqual({
      to: "previous",
      landing: "end",
    });
  });

  it("stays in the block anywhere past offset 0", () => {
    expect(caretExit("ArrowLeft", "ship it", 1, 1)).toBeNull();
  });

  it("steps to the next block's start from the end", () => {
    expect(caretExit("ArrowRight", "ship it", 7, 7)).toEqual({
      to: "next",
      landing: "start",
    });
  });

  it("stays in the block one char short of the end", () => {
    expect(caretExit("ArrowRight", "ship it", 6, 6)).toBeNull();
  });

  it("counts the end in codepoints, so an emoji block still exits", () => {
    // "a🎉" is 2 codepoints and 3 UTF-16 units. A caller that converted
    // its `selectionStart` gets the exit; one that forgot would read 3
    // as past the end — which the clamp also absorbs.
    expect(caretExit("ArrowRight", "a🎉", 2, 2)).toEqual({
      to: "next",
      landing: "start",
    });
  });

  it("leaves a selection alone — an arrow collapses it in place", () => {
    expect(caretExit("ArrowLeft", "ship it", 0, 3)).toBeNull();
    expect(caretExit("ArrowRight", "ship it", 4, 7)).toBeNull();
    expect(caretExit("ArrowUp", "ship it", 0, 7)).toBeNull();
  });

  it("an empty block exits in both directions", () => {
    expect(caretExit("ArrowLeft", "", 0, 0)?.to).toBe("previous");
    expect(caretExit("ArrowRight", "", 0, 0)?.to).toBe("next");
  });
});

describe("caretExit — ↑ / ↓", () => {
  it("carries the column out of a single-line block", () => {
    expect(caretExit("ArrowUp", "ship it", 4, 4)).toEqual({
      to: "previous",
      landing: { column: 4 },
    });
    expect(caretExit("ArrowDown", "ship it", 4, 4)).toEqual({
      to: "next",
      landing: { column: 4 },
    });
  });

  it("a multi-line block absorbs the move off its inner lines", () => {
    const fence = "```rust\nfn main() {}\n```";
    const onSecondLine = 8 + 3; // inside `fn main`
    expect(caretExit("ArrowUp", fence, onSecondLine, onSecondLine)).toBeNull();
    expect(caretExit("ArrowDown", fence, onSecondLine, onSecondLine)).toBeNull();
  });

  it("↑ exits only from the first line, ↓ only from the last", () => {
    const fence = "```rust\nfn main() {}\n```";
    expect(caretExit("ArrowUp", fence, 2, 2)).toEqual({
      to: "previous",
      landing: { column: 2 },
    });
    expect(caretExit("ArrowDown", fence, 2, 2)).toBeNull();

    const lastLineStart = "```rust\nfn main() {}\n".length;
    expect(caretExit("ArrowDown", fence, lastLineStart + 1, lastLineStart + 1))
      .toEqual({ to: "next", landing: { column: 1 } });
    expect(caretExit("ArrowUp", fence, lastLineStart + 1, lastLineStart + 1))
      .toBeNull();
  });

  it("a block ending in a newline still has that empty last line", () => {
    // Caret on the trailing empty line: ↓ leaves, ↑ does not.
    expect(caretExit("ArrowDown", "one\n", 4, 4)).toEqual({
      to: "next",
      landing: { column: 0 },
    });
    expect(caretExit("ArrowUp", "one\n", 4, 4)).toBeNull();
  });
});

describe("entryCaretOffset", () => {
  it("resolves the far edges", () => {
    expect(entryCaretOffset("hello", { to: "previous", landing: "end" })).toBe(5);
    expect(entryCaretOffset("hello", { to: "next", landing: "start" })).toBe(0);
  });

  it("counts the end in codepoints, not UTF-16 units", () => {
    expect(entryCaretOffset("a🎉", { to: "previous", landing: "end" })).toBe(2);
  });

  it("aims a column at the line facing the block being left", () => {
    const fence = "```rust\nfn main() {}\n```";
    // Stepping UP into this block arrives at its last line (` ``` `).
    expect(
      entryCaretOffset(fence, { to: "previous", landing: { column: 2 } }),
    ).toBe("```rust\nfn main() {}\n".length + 2);
    // Stepping DOWN into it arrives at the first.
    expect(entryCaretOffset(fence, { to: "next", landing: { column: 2 } })).toBe(2);
  });

  it("clamps a column past the target line's end instead of overshooting", () => {
    // The neighbour's last line is 3 chars; column 9 must not land on
    // the line after it (there is none) or past the text.
    expect(entryCaretOffset("longer line\nabc", {
      to: "previous",
      landing: { column: 9 },
    })).toBe("longer line\n".length + 3);
    expect(entryCaretOffset("abc\nlonger line", {
      to: "next",
      landing: { column: 9 },
    })).toBe(3);
  });

  it("an empty neighbour takes the caret at 0 from every landing", () => {
    expect(entryCaretOffset("", { to: "previous", landing: "end" })).toBe(0);
    expect(entryCaretOffset("", { to: "next", landing: { column: 4 } })).toBe(0);
  });
});

describe("resolveCaretExit", () => {
  const block = (
    id: string,
    text: string,
    over: Partial<BlockNode> = {},
  ): BlockNode =>
    ({
      id,
      text,
      todo: null,
      tokens: [],
      collapsed: false,
      properties: [],
      children: [],
      ...over,
    }) as BlockNode;

  const up = (landing: CaretExit["landing"] = "end"): CaretExit => ({
    to: "previous",
    landing,
  });
  const down = (landing: CaretExit["landing"] = "start"): CaretExit => ({
    to: "next",
    landing,
  });

  const page = () => [block("a", "first"), block("b", "second")];

  it("steps to the previous block, caret at its end", () => {
    expect(resolveCaretExit(page(), "b", up())).toEqual({
      id: "a",
      caret: 5,
      text: "first",
    });
  });

  it("steps to the next block, caret at its start", () => {
    expect(resolveCaretExit(page(), "a", down())).toEqual({
      id: "b",
      caret: 0,
      text: "second",
    });
  });

  it("stays put at the top and at the bottom of the outline", () => {
    expect(resolveCaretExit(page(), "a", up())).toBeNull();
    expect(resolveCaretExit(page(), "b", down())).toBeNull();
  });

  it("measures the caret against the wire text, TODO prefix included", () => {
    // The editor shows `TODO buy milk`, so "end" is 13, not the DTO's
    // split `text` length of 8.
    const blocks = [block("a", "buy milk", { todo: "TODO" }), block("b", "next")];
    expect(resolveCaretExit(blocks, "b", up())).toEqual({
      id: "a",
      caret: 13,
      text: "TODO buy milk",
    });
    expect(resolveCaretExit(blocks, "b", up({ column: 0 }))).toEqual({
      id: "a",
      caret: 0,
      text: "TODO buy milk",
    });
  });

  it("walks into a child, and skips a collapsed subtree", () => {
    const open = [
      block("a", "parent", { children: [block("a1", "child")] }),
      block("b", "after"),
    ];
    expect(resolveCaretExit(open, "a", down())).toEqual({
      id: "a1",
      caret: 0,
      text: "child",
    });

    const folded = [
      block("a", "parent", {
        collapsed: true,
        children: [block("a1", "child")],
      }),
      block("b", "after"),
    ];
    expect(resolveCaretExit(folded, "a", down())).toEqual({
      id: "b",
      caret: 0,
      text: "after",
    });
  });

  it("goes nowhere from a block that left the walk, never to the first one", () => {
    // `previousVisibleId` reads a missing id as `ids[0]`, which would
    // teleport the caret to the top of the page from an `←` aimed at the
    // character to the user's left.
    expect(resolveCaretExit(page(), "deleted-by-a-peer", up())).toBeNull();
    expect(resolveCaretExit(page(), "deleted-by-a-peer", down())).toBeNull();
  });
});
