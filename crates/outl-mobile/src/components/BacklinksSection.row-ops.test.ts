/**
 * What a gesture on a backlink row actually does.
 *
 * Split from `BacklinksSection.test.tsx` by subject, not by line count:
 * that file renders the card and pins the chrome (the tap, the hold,
 * the sheet, the busy marker), and this one drives the ops directly.
 * The two questions are genuinely different — "does the gesture reach
 * it" versus "does it address the right block on the right page, once"
 * — and the second one does not need a DOM to answer.
 *
 * Every write here is a cross-page mutation followed by an
 * `O(blocks-in-workspace)` re-ask, so the window in which one is in
 * flight is long enough for a second gesture to land in it. None of
 * these ops is idempotent from the user's seat: a second toggle flips
 * the task back, a second indent moves the block again. A timeout is
 * not the guard and would be worse than none — fire one while the write
 * still lands and the user retries, so the task flips twice.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, PageView } from "@outl/shared/api/types";

import {
  type BacklinkCommit,
  createBacklinkRowOps,
} from "./BacklinksSection.row-ops";

/**
 * The cross-page commands, **and** the open-page twins none of these
 * gestures may reach. The second group is mocked on purpose: a `not
 * called` assertion against a name the module does not export passes
 * whatever the code does.
 */
const commands = vi.hoisted(() => ({
  backlinkRowOp: vi.fn(),
  createBacklinkSibling: vi.fn(),
  editBacklinkBlock: vi.fn(),
  editBlock: vi.fn(),
  toggleTodo: vi.fn(),
  deleteBlock: vi.fn(),
  indentBlock: vi.fn(),
  outdentBlock: vi.fn(),
  moveBlockUp: vi.fn(),
  moveBlockDown: vi.fn(),
  createBlock: vi.fn(),
}));
vi.mock("@outl/shared/api/commands", () => commands);
vi.mock("../lib/haptics", () => ({ haptic: vi.fn() }));

function backlink(over: Partial<Backlink> = {}): Backlink {
  return {
    block_id: "b1",
    todo: null,
    ancestors: [],
    source_page: { id: "p1", slug: "journal", title: "Journal", kind: "journal" },
    source_block: {
      id: "b1",
      text: "cites [[project]]",
      todo: null,
      tokens: [],
      collapsed: false,
      properties: [],
      children: [],
    },
    source_block_path: [],
    ...over,
  } as unknown as Backlink;
}

/** A second citing block, on the same card as `backlink()`. */
function otherRow(): Backlink {
  const base = backlink();
  return {
    ...base,
    block_id: "b2",
    source_block: { ...base.source_block, id: "b2" },
  };
}

/** A citing block with `count` descendants, none of them rendered. */
function withChildren(count: number): Backlink {
  const base = backlink();
  return {
    ...base,
    source_block: {
      ...base.source_block,
      children: Array.from({ length: count }, (_, i) => ({
        ...base.source_block,
        id: `child-${i}`,
        children: [],
      })),
    },
  };
}

/** A reply of the source page — only its identity matters here. */
const sourceView = () =>
  ({ page: { id: "p1", slug: "journal" } }) as unknown as PageView;

/** `vi.fn` needs the signature: `runOnSource` awaits what this
 *  returns, so a spy that cannot be typed as returning a promise
 *  cannot stand in for the host's late-resolving re-ask. */
type CommitSpy = ReturnType<
  typeof vi.fn<(commit: BacklinkCommit) => void | Promise<void>>
>;
const commitSpy = (): CommitSpy =>
  vi.fn<(commit: BacklinkCommit) => void | Promise<void>>();

function ops(onEditCommitted: CommitSpy = commitSpy()) {
  const onError = vi.fn();
  return {
    // Wrapped, not passed through: a bare `vi.fn()` has no call
    // signature to match the dep's, and the arrow still records the
    // call *and* forwards the promise `runOnSource` awaits.
    api: createBacklinkRowOps({
      onEditCommitted: (commit) => onEditCommitted(commit),
      onError: (message) => onError(message),
    }),
    onEditCommitted,
    onError,
  };
}

