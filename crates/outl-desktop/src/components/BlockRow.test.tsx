import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";
import { listTemplates } from "@outl/shared/api/commands";
import { BlockRow, type BlockCallbacks } from "./BlockRow";
import { appState, setAppState } from "../lib/store";

/**
 * Regression for #119: in the Mac app, pressing Enter inside a block
 * inserted a literal `\n` instead of creating a new block. The fix
 * intercepts a bare `Enter` in `handleKeydown` and routes it to
 * `onEnter` (commit + sibling below, TUI parity), while `Shift+Enter`
 * stays the soft break (falls through to the textarea → `\n`).
 *
 * These pin that (a) plain Enter fires `onEnter` and prevents the
 * default newline, and (b) Shift+Enter does NOT fire `onEnter` so the
 * multi-line soft break survives.
 */

// Only the Tauri-backed helpers BlockRow imports need stubbing; the
// autocomplete round-trips resolve to empty so no popup opens.
vi.mock("@outl/shared/api/commands", () => ({
  openRef: vi.fn(),
  pluginList: vi.fn().mockResolvedValue([]),
  searchEmojis: vi.fn().mockResolvedValue([]),
  searchPages: vi.fn().mockResolvedValue([]),
  searchPersons: vi.fn().mockResolvedValue([]),
  listTemplates: vi.fn().mockResolvedValue([]),
}));
vi.mock("@outl/shared/plugins/transformer-registry", () => ({
  runTransform: vi.fn(),
  transformerFor: vi.fn(() => null),
}));

let dispose: (() => void) | undefined;

function makeBlock(id: string, text = "hello"): BlockNode {
  return {
    id,
    text,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  };
}

function makeCb(over: Partial<BlockCallbacks> = {}): BlockCallbacks {
  return {
    onStartEdit: vi.fn(),
    onCommit: vi.fn().mockResolvedValue(undefined),
    onEnter: vi.fn().mockResolvedValue(undefined),
    onCreateBefore: vi.fn().mockResolvedValue(undefined),
    onIndent: vi.fn().mockResolvedValue(undefined),
    onOutdent: vi.fn().mockResolvedValue(undefined),
    onDeleteEmpty: vi.fn().mockResolvedValue(undefined),
    onToggleTodo: vi.fn().mockResolvedValue(undefined),
    onToggleCollapsed: vi.fn().mockResolvedValue(undefined),
    onPasteMarkdown: vi.fn().mockResolvedValue(undefined),
    onPastePlain: vi.fn().mockResolvedValue(undefined),
    onRunCodeBlock: vi.fn().mockResolvedValue(undefined),
    onRunPluginCommand: vi.fn().mockResolvedValue(undefined),
    onRefClick: vi.fn(),
    onTagClick: vi.fn(),
    onOpenPage: vi.fn(),
    ...over,
  };
}

function mountEditing(block: BlockNode, cb: BlockCallbacks): HTMLTextAreaElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BlockRow
        block={block}
        depth={0}
        editingId={block.id}
        visualSet={null}
        cb={cb}
      />
    ),
    host,
  );
  const ta = host.querySelector("textarea");
  if (!ta) throw new Error("textarea did not render in edit mode");
  return ta;
}

afterEach(() => {
  dispose?.();
  dispose = undefined;
  setAppState("caretIntent", null);
  document.body.innerHTML = "";
});

describe("BlockRow Enter key — #119", () => {
  it("plain Enter fires onEnter (new block below) and prevents the newline", async () => {
    const cb = makeCb();
    const block = makeBlock("blk-1");
    const ta = mountEditing(block, cb);

    const ev = new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
    });
    ta.dispatchEvent(ev);
    await Promise.resolve();

    // The caret (3rd arg) rides along so the backend can split the block
    // at that offset (issue #184). It defaults to the end of the text
    // ("hello" → 5) when the textarea mounts with the caret at the end.
    expect(cb.onEnter).toHaveBeenCalledWith("blk-1", "hello", 5);
    expect(ev.defaultPrevented).toBe(true);
  });

  it("plain Enter mid-text passes the caret offset so the backend splits there", async () => {
    const cb = makeCb();
    const block = makeBlock("blk-split", "hello world");
    const ta = mountEditing(block, cb);
    // Caret between "hello" and " world".
    ta.setSelectionRange(5, 5);

    const ev = new KeyboardEvent("keydown", {
      key: "Enter",
      bubbles: true,
      cancelable: true,
    });
    ta.dispatchEvent(ev);
    await Promise.resolve();

    expect(cb.onEnter).toHaveBeenCalledWith("blk-split", "hello world", 5);
    expect(ev.defaultPrevented).toBe(true);
  });

  it("Shift+Enter does NOT fire onEnter (soft break stays in the block)", async () => {
    const cb = makeCb();
    const block = makeBlock("blk-2");
    const ta = mountEditing(block, cb);

    const ev = new KeyboardEvent("keydown", {
      key: "Enter",
      shiftKey: true,
      bubbles: true,
      cancelable: true,
    });
    ta.dispatchEvent(ev);
    await Promise.resolve();

    expect(cb.onEnter).not.toHaveBeenCalled();
  });
});

