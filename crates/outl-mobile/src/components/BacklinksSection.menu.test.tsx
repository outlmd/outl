import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, PageView } from "@outl/shared/api/types";

import { BacklinksSection } from "./BacklinksSection";
import type { BacklinkCommit } from "./BacklinksSection.row-ops";

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

let dispose: (() => void) | undefined;

beforeEach(() => {
  vi.clearAllMocks();
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

/** The trail from the bug report: a tag, a namespaced ref and a mail
 *  link, in the branch the citing block sits under. */
const ancestors: Backlink["ancestors"] = [
  {
    id: "crumb-1",
    text: "#alinhamento [[buser/team/vendas]] internet 0800 [email](https://mail.google.com/mail?authuser=someone)",
    tokens: [
      { kind: "tag", value: "#alinhamento" },
      { kind: "plain", value: " " },
      { kind: "ref", value: "buser/team/vendas" },
      { kind: "plain", value: " internet 0800 " },
      {
        kind: "link",
        value: "email",
        href: "https://mail.google.com/mail?authuser=someone",
      },
    ],
  },
];

function backlink(over: Partial<Backlink> = {}): Backlink {
  return {
    block_id: "b1",
    todo: null,
    ancestors,
    source_page: { id: "p1", slug: "journal", title: "Journal", kind: "journal" },
    source_block: {
      id: "b1",
      text: "cites [[project]]",
      todo: null,
      tokens: [{ kind: "plain", value: "cites " }],
      collapsed: false,
      properties: [],
      children: [],
    },
    source_block_path: [],
    ...over,
  };
}

/** A reply of the source page, which is all a cross-page mutation
 *  returns. Only the identity matters to this component. */
function sourceView(): PageView {
  return { page: { id: "p1", slug: "journal" } } as unknown as PageView;
}

interface Harness {
  host: HTMLElement;
  onJump: ReturnType<typeof vi.fn>;
  onEditCommitted: CommitSpy;
  onError: ReturnType<typeof vi.fn>;
  onToggleOrder: ReturnType<typeof vi.fn>;
}

/** `vi.fn` needs the signature: the section awaits what this returns,
 *  so a spy that cannot be typed as returning a promise cannot stand in
 *  for the host's late-resolving re-ask. */
type CommitSpy = ReturnType<
  typeof vi.fn<(commit: BacklinkCommit) => void | Promise<void>>
>;
const commitSpy = (): CommitSpy =>
  vi.fn<(commit: BacklinkCommit) => void | Promise<void>>();

/** A re-ask that does not resolve until `land()` is called — the gap a
 *  committed row must not render its pre-edit text in. */
function heldRefresh() {
  let land = () => {};
  const fn = commitSpy().mockImplementation(
    () => new Promise<void>((resolve) => (land = resolve)),
  );
  return { fn, land: () => land() };
}

/**
 * `onEditCommitted` may resolve a promise, which is what lets the
 * section hold the editor open until the fresh rows land. Tests that
 * care about that gap hand in their own deferred resolver.
 */
function mount(
  backlinks: Backlink[],
  onEditCommitted: CommitSpy = commitSpy(),
): Harness {
  const onJump = vi.fn();
  const onError = vi.fn();
  const onToggleOrder = vi.fn();
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BacklinksSection
        backlinks={backlinks}
        onJump={onJump}
        order="newest"
        onToggleOrder={onToggleOrder}
        // Wrapped rather than passed straight through: a bare `vi.fn()`
        // has no call signature to match the prop's, and the arrow
        // still records the call *and* forwards the promise the
        // section awaits.
        onEditCommitted={(commit) => onEditCommitted(commit)}
        onError={(message) => onError(message)}
      />
    ),
    host,
  );
  return { host, onJump, onEditCommitted, onError, onToggleOrder };
}

/** The row itself, which is what a tap opens the editor on. */
function row(host: HTMLElement): HTMLElement {
  return host.querySelector<HTMLElement>("[data-backlink-row]")!;
}

function textarea(host: HTMLElement): HTMLTextAreaElement | null {
  return host.querySelector("textarea");
}

const tick = () => new Promise((r) => setTimeout(r, 0));

/**
 * Hold a finger on a row past `createLongPress`'s 450ms threshold.
 *
 * **The `click()` is not decoration.** A real completed hold is
 * followed by a `click`, which is the whole reason `consumedClick()`
 * exists — without dispatching it here, the assertion that a long press
 * does not also open the editor holds whether or not the guard is
 * there. Same shape as `JournalChrome.test.tsx` and
 * `PageSwitcher.test.tsx`.
 */