/** An `onEditCommitted` that does not resolve until told to. */
function heldRefresh() {
  let land = () => {};
  const fn = commitSpy().mockImplementation(
    () => new Promise<void>((resolve) => (land = resolve)),
  );
  return { fn, land: () => land() };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

beforeEach(() => {
  vi.clearAllMocks();
});

/**
 * Every gesture, the op tag it sends, and the open-page command it must
 * never reach. The second column is the whole point: those commands act
 * on the page the user is looking at, so each one is *tolerant* of a
 * block id the tree no longer holds, in a different way per command.
 */
const OPS: Array<{
  name: string;
  run: (api: ReturnType<typeof ops>["api"], l: Backlink) => Promise<void>;
  op: string;
  /** The `block.rs` twin, and what its tolerance does here. */
  twin: ReturnType<typeof vi.fn>;
}> = [
  // `indent` / `move_*` use `ensure_in_tree`, not `ensure_live`, because
  // `trash::restore` is itself a move — so a trashed node is a legal
  // source and these three *land*, re-parenting a block the user can no
  // longer reach inside the trash.
  { name: "indent", run: (api, l) => api.indent(l), op: "indent", twin: commands.indentBlock },
  { name: "outdent", run: (api, l) => api.outdent(l), op: "outdent", twin: commands.outdentBlock },
  { name: "moveUp", run: (api, l) => api.moveUp(l), op: "move-up", twin: commands.moveBlockUp },
  { name: "moveDown", run: (api, l) => api.moveDown(l), op: "move-down", twin: commands.moveBlockDown },
  // These two do raise, and their raise is `ActionError`'s `Display` — a
  // ULID and an implementation noun.
  { name: "toggleTodo", run: (api, l) => api.toggleRowTodo(l), op: "toggle-todo", twin: commands.toggleTodo },
  { name: "delete", run: (api, l) => api.requestDelete(l), op: "delete", twin: commands.deleteBlock },
];

describe("every op is addressed to the citing block's own page", () => {
  for (const { name, run, op, twin } of OPS) {
    it(`${name} sends the source page id, never the open page's`, async () => {
      commands.backlinkRowOp.mockResolvedValue(sourceView());
      const o = ops();

      await run(o.api, backlink());

      // The only page id in scope on this card is the one on screen,
      // which is the wrong one. `backlinkEditTarget` resolves the pair.
      expect(commands.backlinkRowOp).toHaveBeenCalledWith("p1", "b1", op);
      // **The assertion that matters is this one.** Routed through the
      // twin, a gesture on a block a peer trashed is not a refusal — it
      // is a mutation that lands somewhere the user cannot see or undo,
      // and no message-only test can tell the two apart.
      expect(twin).not.toHaveBeenCalled();
      expect(o.onEditCommitted).toHaveBeenCalledWith({
        sourcePageId: "p1",
        view: expect.anything(),
      });
    });

    it(`${name} writes nothing on an orphan backlink`, async () => {
      const o = ops();

      await run(o.api, backlink({ source_page: null }));

      expect(commands.backlinkRowOp).not.toHaveBeenCalled();
      expect(twin).not.toHaveBeenCalled();
      expect(o.onEditCommitted).not.toHaveBeenCalled();
    });
  }

  it("creates a sibling below on the source page, and reports it", async () => {
    commands.createBacklinkSibling.mockResolvedValue({
      view: sourceView(),
      new_id: "fresh",
    });
    const o = ops();
    const l = backlink();

    await o.api.createBelow(l);

    // `createBlock` recovers from a stale anchor by appending at the end
    // of the page it is given, so `o` on a stale row left an empty junk
    // block on a page nothing on screen showed — and then reported
    // "Added an empty block after this reference".
    expect(commands.createBacklinkSibling).toHaveBeenCalledWith(
      "p1",
      "b1",
      "below",
    );
    expect(commands.createBlock).not.toHaveBeenCalled();
    // `created` is what lets the host say where the block went — and
    // only the host can, because the sentence is a lie when the source
    // page *is* the page on screen.
    expect(o.onEditCommitted).toHaveBeenCalledWith({
      sourcePageId: "p1",
      view: expect.anything(),
      created: { page: l.source_page, side: "below" },
    });
  });

  it("creates a sibling above on the other side of the same block", async () => {
    commands.createBacklinkSibling.mockResolvedValue({
      view: sourceView(),
      new_id: "fresh",
    });
    const o = ops();

    await o.api.createAbove(backlink());

    expect(commands.createBacklinkSibling).toHaveBeenCalledWith(
      "p1",
      "b1",
      "above",
    );
    expect(commands.createBlock).not.toHaveBeenCalled();
  });

  it("creates nothing on an orphan backlink", async () => {
    const o = ops();

    await o.api.createBelow(backlink({ source_page: null }));
    await o.api.createAbove(backlink({ source_page: null }));

    expect(commands.createBacklinkSibling).not.toHaveBeenCalled();
    expect(commands.createBlock).not.toHaveBeenCalled();
  });

  it("edits text with the strict command, never editBlock", async () => {
    commands.editBacklinkBlock.mockResolvedValue(sourceView());
    const o = ops();

    await o.api.commitRow(backlink(), "edited text");

    // `editBlock` recovers from a stale id by appending at the end of
    // the page, which moves the user's text to a page they are not
    // looking at.
    expect(commands.editBacklinkBlock).toHaveBeenCalledWith(
      "p1",
      "b1",
      "edited text",
    );
    expect(commands.editBlock).not.toHaveBeenCalled();
  });

  it("surfaces a refusal and reports no commit", async () => {
    commands.backlinkRowOp.mockRejectedValue("block is not in the tree");
    const o = ops();

    await o.api.indent(backlink());

    expect(o.onError).toHaveBeenCalledWith(
      expect.stringContaining("not in the tree"),
    );
    expect(o.onEditCommitted).not.toHaveBeenCalled();
  });
});

/**
 * The catalog's sentence, as `stale_backlink_block_unchanged_refusal`
 * words it for a block that is no longer live.
 */
const CATALOG_REFUSAL = "That block was deleted, so nothing changed.";

/** What `ActionError`'s `Display` prints: a ULID and an implementation
 *  noun, which is what the open-page commands rejected with. */
const RAW_ACTION_ERROR = "block 01J7Z9KQ8V3M4N5P6R7S8T9VWX is not in the tree";

describe("a refused gesture says so, in the catalog's words", () => {
  it("shows the catalog's sentence rather than a ULID", async () => {
    // Both are armed: the twin rejects the way the backend actually
    // rejects, so this fails on the *wording the user reads* rather
    // than on an unmocked command.
    commands.backlinkRowOp.mockRejectedValue(CATALOG_REFUSAL);
    commands.toggleTodo.mockRejectedValue(RAW_ACTION_ERROR);
    const o = ops();

    await o.api.toggleRowTodo(backlink());

    expect(o.onError).toHaveBeenCalledWith(CATALOG_REFUSAL);
    // Root `CLAUDE.md` invariant 12: the nudge wording belongs in the
    // catalog, never in a client — and a raw id is not wording at all.
    expect(o.onError).not.toHaveBeenCalledWith(
      expect.stringMatching(/\b0[0-9A-HJKMNP-TV-Z]{25}\b/),
    );
  });

  it("reports no success when the write was refused", async () => {
    // `deleteBlock` *resolves* on a block that has already left the
    // tree — the open page's tolerance — so routed through it a refused
    // delete reported a landed mutation and the row simply came back.
    commands.backlinkRowOp.mockRejectedValue(CATALOG_REFUSAL);
    commands.deleteBlock.mockResolvedValue(sourceView());
    const o = ops();

    await o.api.requestDelete(backlink());

    expect(o.onEditCommitted).not.toHaveBeenCalled();
    expect(o.onError).toHaveBeenCalledWith(CATALOG_REFUSAL);
  });
});

describe("the card takes one write at a time", () => {
  it("refuses a second op while the first is in flight", async () => {
    const refresh = heldRefresh();
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const o = ops(refresh.fn);

    void o.api.indent(backlink());
    await tick();
    expect(o.api.pending()).toBe("b1");

    // A different op on a *different* row, the same card: the re-ask
    // replaces every row, so two writes in flight race on the rows
    // they are both about to be judged by. The same row twice would
    // pass under a per-row guard too, which is the decision this pins.
    await o.api.toggleRowTodo(otherRow());
    expect(commands.backlinkRowOp).toHaveBeenCalledTimes(1);
    expect(commands.backlinkRowOp).not.toHaveBeenCalledWith(
      "p1",
      "b2",
      "toggle-todo",
    );
    expect(o.api.pending()).toBe("b1");

    refresh.land();
    await tick();
    expect(o.api.pending()).toBeNull();
  });

  it("clears the pending row even when the write is refused", async () => {
    commands.backlinkRowOp.mockRejectedValue(new Error("nope"));
    const o = ops();

    await o.api.indent(backlink());

    // A guard that outlives its write is a card that never accepts
    // another gesture.
    expect(o.api.pending()).toBeNull();
  });

  it("holds the editor open until the fresh rows land", async () => {
    const refresh = heldRefresh();
    commands.editBacklinkBlock.mockResolvedValue(sourceView());
    const o = ops(refresh.fn);

    o.api.openEditor(backlink());
    expect(o.api.editing()).toBe("b1");

    void o.api.commitRow(backlink(), "edited");
    await tick();

    // Close it before the scan answers and the row renders the rows it
    // still holds — the pre-edit text — so the user watches their edit
    // revert and then reappear.
    expect(o.api.editing()).toBe("b1");

    refresh.land();
    await tick();
    expect(o.api.editing()).toBeNull();
  });

  it("keeps the editor on its draft when the write is refused", async () => {
    commands.editBacklinkBlock.mockRejectedValue("gone");
    const o = ops();

    o.api.openEditor(backlink());
    await o.api.commitRow(backlink(), "text the user typed");

    // Nothing was written, so closing it would drop text the user can
    // no longer see.
    expect(o.api.editing()).toBe("b1");
  });

  it("does not open an editor while a write is in flight", async () => {
    const refresh = heldRefresh();
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const o = ops(refresh.fn);

    void o.api.indent(backlink());
    await tick();
    o.api.openEditor(backlink());

    expect(o.api.editing()).toBeNull();
    refresh.land();
    await tick();
  });
});

describe("the direction toggle's re-ask holds every door, not only the row write", () => {
  /** Start a reorder whose re-ask has not landed, and return the way out. */
  async function reorderInFlight(o: ReturnType<typeof ops>) {
    let land = () => {};
    void o.api.runReorder(
      () => new Promise<void>((resolve) => (land = resolve)),
    );
    await tick();
    return async () => {
      land();
      await tick();
    };
  }

  it("opens no editor", async () => {
    const o = ops();
    const finish = await reorderInFlight(o);

    o.api.openEditor(backlink());
    expect(o.api.editing()).toBeNull();

    await finish();
    o.api.openEditor(backlink());
    expect(o.api.editing()).toBe("b1");
  });

  it("opens no long-press sheet", async () => {
    const o = ops();
    const finish = await reorderInFlight(o);

    o.api.openMenu(backlink());
    expect(o.api.menuLink()).toBeNull();

    await finish();
    o.api.openMenu(backlink());
    expect(o.api.menuLink()).not.toBeNull();
  });

  it("neither prompts for nor performs a delete", async () => {
    const o = ops();
    const finish = await reorderInFlight(o);

    // One row with children (the prompt branch) and one without (the
    // direct write): the guard has to sit before both.
    await o.api.requestDelete(withChildren(2));
    await o.api.requestDelete(backlink());
    expect(o.api.deletePrompt()).toBeNull();
    expect(commands.backlinkRowOp).not.toHaveBeenCalled();

    await finish();
  });

  it("refuses a row write", async () => {
    const o = ops();
    const finish = await reorderInFlight(o);

    await o.api.toggleRowTodo(backlink());
    expect(commands.backlinkRowOp).not.toHaveBeenCalled();

    await finish();
  });
});

describe("an orphan backlink is inert", () => {
  it("opens no editor", () => {
    const o = ops();
    o.api.openEditor(backlink({ source_page: null }));
    expect(o.api.editing()).toBeNull();
  });

  it("opens no context menu", () => {
    const o = ops();
    o.api.openMenu(backlink({ source_page: null }));
    // Not an empty sheet: no sheet.
    expect(o.api.menuLink()).toBeNull();
  });

  it("deletes nothing and asks nothing", async () => {
    const o = ops();
    await o.api.requestDelete(backlink({ source_page: null }));
    expect(o.api.deletePrompt()).toBeNull();
    expect(commands.backlinkRowOp).not.toHaveBeenCalled();
    expect(commands.deleteBlock).not.toHaveBeenCalled();
  });
});

describe("an unchanged draft writes nothing", () => {
  it("leaves without a command or a re-ask", async () => {
    const o = ops();
    o.api.openEditor(backlink());

    // `rawTextWithTodo` rebuilds the canonical word form, so an
    // unconditional commit rewrites `[ ] buy milk` to `TODO buy milk`
    // on a tap in and out — and buys an O(blocks) rescan for it.
    await o.api.commitRow(backlink(), "cites [[project]]");

    expect(commands.editBacklinkBlock).not.toHaveBeenCalled();
    expect(o.onEditCommitted).not.toHaveBeenCalled();
    expect(o.api.editing()).toBeNull();
  });
});

describe("deleting a row whose subtree is not on screen", () => {
  it("goes straight through for a childless block", async () => {
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const o = ops();

    await o.api.requestDelete(backlink());

    expect(o.api.deletePrompt()).toBeNull();
    expect(commands.backlinkRowOp).toHaveBeenCalledWith("p1", "b1", "delete");
  });

  it("asks first when children would go with it", async () => {
    const o = ops();

    await o.api.requestDelete(withChildren(3));

    // Same threshold the outline's own delete uses, and the row here
    // shows *less*: a backlink renders one flat block, so the subtree
    // about to be trashed is invisible.
    expect(o.api.deletePrompt()).toEqual({ id: "b1", descendants: 3 });
    expect(commands.backlinkRowOp).not.toHaveBeenCalled();
  });

  it("deletes on confirm and clears the prompt", async () => {
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const o = ops();
    const target = withChildren(1);

    await o.api.requestDelete(target);
    // The link arrives from the caller: `<BlockDeleteConfirm />` closes
    // itself before it confirms, so reading it back out of the prompt
    // would read `null` and delete nothing — silently.
    await o.api.deleteRow(target);

    expect(commands.backlinkRowOp).toHaveBeenCalledWith("p1", "b1", "delete");
    expect(o.api.deletePrompt()).toBeNull();
  });

  it("writes nothing on cancel", async () => {
    const o = ops();

    await o.api.requestDelete(withChildren(1));
    o.api.cancelDelete();

    expect(commands.backlinkRowOp).not.toHaveBeenCalled();
    expect(o.api.deletePrompt()).toBeNull();
  });
});
