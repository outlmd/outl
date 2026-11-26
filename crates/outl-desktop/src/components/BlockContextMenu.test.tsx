import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";
import { copyBlockEmbed, copyBlockRef } from "@outl/shared/api/commands";

import { copyBlockHandleToClipboard } from "../lib/clipboard-handlers";
import { setAppState } from "../lib/store";
import { BlockRow, type BlockCallbacks } from "./BlockRow";

/**
 * The bullet's right-click menu.
 *
 * The chord path is pinned by `clipboard-handlers.refer.test.ts`; this
 * is the other half of invariant 12's promise, because `support.rs`
 * says "the desktop performs this" without saying a user has to know a
 * chord to get at it. Nothing in the catalog can see a gesture.
 *
 * The `cb` here routes the two rows through the **real**
 * `copyBlockHandleToClipboard`, the same helper `<OutlineView />`
 * hands them, so a row is followed all the way to the command and the
 * clipboard rather than stopping at a spy.
 *
 * Styling is deliberately unasserted. Colours come from
 * `outl_theme::Palette` through `--color-outl-*` and
 * `crates/outl-theme/tests/tokens.rs` is what guards them; a hex
 * literal pinned here would be a second opinion about the theme.
 */

vi.mock("@outl/shared/api/commands", () => ({
  copyBlockEmbed: vi.fn(),
  copyBlockRef: vi.fn(),
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
// The OS clipboard is the plugin's (`lib/os-clipboard.ts`), so the
// write a row ends in is observed there.
const writeText = vi.hoisted(() => vi.fn((_text: string) => Promise.resolve()));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  readText: vi.fn(),
  writeText,
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

/** Only the members a read-mode row actually reaches. The two under
 *  test are optional by design, so `over` is how a case drops them. */
function makeCb(over: Partial<BlockCallbacks> = {}): BlockCallbacks {
  return {
    onStartEdit: vi.fn(),
    onCommit: vi.fn().mockResolvedValue(undefined),
    onEnter: vi.fn().mockResolvedValue(undefined),
    onCreateBefore: vi.fn().mockResolvedValue(undefined),
    resolveCaretExit: vi.fn().mockReturnValue(null),
    openCaretLanding: vi.fn(),
    onIndent: vi.fn().mockResolvedValue(undefined),
    onOutdent: vi.fn().mockResolvedValue(undefined),
    onMoveUp: vi.fn().mockResolvedValue(undefined),
    onMoveDown: vi.fn().mockResolvedValue(undefined),
    onDeleteEmpty: vi.fn().mockResolvedValue(undefined),
    onToggleTodo: vi.fn().mockResolvedValue(undefined),
    onToggleCollapsed: vi.fn().mockResolvedValue(undefined),
    onPasteMarkdown: vi.fn().mockResolvedValue(null),
    onPastePlain: vi.fn().mockResolvedValue(null),
    onRunCodeBlock: vi.fn().mockResolvedValue(undefined),
    onRunPluginCommand: vi.fn().mockResolvedValue(undefined),
    onRefClick: vi.fn(),
    onTagClick: vi.fn(),
    onOpenPage: vi.fn(),
    onFocusBlock: vi.fn(),
    // What `<OutlineView />` wires, helper included.
    onCopyBlockRef: (id) => {
      void copyBlockHandleToClipboard(copyBlockRef(id));
    },
    onCopyBlockEmbed: (id) => {
      void copyBlockHandleToClipboard(copyBlockEmbed(id));
    },
    ...over,
  };
}

function mount(cb: BlockCallbacks, block = makeBlock("blk-a")): HTMLElement {
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BlockRow block={block} depth={0} editingId={null} visualSet={null} cb={cb} />
    ),
    host,
  );
  return host;
}

/** The bullet is the only button carrying a zoom / TODO aria-label. */
function bulletOf(host: HTMLElement): HTMLButtonElement {
  const el = host.querySelector<HTMLButtonElement>(
    'button[aria-label="Zoom in on block"]',
  );
  if (!el) throw new Error("bullet not found");
  return el;
}

function rightClick(el: HTMLElement): MouseEvent {
  const e = new MouseEvent("contextmenu", {
    bubbles: true,
    cancelable: true,
    button: 2,
  });
  el.dispatchEvent(e);
  return e;
}

function menuOf(host: HTMLElement): HTMLElement | null {
  return host.querySelector<HTMLElement>('[role="menu"]');
}

function rowOf(host: HTMLElement, id: string): HTMLButtonElement {
  const el = host.querySelector<HTMLButtonElement>(`[data-menu-action="${id}"]`);
  if (!el) throw new Error(`menu row ${id} not found`);
  return el;
}

beforeEach(() => {
  vi.clearAllMocks();
  setAppState({ selectedBlockId: null });
});

afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

describe("opening the menu", () => {
  it("right-clicking the bullet opens it and keeps the native menu away", () => {
    const host = mount(makeCb());
    expect(menuOf(host)).toBeNull();

    const e = rightClick(bulletOf(host));

    expect(menuOf(host)).not.toBeNull();
    // Nothing else in this client cancels a contextmenu, so without
    // this the webview's own menu covers ours.
    expect(e.defaultPrevented).toBe(true);
  });

  it("moves the selection cursor onto the block it was opened on", () => {
    const host = mount(makeCb());
    rightClick(bulletOf(host));

    // The rows act on a block id, and the user pointed at this one.
    expect(menuOf(host)).not.toBeNull();
    // `appState` is global; read it back through the row's own marker.
    expect(
      host.querySelector('[data-block-id="blk-a"]')?.getAttribute("data-selected"),
    ).toBe("true");
  });

  it("leaves the webview's menu alone when no row is offered", () => {
    // A host that answers neither callback — a backlink row, say.
    const host = mount(
      makeCb({ onCopyBlockRef: undefined, onCopyBlockEmbed: undefined }),
    );

    const e = rightClick(bulletOf(host));

    expect(menuOf(host)).toBeNull();
    expect(e.defaultPrevented).toBe(false);
  });

  it("does not disturb the bullet's left click", () => {
    const cb = makeCb();
    const host = mount(cb);
    bulletOf(host).click();

    // The bullet's click is already overloaded (zoom on a plain
    // bullet, TODO otherwise); the menu rides a different button.
    expect(cb.onFocusBlock).toHaveBeenCalledWith("blk-a");
    expect(menuOf(host)).toBeNull();
  });
});

describe("the rows", () => {
  it("'Copy block ref' copies `((blk-x))`", async () => {
    vi.mocked(copyBlockRef).mockResolvedValue("((blk-abc123))");
    const host = mount(makeCb());
    rightClick(bulletOf(host));

    rowOf(host, "copyBlockRef").click();
    await Promise.resolve();
    await Promise.resolve();

    expect(copyBlockRef).toHaveBeenCalledWith("blk-a");
    expect(copyBlockEmbed).not.toHaveBeenCalled();
    expect(writeText).toHaveBeenCalledWith("((blk-abc123))");
  });

  it("'Copy block embed' copies `!((blk-x))`, from the other command", async () => {
    vi.mocked(copyBlockEmbed).mockResolvedValue("!((blk-abc123))");
    const host = mount(makeCb());
    rightClick(bulletOf(host));

    rowOf(host, "copyBlockEmbed").click();
    await Promise.resolve();
    await Promise.resolve();

    expect(copyBlockEmbed).toHaveBeenCalledWith("blk-a");
    expect(copyBlockRef).not.toHaveBeenCalled();
    expect(writeText).toHaveBeenCalledWith("!((blk-abc123))");
  });

  it("are labelled exactly as mobile's sheet labels them", () => {
    // One action, one label, on every client — the rule written at the
    // top of `outl-mobile/src/components/Journal.context-actions.ts`.
    // Mobile ships "Copy block ref"; a desktop "Copy reference" makes
    // one action look like two.
    const host = mount(makeCb());
    rightClick(bulletOf(host));

    expect(rowOf(host, "copyBlockRef").textContent).toBe("Copy block ref");
    expect(rowOf(host, "copyBlockEmbed").textContent).toBe("Copy block embed");
  });

  it("closes after a row fires", () => {
    vi.mocked(copyBlockRef).mockResolvedValue("((blk-abc123))");
    const host = mount(makeCb());
    rightClick(bulletOf(host));

    rowOf(host, "copyBlockRef").click();

    expect(menuOf(host)).toBeNull();
  });
});

describe("dismissal", () => {
  it("closes on Escape, without also reaching the global dispatcher", () => {
    const host = mount(makeCb());
    rightClick(bulletOf(host));

    // `installShortcuts` listens on `window` in the bubble phase and
    // reads Escape as "leave this mode"; closing a menu must not also
    // drop the user out of Visual.
    const onWindowKey = vi.fn();
    window.addEventListener("keydown", onWindowKey);
    document.body.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
    );
    window.removeEventListener("keydown", onWindowKey);

    expect(menuOf(host)).toBeNull();
    expect(onWindowKey).not.toHaveBeenCalled();
  });

  it("closes on a click away", () => {
    const host = mount(makeCb());
    rightClick(bulletOf(host));

    document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));

    expect(menuOf(host)).toBeNull();
  });

  it("stays open for a press inside itself", () => {
    const host = mount(makeCb());
    rightClick(bulletOf(host));

    rowOf(host, "copyBlockRef").dispatchEvent(
      new MouseEvent("mousedown", { bubbles: true }),
    );

    // A press that is the start of picking a row must not dismiss the
    // row it is aimed at.
    expect(menuOf(host)).not.toBeNull();
  });

  it("stops listening once closed", () => {
    const host = mount(makeCb());
    rightClick(bulletOf(host));
    document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));

    const onWindowKey = vi.fn();
    window.addEventListener("keydown", onWindowKey);
    document.body.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }),
    );
    window.removeEventListener("keydown", onWindowKey);

    // A closed menu that still swallowed Escape would make the chord
    // dead for the rest of the session, on any row ever right-clicked.
    expect(onWindowKey).toHaveBeenCalledTimes(1);
  });
});

describe("keyboard reach", () => {
  it("walks the rows with the arrow keys", () => {
    const host = mount(makeCb());
    rightClick(bulletOf(host));
    const menu = menuOf(host);
    if (!menu) throw new Error("menu not open");

    const down = () =>
      menu.dispatchEvent(
        new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }),
      );

    // Nothing is focused on open, on purpose: the menu can be summoned
    // while another block is being edited, and stealing focus there
    // commits a draft the user did not finish.
    expect(document.activeElement).not.toBe(rowOf(host, "copyBlockRef"));
    down();
    expect(document.activeElement).toBe(rowOf(host, "copyBlockRef"));
    down();
    expect(document.activeElement).toBe(rowOf(host, "copyBlockEmbed"));
    down();
    expect(document.activeElement).toBe(rowOf(host, "copyBlockRef"));
  });
});