async function longPress(el: HTMLElement) {
  el.dispatchEvent(
    new PointerEvent("pointerdown", { bubbles: true, clientX: 1, clientY: 1 }),
  );
  await new Promise((r) => setTimeout(r, 520));
  el.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
  el.click();
}

/** Labels of the rows the long-press sheet is currently offering. */
function menuLabels(): string[] {
  return [...document.querySelectorAll('[data-backlink-menu] button')]
    .map((b) => b.textContent?.trim() ?? "")
    .filter((label) => label.length > 0 && label !== "Drag to close");
}

/** Tap a row of the long-press sheet by its label. */
async function tapMenu(label: string) {
  const hit = [...document.querySelectorAll('[data-backlink-menu] button')].find(
    (b) => b.textContent?.trim() === label,
  );
  if (!hit) throw new Error(`no menu row "${label}" (have: ${menuLabels()})`);
  (hit as HTMLElement).click();
  await tick();
}

/** The confirm dialog a delete raises when the row has children. */
const dialog = () =>
  document.querySelector('[role="alertdialog"]') as HTMLElement | null;

/** A `Backlink` whose citing block carries `count` children. */
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

/**
 * Mobile has no chord surface, so where the desktop binds a key the
 * phone offers a row in `<BlockContextMenu>` — the same sheet the
 * outline's long-press opens.
 *
 * What each row *does* is `BacklinksSection.row-ops.ts`'s, and is
 * covered there. What is pinned here is the gesture reaching it: the
 * hold, the sheet, and the two states in which no sheet may open.
 */
