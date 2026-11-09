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
// `Cmd/Ctrl+Shift+V` reads the clipboard on the backend (the macOS
// WKWebView prompts for a web-API read outside a paste gesture), so the
// plugin is the seam for the paste-without-formatting path.
const clipboard = vi.hoisted(() => ({ readText: vi.fn() }));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => clipboard);

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

describe("BlockRow image rendering — #322", () => {
  it("renders an inline image as an <img> rather than a file chip", () => {
    const block: BlockNode = {
      ...makeBlock("blk-img", "![diagram](https://example.com/diagram.png)"),
      tokens: [
        {
          kind: "image",
          alt: "diagram",
          href: "https://example.com/diagram.png",
        },
      ],
    };
    const host = mountView(block, makeCb());
    const img = host.querySelector('img[alt="diagram"]');
    expect(img).not.toBeNull();
    expect(img?.getAttribute("src")).toBe("https://example.com/diagram.png");
    expect(img?.getAttribute("alt")).toBe("diagram");
  });

  it("preserves desktop inline style for refs and tags alongside images", () => {
    const block: BlockNode = {
      ...makeBlock(
        "blk-ref-tag",
        "[[project]] #urgent ![diagram](https://example.com/diagram.png)",
      ),
      tokens: [
        { kind: "ref", value: "project" },
        { kind: "plain", value: " " },
        { kind: "tag", value: "#urgent" },
        { kind: "plain", value: " " },
        {
          kind: "image",
          alt: "diagram",
          href: "https://example.com/diagram.png",
        },
      ],
    };
    const host = mountView(block, makeCb());
    const img = host.querySelector('img[alt="diagram"]');
    expect(img).not.toBeNull();

    // Verify ref and tag use desktop inline classes, not mobile pill chips
    const buttons = host.querySelectorAll('[role="button"]');
    const refSpan = Array.from(buttons).find((b) => b.textContent === "project");
    expect(refSpan).toBeDefined();
    expect(refSpan?.className).toContain("text-(--color-outl-ref-link-fg)");
    expect(refSpan?.className).not.toContain("bg-(--color-outl-accent)/12");

    const tagSpan = Array.from(buttons).find((b) => b.textContent === "#urgent");
    expect(tagSpan).toBeDefined();
    expect(tagSpan?.className).toContain("text-(--color-outl-tag-link-fg)");
  });
});

/**
 * Regression for #488: a paste rewrites the block the user is typing
 * in, and the draft is seeded once per row (only `startEdit` re-seeds
 * it). So re-rendering the outline left the textarea on the pre-paste
 * string, and the next Esc / blur committed *that* back over the paste
 * — the pasted link vanished from the block and from the `.md`, with
 * nothing on screen to say so.
 *
 * The editor applies the update the paste callback resolves to. These
 * pin both halves: the textarea shows the pasted text with the caret
 * after it, and the commit that follows carries the pasted text.
 */
function pasteEvent(flavours: Record<string, string>): ClipboardEvent {
  // jsdom's `ClipboardEvent` ignores `clipboardData` in its init dict.
  const ev = new Event("paste", {
    bubbles: true,
    cancelable: true,
  }) as ClipboardEvent;
  Object.defineProperty(ev, "clipboardData", {
    value: { getData: (type: string) => flavours[type] ?? "" },
  });
  return ev;
}

/** Let the paste handler's awaits settle. */
function flush(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0));
}

describe("BlockRow paste into the block being edited — #488", () => {
  const HOST_AFTER = "Some text[See the invoice](https://example.test/i)";

  it("shows the pasted text and commits it, not the stale draft", async () => {
    const cb = makeCb({
      onPasteMarkdown: vi
        .fn()
        .mockResolvedValue({ text: HOST_AFTER, caret: HOST_AFTER.length }),
    });
    const block = makeBlock("blk-paste", "Some text");
    const ta = mountEditing(block, cb);
    await flush();
    ta.setSelectionRange(9, 9);

    const ev = pasteEvent({
      // A link copied out of an email: HTML only, so `choosePasteRoute`
      // routes it `rich` and the backend gets `[label](url)`.
      "text/html": '<a href="https://example.test/i">See the invoice</a>',
      "text/plain": "See the invoice",
    });
    ta.dispatchEvent(ev);
    await flush();

    expect(ev.defaultPrevented).toBe(true);
    expect(cb.onPasteMarkdown).toHaveBeenCalled();
    expect(ta.value).toBe(HOST_AFTER);
    expect(ta.selectionStart).toBe(HOST_AFTER.length);

    // The part that was silent data loss: Esc used to write "Some text".
    ta.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Escape",
        bubbles: true,
        cancelable: true,
      }),
    );
    await flush();
    expect(cb.onCommit).toHaveBeenCalledWith("blk-paste", HOST_AFTER);
  });

  it("leaves the draft alone when the paste did not touch the host block", async () => {
    // `null` is what a paste that only created siblings resolves to.
    // Writing the draft anyway would be a round-trip's worth of racing
    // the user's keystrokes for no change.
    const cb = makeCb({ onPasteMarkdown: vi.fn().mockResolvedValue(null) });
    const block = makeBlock("blk-untouched", "Some text");
    const ta = mountEditing(block, cb);
    await flush();

    ta.dispatchEvent(
      pasteEvent({ "text/plain": "- one\n- two", "text/html": "" }),
    );
    await flush();

    expect(cb.onPasteMarkdown).toHaveBeenCalled();
    expect(ta.value).toBe("Some text");
  });

  it("leaves the caret where the reply put it, not at the end", async () => {
    // The reply's caret is not always end-of-text: a splice keeps the
    // user's tail to the right of it. Clamping to the end would drop
    // them past text they were mid-sentence in.
    const cb = makeCb({
      onPasteMarkdown: vi
        .fn()
        .mockResolvedValue({ text: "hello BRAVE world", caret: 12 }),
    });
    const ta = mountEditing(makeBlock("blk-mid", "hello world"), cb);
    await flush();
    ta.setSelectionRange(6, 6);

    ta.dispatchEvent(pasteEvent({ "text/plain": "- a\n- b", "text/html": "" }));
    await flush();

    expect(ta.value).toBe("hello BRAVE world");
    expect(ta.selectionStart).toBe(12);
  });

  it("applies the reply on the paste-without-formatting path too", async () => {
    // `Cmd/Ctrl+Shift+V` is the only desktop path whose caret is
    // mid-string by design (`outl_actions::paste_plain` keeps the tail
    // in the same block), and it had no test at all.
    clipboard.readText.mockResolvedValue("BRAVE ");
    const cb = makeCb({
      onPastePlain: vi
        .fn()
        .mockResolvedValue({ text: "hello BRAVE world", caret: 12 }),
    });
    const ta = mountEditing(makeBlock("blk-plain", "hello world"), cb);
    await flush();
    ta.setSelectionRange(6, 6);

    ta.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "v",
        metaKey: true,
        shiftKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    await flush();

    // The host text goes along so the backend splices at a caret
    // measured against what the textarea is holding.
    expect(cb.onPastePlain).toHaveBeenCalledWith(
      "blk-plain",
      6,
      "BRAVE ",
      "hello world",
    );
    expect(ta.value).toBe("hello BRAVE world");
    expect(ta.selectionStart).toBe(12);
  });
});