describe("BlockRow blockquote chrome — #140", () => {
  it("keeps the outline bullet outside the quoted body", () => {
    const host = mountView(makeBlock("blk-quote", "> quoted"), makeCb());
    const bullet = host.querySelector("button[aria-label='Mark as TODO']");
    const quote = host.querySelector(".border-l-2");

    expect(bullet).not.toBeNull();
    expect(quote).not.toBeNull();
    expect(quote?.contains(bullet)).toBe(false);
  });
});

/**
 * A `call:<name>` fence's language chip doubles as a link to the
 * template's page: it resolves `<name> → slug` via `listTemplates`,
 * turns the chip into a button on a match, and stays an inert label
 * when no template owns the name. Navigation goes through
 * `onOpenPage` (exact `openPageBySlug`), never `onRefClick`/`openRef`
 * (which would *create* a page on a miss).
 */
function mountView(block: BlockNode, cb: BlockCallbacks): HTMLDivElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BlockRow
        block={block}
        depth={0}
        editingId={"someone-else"}
        visualSet={null}
        cb={cb}
      />
    ),
    host,
  );
  return host;
}

describe("BlockRow call: fence → template link", () => {
  it("renders the chip as a link that opens the template page", async () => {
    vi.mocked(listTemplates).mockResolvedValue([
      { name: "foo", slug: "templates/foo" },
    ]);
    const cb = makeCb();
    const block = makeBlock("blk-c1", "```call:foo\nkey: val\n```");
    const host = mountView(block, cb);

    await vi.waitFor(() => {
      if (!host.querySelector("button[title^='Open template']")) {
        throw new Error("template link not rendered yet");
      }
    });

    const link = host.querySelector(
      "button[title^='Open template']",
    ) as HTMLButtonElement;
    link.click();
    expect(cb.onOpenPage).toHaveBeenCalledWith("templates/foo");
  });

  it("leaves the chip inert when no template owns the name", async () => {
    vi.mocked(listTemplates).mockResolvedValue([]);
    const cb = makeCb();
    const block = makeBlock("blk-c2", "```call:missing\nx\n```");
    const host = mountView(block, cb);

    // Let the (empty) resolve settle, then assert no link appeared.
    await Promise.resolve();
    await Promise.resolve();
    expect(host.querySelector("button[title^='Open template']")).toBeNull();
    expect(cb.onOpenPage).not.toHaveBeenCalled();
  });
});

/**
 * The ghost-block policy, client half (issue #213 item 1, [RFC 0129]).
 *
 * **Never write an op for something the user did not do.** The op log is
 * append-only and every op syncs to every device, so a commit that changes
 * nothing is not free: it is a permanent row, it shows up in page history as a
 * change the user did not make, and it re-triggers projection on every peer.
 *
 * `commit()` enforces it with `if (raw !== wire)`. That was one line with
 * nothing pinning it, and it is the kind of line a refactor drops without
 * noticing — the visible behaviour (leaving edit mode) is identical either way.
 * `Workspace::op_is_noop` is the core-side twin; this is the half that stops the
 * op being *emitted* in the first place.
 */
describe("BlockRow commit — the ghost-block policy (#213)", () => {
  it("does not commit when the text is unchanged", async () => {
    const cb = makeCb();
    const block = makeBlock("blk-noop", "unchanged");
    const ta = mountEditing(block, cb);

    // The user focused the block, typed nothing, and left.
    ta.dispatchEvent(new FocusEvent("blur", { bubbles: true }));
    await Promise.resolve();
    await Promise.resolve();

    expect(
      cb.onCommit,
      "an unchanged block must not reach the backend — that is a ghost op",
    ).not.toHaveBeenCalled();
  });

  it("still leaves edit mode when the text is unchanged", async () => {
    // The mirror case, and the reason the early return cannot simply be
    // "do nothing": an Esc on an unmodified block used to leave the textarea
    // visible with raw markdown showing. Suppressing the op must not suppress
    // the mode flip.
    const cb = makeCb();
    const block = makeBlock("blk-esc", "unchanged");
    const ta = mountEditing(block, cb);

    ta.dispatchEvent(new FocusEvent("blur", { bubbles: true }));
    await Promise.resolve();
    await Promise.resolve();

    expect(appState.editingBlockId).toBeNull();
  });

  it("commits when the user actually changed the text", async () => {
    // Guards the guard: a test that only asserts "did not commit" passes just
    // as well if commit never fires at all.
    const cb = makeCb();
    const block = makeBlock("blk-edit", "before");
    const ta = mountEditing(block, cb);

    ta.value = "after";
    ta.dispatchEvent(new Event("input", { bubbles: true }));
    await Promise.resolve();
    ta.dispatchEvent(new FocusEvent("blur", { bubbles: true }));
    await Promise.resolve();
    await Promise.resolve();

    expect(cb.onCommit).toHaveBeenCalledWith("blk-edit", "after");
  });
});