describe("the backlink row's long press", () => {
  it("opens the context menu", async () => {
    const h = mount([backlink()]);

    await longPress(row(h.host));

    expect(menuLabels()).toContain("Indent");
    // The hold must not also open the editor: the `click` a completed
    // hold produces is swallowed, or every long press would end in
    // Insert on top of the sheet.
    expect(textarea(h.host)).toBeNull();
  });

  it("runs the tapped row's op on the source page", async () => {
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const h = mount([backlink()]);

    await longPress(row(h.host));
    await tapMenu("Indent");

    expect(commands.backlinkRowOp).toHaveBeenCalledWith("p1", "b1", "indent");
    // `indentBlock` uses `ensure_in_tree`, not `ensure_live` — because
    // `trash::restore` is itself a move — so on a row whose block a peer
    // trashed it re-parents that block *inside the trash*, out of the
    // trash listing and out of the user's reach. It answers `Ok` doing
    // it, which is why the assertion is the call and not a message.
    expect(commands.indentBlock).not.toHaveBeenCalled();
  });

  it("creates the sibling beside the citing block, not at the page end", async () => {
    commands.createBacklinkSibling.mockResolvedValue({
      view: sourceView(),
      new_id: "fresh",
    });
    const h = mount([backlink()]);

    // The sheet is this client's only door to `o` / `O`: there is no
    // chord surface at all.
    await longPress(row(h.host));
    await tapMenu("New block below");

    expect(commands.createBacklinkSibling).toHaveBeenCalledWith(
      "p1",
      "b1",
      "below",
    );
    // `createBlock` appends at the end of the page when its anchor is
    // stale, so this reported "Added an empty block after this
    // reference" about a block that went somewhere else entirely — and
    // one junk block per repeat while the row was held.
    expect(commands.createBlock).not.toHaveBeenCalled();
  });

  it("offers nothing on an orphan backlink", async () => {
    const h = mount([
      backlink({
        source_page: null,
        todo: "TODO",
        source_block: { ...backlink().source_block, todo: "TODO" },
      }),
    ]);

    await longPress(row(h.host));

    // Not "an empty sheet": no sheet. There is no page to address any
    // of these to, and guessing one writes to the wrong document.
    expect(document.querySelector("[data-backlink-menu]")).toBeNull();
    expect(commands.backlinkRowOp).not.toHaveBeenCalled();
    expect(commands.indentBlock).not.toHaveBeenCalled();
  });

  it("is refused while a write on this card is in flight", async () => {
    const refresh = heldRefresh();
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const h = mount(
      [
        backlink({
          todo: "TODO",
          source_block: { ...backlink().source_block, todo: "TODO" },
        }),
      ],
      refresh.fn,
    );

    // A checkbox toggle, so the row (and its hold target) is still
    // rendered while the write runs.
    h.host.querySelector<HTMLElement>("[data-backlink-todo]")!.click();
    await tick();

    // A long press firing an op mid-write is the concurrent-write
    // defect through a different door.
    await longPress(row(h.host));
    expect(document.querySelector("[data-backlink-menu]")).toBeNull();

    refresh.land();
    await tick();
  });

  it("refuses the direction toggle while a write is in flight", async () => {
    const refresh = heldRefresh();
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const h = mount(
      [
        backlink({
          todo: "TODO",
          source_block: { ...backlink().source_block, todo: "TODO" },
        }),
      ],
      refresh.fn,
    );
    const toggle = () =>
      [...h.host.querySelectorAll("button")]
        .find((b) => b.textContent?.includes("Newest"))!
        .click();

    h.host.querySelector<HTMLElement>("[data-backlink-todo]")!.click();
    await tick();

    // A reorder is a write too: it lands a second `PageBacklinks` into
    // the rows the commit's re-ask is about to replace.
    toggle();
    expect(h.onToggleOrder).not.toHaveBeenCalled();

    refresh.land();
    await tick();
    toggle();
    expect(h.onToggleOrder).toHaveBeenCalledTimes(1);
  });

  it("refuses a row write while the direction toggle's re-ask is in flight", async () => {
    // The opposite direction of the test above, and the one that was
    // unguarded: a row write landing under a reorder's re-ask is replaced
    // by rows that predate it, and the user sees their tap undone.
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const h = mount([
      backlink({
        todo: "TODO",
        source_block: { ...backlink().source_block, todo: "TODO" },
      }),
    ]);
    let land = () => {};
    h.onToggleOrder.mockImplementation(
      () => new Promise<void>((resolve) => (land = resolve)),
    );
    [...h.host.querySelectorAll("button")]
      .find((b) => b.textContent?.includes("Newest"))!
      .click();
    await tick();

    h.host.querySelector<HTMLElement>("[data-backlink-todo]")!.click();
    await tick();
    expect(commands.backlinkRowOp).not.toHaveBeenCalled();
    // The guard is card-wide, but the dim is the acting row's only.
    expect(h.host.querySelector('[data-backlink-pending="true"]')).toBeNull();

    land();
    await tick();
    h.host.querySelector<HTMLElement>("[data-backlink-todo]")!.click();
    await tick();
    expect(commands.backlinkRowOp).toHaveBeenCalledTimes(1);
  });

  it("marks the row busy while its write is in flight", async () => {
    const refresh = heldRefresh();
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const h = mount(
      [
        backlink({
          todo: "TODO",
          source_block: { ...backlink().source_block, todo: "TODO" },
        }),
      ],
      refresh.fn,
    );

    h.host.querySelector<HTMLElement>("[data-backlink-todo]")!.click();
    await tick();

    // A cross-page write plus an O(blocks-in-workspace) re-ask is long
    // enough that a card which looks idle reads as a tap that did
    // nothing.
    expect(
      h.host.querySelector('[data-backlink-pending="true"]'),
    ).not.toBeNull();

    refresh.land();
    await tick();
    expect(h.host.querySelector('[data-backlink-pending="true"]')).toBeNull();
  });

  it("leaves another row's open editor alone when it deletes this one", async () => {
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const refresh = heldRefresh();
    const first = backlink();
    const second = backlink({
      block_id: "b2",
      source_block: { ...first.source_block, id: "b2" },
    });
    const h = mount([first, second], refresh.fn);

    // A row being edited has no hold recogniser mounted at all, so the
    // sheet can never be raised on the row whose editor is up. The
    // reachable case is the *other* row's sheet with this editor still
    // open: tapping a sheet row moves no focus on a phone.
    h.host.querySelector<HTMLElement>('[data-backlink-row="b2"]')!.click();
    expect(textarea(h.host)).not.toBeNull();
    await longPress(
      h.host.querySelector<HTMLElement>('[data-backlink-row="b1"]')!,
    );
    await tapMenu("Delete");
    await tick();

    expect(commands.backlinkRowOp).toHaveBeenCalledWith("p1", "b1", "delete");
    // The draft is b2's, and b2 was not deleted.
    expect(textarea(h.host)).not.toBeNull();

    refresh.land();
    await tick();
  });

  it("asks before deleting a row whose children are not on screen", async () => {
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const h = mount([withChildren(2)]);

    await longPress(row(h.host));
    await tapMenu("Delete");

    // A backlink row renders one flat block, so the subtree about to
    // go with it is invisible here — the user has *less* information
    // than in the outline, where this prompt already exists. The
    // wording is `JournalDeleteDialogs`'s, not a third copy.
    expect(dialog()?.textContent).toContain("2 children");
    expect(commands.backlinkRowOp).not.toHaveBeenCalled();

    [...dialog()!.querySelectorAll("button")]
      .find((b) => b.textContent?.trim() === "Delete")!
      .click();
    await tick();

    expect(commands.backlinkRowOp).toHaveBeenCalledWith("p1", "b1", "delete");
  });
});