/**
 * Regression for #477: Tab / Shift-Tab indented or outdented the
 * edited block but left edit mode, and the selection stayed on
 * whatever block was selected beforehand — so the next keystroke ran
 * as a Normal-mode command against the wrong block.
 *
 * `onIndent` / `onOutdent` themselves (re-entering edit mode on the
 * moved block, even on a failed / no-op move) are pinned in
 * `outline-structure.test.ts`, against the real `<OutlineView />`
 * wiring extracted into `createStructureOps` — a stub `cb` here, as
 * used by every other test in this file, can't exercise that half.
 *
 * What a stub *can* pin is `<BlockEditor />`'s side of the caret
 * handoff: the Tab handler has to capture the caret before `commit()`
 * tears the textarea down, since indenting/outdenting mounts a fresh
 * `<BlockEditor />` once the block re-enters edit mode, and that
 * fresh instance has no memory of where the user's caret was.
 */
describe("BlockRow Tab / Shift-Tab indent-outdent caret handoff — #477", () => {
  it("Tab captures the caret as a caretIntent offset before committing", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-indent", "hello world"), cb);
    // Let this mount's own pending caret-intent microtask (queued by
    // `onMount`, caretIntent is `null` at this point) drain first —
    // otherwise it runs mid-test, after Tab has set a fresh intent,
    // and swallows it before the assertion below ever sees it.
    await Promise.resolve();
    ta.setSelectionRange(5, 5);

    ta.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        bubbles: true,
        cancelable: true,
      }),
    );
    await Promise.resolve();
    await Promise.resolve();

    expect(cb.onIndent).toHaveBeenCalledWith("blk-indent");
    // This stub `cb.onIndent` never flips `editingBlockId`, so the row
    // never remounts and nothing has consumed the intent yet — it is
    // still sitting there, captured, for the real `<OutlineView />`
    // wiring to hand to the block's next mount.
    expect(appState.caretIntent).toBe(5);
  });

  it("Shift+Tab captures the caret and fires onOutdent, not onIndent", async () => {
    const cb = makeCb();
    const ta = mountEditing(makeBlock("blk-outdent", "hello world"), cb);
    await Promise.resolve();
    ta.setSelectionRange(3, 3);

    ta.dispatchEvent(
      new KeyboardEvent("keydown", {
        key: "Tab",
        shiftKey: true,
        bubbles: true,
        cancelable: true,
      }),
    );
    await Promise.resolve();
    await Promise.resolve();

    expect(cb.onOutdent).toHaveBeenCalledWith("blk-outdent");
    expect(cb.onIndent).not.toHaveBeenCalled();
    expect(appState.caretIntent).toBe(3);
  });

  it("a caretIntent offset present on mount lands the caret there, not at the end", async () => {
    // The other half of the handoff: once the moved block remounts
    // into edit mode, the fresh `<BlockEditor />` must read the
    // captured offset back out and clear it, the same contract
    // `EnterInsertAtEnd`'s `"end"` sentinel already had.
    setAppState("caretIntent", 5);
    const ta = mountEditing(makeBlock("blk-remount", "hello world"), makeCb());
    await Promise.resolve();
    await Promise.resolve();

    expect(ta.selectionStart).toBe(5);
    expect(ta.selectionEnd).toBe(5);
    expect(appState.caretIntent).toBeNull();
  });

  it("a caretIntent offset of 0 is honoured, not treated as absent", async () => {
    // `intent &&` would skip a falsy `0` — the caret-at-start case —
    // and leave whatever position focus() happened to land on.
    setAppState("caretIntent", 0);
    const ta = mountEditing(makeBlock("blk-zero", "hello world"), makeCb());
    await Promise.resolve();
    await Promise.resolve();

    expect(ta.selectionStart).toBe(0);
    expect(appState.caretIntent).toBeNull();
  });
});
