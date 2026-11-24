import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Backlink, PageView } from "@outl/shared/api/types";

import { BacklinksSection } from "./BacklinksSection";
import type { BacklinkCommit } from "./BacklinksSection.row-ops";

const commands = vi.hoisted(() => ({
  backlinkRowOp: vi.fn(),
  createBacklinkSibling: vi.fn(),
  editBacklinkBlock: vi.fn(),
  resolveEmbeds: vi.fn(() => Promise.resolve({})),
  resolvePageEmbeds: vi.fn(() => Promise.resolve({})),
  editBlock: vi.fn(),
  toggleTodo: vi.fn(),
  deleteBlock: vi.fn(),
  indentBlock: vi.fn(),
  outdentBlock: vi.fn(),
  moveBlockUp: vi.fn(),
  moveBlockDown: vi.fn(),
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

/** The card header, which still opens the source page. */
function header(host: HTMLElement): HTMLElement {
  return host.querySelector<HTMLElement>("[data-backlink-group]")!;
}

function textarea(host: HTMLElement): HTMLTextAreaElement | null {
  return host.querySelector("textarea");
}

/** One keystroke, as the platform delivers it. */
function type(ta: HTMLTextAreaElement, next: string) {
  ta.value = next;
  ta.dispatchEvent(new InputEvent("input", { bubbles: true }));
}

/** Blur is not a delegated event, so Solid listens on the element. */
function blur(ta: HTMLTextAreaElement) {
  ta.dispatchEvent(new FocusEvent("blur"));
}

const tick = () => new Promise((r) => setTimeout(r, 0));


describe("BacklinksSection breadcrumb", () => {
  // Parity with the desktop: both clients render the trail through
  // `<CrumbTrail />`, so neither can start showing the raw source
  // while the other renders it.
  it("renders the ancestor trail instead of its markdown source", () => {
    const host = mount([backlink()]).host;

    expect(host.textContent).toContain("buser/team/vendas");
    expect(host.textContent).toContain("#alinhamento");
    expect(host.textContent).toContain("email");
    expect(host.textContent).not.toContain("[[");
    expect(host.textContent).not.toContain("](");
    expect(host.textContent).not.toContain("mail.google.com");
  });
});

/**
 * A backlink row is a block that lives on another page, and editing it
 * means the mutation is addressed there. `@outl/shared/backlink-edit`
 * owns both decisions (which pair a gesture targets, what the reply may
 * touch); what is pinned here is this client's half of it.
 */
describe("editing a backlink where it is cited", () => {
  it("a tap on a backlink row opens an editor instead of jumping", () => {
    const h = mount([backlink()]);

    row(h.host).click();

    expect(textarea(h.host)).not.toBeNull();
    expect(h.onJump).not.toHaveBeenCalled();
  });

  it("commits to the source page with the strict command", async () => {
    commands.editBacklinkBlock.mockResolvedValue(sourceView());
    const h = mount([backlink()]);

    row(h.host).click();
    const ta = textarea(h.host)!;
    type(ta, "cites [[project]] and a note");
    blur(ta);
    await tick();

    // `editBlock` tolerates a stale id by appending to the end of the
    // page, which would move the user's text to a page they are not
    // looking at. A backlink row only ever uses the strict command.
    expect(commands.editBacklinkBlock).toHaveBeenCalledWith(
      "p1",
      "b1",
      "cites [[project]] and a note",
    );
    expect(commands.editBlock).not.toHaveBeenCalled();
  });

  it("asks for the rows once per commit, never per keystroke", async () => {
    commands.editBacklinkBlock.mockResolvedValue(sourceView());
    const h = mount([backlink()]);

    row(h.host).click();
    const ta = textarea(h.host)!;
    type(ta, "cites [[project]] a");
    type(ta, "cites [[project]] ab");
    type(ta, "cites [[project]] abc");
    expect(h.onEditCommitted).not.toHaveBeenCalled();

    blur(ta);
    await tick();

    // `pageBacklinks` is an O(blocks-in-workspace) scan (invariant 15).
    expect(h.onEditCommitted).toHaveBeenCalledTimes(1);
  });

  it("writes nothing when the draft comes back unchanged", async () => {
    const h = mount([backlink()]);

    row(h.host).click();
    blur(textarea(h.host)!);
    await tick();

    // The draft is rebuilt from the DTO's split `todo` + `text`, so an
    // unconditional commit rewrites `[ ] buy milk` to `TODO buy milk`
    // on a tap in and out — and buys an O(blocks) rescan for it.
    expect(commands.editBacklinkBlock).not.toHaveBeenCalled();
    expect(h.onEditCommitted).not.toHaveBeenCalled();
    expect(textarea(h.host)).toBeNull();
  });

  it("reports which page the reply describes, and decides nothing about it", async () => {
    const reply = sourceView();
    commands.editBacklinkBlock.mockResolvedValue(reply);
    const h = mount([backlink()]);

    row(h.host).click();
    const ta = textarea(h.host)!;
    type(ta, "edited");
    blur(ta);
    await tick();

    // The reply describes the **source** page, so whether it may be
    // installed as the open page's view is `backlinkCommitPlan`'s
    // answer — and the only place that holds both page ids is the host
    // (`Journal.backlinks.ts`). This section reports the fact and
    // nothing more, which is why it no longer takes `openPageId`.
    expect(h.onEditCommitted).toHaveBeenCalledWith({
      sourcePageId: "p1",
      view: reply,
    });
  });

  it("keeps the editor on the user's own text until the fresh rows land", async () => {
    commands.editBacklinkBlock.mockResolvedValue(sourceView());
    const refresh = heldRefresh();
    const h = mount([backlink()], refresh.fn);

    // Control: the row does render its tokens, so the absence below is
    // a fact about the editor state and not about an empty fixture.
    expect(h.host.textContent).toContain("cites");

    row(h.host).click();
    const ta = textarea(h.host)!;
    type(ta, "the text the user typed");
    blur(ta);
    await tick();

    // The rows are fed by an O(blocks-in-workspace) scan. Close the
    // editor before it answers and the row renders the *pre-edit*
    // tokens it still holds — the user watches their edit revert and
    // then reappear. So the editor stays up on their own text.
    expect(textarea(h.host)?.value).toBe("the text the user typed");
    // The fixture's tokens spell "cites", so this fails if the row
    // renders them under the editor.
    expect(h.host.textContent).not.toContain("cites");

    refresh.land();
    await tick();
    expect(textarea(h.host)).toBeNull();
  });

  it("surfaces the refusal when the block is gone", async () => {
    commands.editBacklinkBlock.mockRejectedValue("block is not in the tree");
    const h = mount([backlink()]);

    row(h.host).click();
    const ta = textarea(h.host)!;
    type(ta, "text the user typed");
    blur(ta);
    await tick();

    expect(h.onError).toHaveBeenCalledWith(
      expect.stringContaining("not in the tree"),
    );
    expect(h.onEditCommitted).not.toHaveBeenCalled();
    // Nothing was written, so the editor keeps the draft rather than
    // dropping text the user can no longer see.
    expect(textarea(h.host)?.value).toBe("text the user typed");
  });

  it("the checkbox toggles the task on the source page", async () => {
    commands.backlinkRowOp.mockResolvedValue(sourceView());
    const h = mount([
      backlink({
        todo: "TODO",
        source_block: {
          ...backlink().source_block,
          todo: "TODO",
        },
      }),
    ]);

    h.host.querySelector<HTMLElement>("[data-backlink-todo]")!.click();
    await tick();

    // The cross-page command, never `toggleTodo`: that one acts on the
    // page the user is looking at and reports a raw `ActionError` for a
    // block id the tree no longer holds, which a backlink's routinely
    // is.
    expect(commands.backlinkRowOp).toHaveBeenCalledWith(
      "p1",
      "b1",
      "toggle-todo",
    );
    expect(commands.toggleTodo).not.toHaveBeenCalled();
    expect(h.onEditCommitted).toHaveBeenCalledTimes(1);
    expect(textarea(h.host)).toBeNull();
  });

  it("the card header still jumps", () => {
    const link = backlink();
    const h = mount([link]);

    header(h.host).click();

    expect(h.onJump).toHaveBeenCalledWith(link);
    expect(textarea(h.host)).toBeNull();
  });

  it("an orphan backlink stays read-only", async () => {
    // `todo` is set on purpose: with `todo: null` no checkbox renders
    // at all, so the inert-marker assertion below could not fail and
    // the guard it covers would be deletable with a green suite.
    const h = mount([
      backlink({
        source_page: null,
        todo: "TODO",
        source_block: { ...backlink().source_block, todo: "TODO" },
      }),
    ]);

    row(h.host).click();
    await tick();

    // No page to address the mutation to, and guessing one would write
    // to the wrong document.
    expect(textarea(h.host)).toBeNull();
    expect(commands.editBacklinkBlock).not.toHaveBeenCalled();
    // The marker still draws (the block *is* a task) — it is the
    // button around it that an orphan does not get.
    expect(h.host.querySelector("[data-backlink-todo]")).toBeNull();
    expect(h.host.querySelector('[aria-label="Toggle task"]')).toBeNull();
    // Nor does its row pretend to navigate: the group header for an
    // orphan has no page either, so the tap is simply inert.
    expect(h.onJump).not.toHaveBeenCalled();
  });
});
