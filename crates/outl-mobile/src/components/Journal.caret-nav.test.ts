import { describe, expect, it, vi } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";
import type { CaretExit } from "@outl/shared/caret-nav";
import { createCaretNav, type CaretNavDeps } from "./Journal.caret-nav";

/**
 * The ordering, not the walk.
 *
 * `resolveCaretExit`'s behaviour is `@outl/shared/caret-nav`'s and is
 * tested there. What only this client can get wrong is the sequence:
 * the answer has to be synchronous (the textarea decides whether to
 * swallow the key from it), and `commitEdit` has to land *before* the
 * neighbour is opened, because it ends with `setEditingId(null)`.
 */

function block(id: string, text: string, todo: string | null = null): BlockNode {
  return {
    id,
    text,
    todo,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  } as BlockNode;
}

const up: CaretExit = { to: "previous", landing: "end" };

/** How the harness's `commitEdit` ends. `"fails"` is what the real one
 *  does on a backend error or a timeout: it surfaces the message itself
 *  and resolves `false`, leaving the editor open on the draft. */
type CommitOutcome = "saves" | "fails" | "throws";

function harness(blocks: BlockNode[], outcome: CommitOutcome = "saves") {
  const calls: string[] = [];
  let release!: () => void;
  const committed = new Promise<void>((resolve) => (release = resolve));
  const deps: CaretNavDeps = {
    navBlocks: () => blocks,
    commitEdit: async () => {
      calls.push("commitEdit");
      await committed;
      if (outcome === "throws") throw new Error("backend said no");
      return outcome === "saves";
    },
    startEdit: (id, initial) => calls.push(`startEdit:${id}:${initial}`),
  };
  return { calls, nav: createCaretNav(deps), release };
}

const page = () => [block("a", "first"), block("b", "second")];

/** A textarea holding `value`, caret parked at the end the way
 *  `EditableTextarea` leaves it on mount. */
function textarea(value: string): HTMLTextAreaElement {
  const el = document.createElement("textarea");
  el.value = value;
  el.setSelectionRange(value.length, value.length);
  return el;
}

describe("createCaretNav", () => {
  it("answers synchronously, before the commit has resolved", () => {
    const { calls, nav } = harness(page());
    // The textarea calls `preventDefault` off this return value, so it
    // must not be a promise and must not wait on the round-trip.
    expect(nav.onCaretExit("b", up)).toBe(true);
    expect(calls).toEqual(["commitEdit"]);
  });

  it("opens the neighbour only once the commit lands", async () => {
    const { calls, nav, release } = harness(page());
    nav.onCaretExit("b", up);
    expect(calls).toEqual(["commitEdit"]);

    release();
    await vi.waitFor(() => expect(calls).toHaveLength(2));
    // Order is the point: `commitEdit` ends with `setEditingId(null)`,
    // so a `startEdit` before it would drop the user out of Insert.
    expect(calls).toEqual(["commitEdit", "startEdit:a:first"]);
  });

  it("seeds the draft with the wire text the caret was measured against", async () => {
    const { calls, nav, release } = harness([
      block("a", "buy milk", "TODO"),
      block("b", "second"),
    ]);
    nav.onCaretExit("b", up);
    release();
    await vi.waitFor(() => expect(calls).toHaveLength(2));
    expect(calls).toEqual(["commitEdit", "startEdit:a:TODO buy milk"]);
    // `TODO buy milk` is 13 codepoints, so "end" is 13 — seeding the
    // editor from the DTO's split `text` would have put the caret 5
    // short of where the user's arrow pointed.
    const ta = textarea("TODO buy milk");
    nav.applyPendingCaret(ta);
    expect(ta.selectionStart).toBe(13);
  });

  it("commits nothing at the outline's edge, and says the step was not taken", () => {
    const { calls, nav } = harness(page());
    expect(nav.onCaretExit("a", up)).toBe(false);
    // Nothing committed: a `←` in the first block must leave the user
    // in Insert, not drop them out of it.
    expect(calls).toEqual([]);
  });

  it("opens the caret where the step asked, then forgets it", async () => {
    const { calls, nav, release } = harness(page());
    nav.onCaretExit("b", up);
    release();
    // The offset is recorded behind the commit, so wait for it to land.
    await vi.waitFor(() => expect(calls).toHaveLength(2));

    const first = textarea("first");
    nav.applyPendingCaret(first);
    expect(first.selectionStart).toBe(5);

    // Forgotten after one use: every other entry into Insert wants the
    // textarea's own "end of the text", because a tap into a block
    // means "type here".
    const other = textarea("something else");
    other.setSelectionRange(3, 3);
    nav.applyPendingCaret(other);
    expect(other.selectionStart).toBe(3);
  });

  it("leaves the caret alone when no step asked for one", () => {
    const { nav } = harness(page());
    const el = textarea("untouched");
    el.setSelectionRange(2, 2);
    nav.applyPendingCaret(el);
    expect(el.selectionStart).toBe(2);
  });

  it.each<CommitOutcome>(["fails", "throws"])(
    "leaves the user on their draft when the commit %s",
    async (outcome) => {
      const { calls, nav, release } = harness(page(), outcome);
      nav.onCaretExit("b", up);
      release();
      // `commitEdit` surfaces its own error and keeps the editor open
      // on the draft; a `startEdit` now would overwrite that draft with
      // the neighbour's text. The real one *resolves* on failure, so a
      // net that only covered a rejection let exactly that through.
      await expect(
        vi.waitFor(() => expect(calls).toHaveLength(2), { timeout: 150 }),
      ).rejects.toThrow();
      expect(calls).toEqual(["commitEdit"]);
    },
  );

  it("swallows a key repeat while a step is in flight", async () => {
    const { calls, nav, release } = harness(page());
    expect(nav.onCaretExit("b", up)).toBe(true);
    // A held arrow repeats faster than the save round-trip: the repeat
    // must still be swallowed, and must not commit the same draft twice.
    expect(nav.onCaretExit("b", up)).toBe(true);
    expect(calls).toEqual(["commitEdit"]);

    release();
    await vi.waitFor(() => expect(calls).toHaveLength(2));
    expect(calls).toEqual(["commitEdit", "startEdit:a:first"]);
  });
});
