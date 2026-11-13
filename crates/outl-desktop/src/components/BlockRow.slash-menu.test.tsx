/**
 * The inline `/command` menu, driven through a real `<BlockRow />` in
 * edit mode: type `/` into the block's textarea and the popup must
 * list something.
 *
 * It is pinned here rather than in `SuggestPopups.test.tsx` because
 * that file pins the *rendering* of a list it is handed, and the defect
 * this covers lived upstream of it — between the keystroke and the
 * list. Each link in that chain fails the same way from a screenshot.
 */
import { render } from "solid-js/web";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";
import { BlockRow, type BlockCallbacks } from "./BlockRow";
import { setAppState } from "../lib/store";
import { installShortcuts } from "../lib/shortcuts";
import type { Binding } from "../lib/api";

// The dispatcher publishes the native half of the slash menu, so the
// catalog it reads is stubbed down to one row we can assert on.
vi.mock("../lib/api", async (importOriginal) => {
  const real = await importOriginal<typeof import("../lib/api")>();
  const bindings: Binding[] = [
    {
      chord: [{ mods: real.MOD_META, key: { kind: "Char", value: "j" } }],
      mode: "global",
      action: { kind: "OpenToday" },
      description: "Open today's journal",
    },
  ] as Binding[];
  return {
    ...real,
    listShortcutBindings: vi.fn(async () => bindings),
    listActionSupport: vi.fn(async () => []),
    pluginKeybindings: vi.fn(async () => []),
  };
});

vi.mock("@outl/shared/api/commands", () => ({
  openRef: vi.fn(),
  pluginList: vi.fn().mockResolvedValue([
    { plugin_id: "demo", command_id: "stats", title: "Workspace stats" },
  ]),
  searchBlocks: vi.fn().mockResolvedValue([]),
  searchEmojis: vi.fn().mockResolvedValue([]),
  searchPages: vi.fn().mockResolvedValue([]),
  searchPersons: vi.fn().mockResolvedValue([]),
  listTemplates: vi.fn().mockResolvedValue([]),
}));
vi.mock("@outl/shared/plugins/transformer-registry", () => ({
  runTransform: vi.fn(),
  transformerFor: vi.fn(() => null),
}));
vi.mock("@tauri-apps/plugin-clipboard-manager", () => ({
  readText: vi.fn(),
}));

let dispose: (() => void) | undefined;

function makeCb(): BlockCallbacks {
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
  };
}

function mountEditing(text: string): {
  host: HTMLElement;
  ta: HTMLTextAreaElement;
} {
  const block: BlockNode = {
    id: "blk-1",
    text,
    todo: null,
    tokens: [],
    collapsed: false,
    properties: [],
    children: [],
  };
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BlockRow
        block={block}
        depth={0}
        editingId={block.id}
        visualSet={null}
        cb={makeCb()}
      />
    ),
    host,
  );
  const ta = host.querySelector("textarea");
  if (!ta) throw new Error("textarea did not render in edit mode");
  return { host, ta };
}

/** Type `ch` at the caret the way the webview does: `beforeinput` first
 *  (the auto-pair hook's seam), then the value, then `input`. */
function typeChar(ta: HTMLTextAreaElement, ch: string) {
  const before = new InputEvent("beforeinput", {
    data: ch,
    inputType: "insertText",
    bubbles: true,
    cancelable: true,
  });
  ta.dispatchEvent(before);
  if (before.defaultPrevented) return;
  const at = ta.selectionStart ?? ta.value.length;
  ta.value = ta.value.slice(0, at) + ch + ta.value.slice(at);
  ta.setSelectionRange(at + ch.length, at + ch.length);
  ta.dispatchEvent(
    new InputEvent("input", { data: ch, inputType: "insertText", bubbles: true }),
  );
}

async function settle() {
  for (let i = 0; i < 5; i += 1) await Promise.resolve();
}

let uninstall: (() => void) | undefined;

beforeEach(async () => {
  uninstall = await installShortcuts({ OpenToday: vi.fn() });
});

afterEach(() => {
  uninstall?.();
  uninstall = undefined;
  dispose?.();
  dispose = undefined;
  setAppState("caretIntent", null);
  document.body.innerHTML = "";
});

function rowsOf(host: HTMLElement): string[] {
  return Array.from(host.querySelectorAll('[role="option"]')).map(
    (r) => r.textContent ?? "",
  );
}

describe("the inline /command menu", () => {
  it("lists commands when `/` is typed into an empty block", async () => {
    const { host, ta } = mountEditing("");
    ta.setSelectionRange(0, 0);

    typeChar(ta, "/");
    await settle();

    expect(rowsOf(host)).toContain("/statsWorkspace stats");
  });

  /**
   * The universe was `/upload` + templates + plugins, so on a workspace
   * with neither this popup opened holding one row while the TUI's `/`
   * listed 31 built-ins. The client's own actions are the native half.
   */
  it("lists the client's own actions, not just plugins and templates", async () => {
    const { host, ta } = mountEditing("");
    ta.setSelectionRange(0, 0);

    typeChar(ta, "/");
    await settle();

    expect(rowsOf(host)).toContain("/open-todayOpen today's journal");
  });

  it("runs a native row through the dispatcher's own handler", async () => {
    const openToday = vi.fn();
    uninstall?.();
    uninstall = await installShortcuts({ OpenToday: openToday });

    const { host, ta } = mountEditing("");
    ta.setSelectionRange(0, 0);
    for (const ch of "/open-today") typeChar(ta, ch);
    await settle();

    const row = Array.from(host.querySelectorAll('[role="option"] button')).find(
      (b) => b.textContent?.startsWith("/open-today"),
    );
    row?.dispatchEvent(
      new MouseEvent("mousedown", { bubbles: true, cancelable: true }),
    );
    await settle();

    expect(openToday).toHaveBeenCalled();
  });

  /**
   * The trigger mirrors the TUI's `detect_trigger`, which fires on any
   * word-initial `/`. It used to require the block's FIRST character,
   * so `nota /` opened nothing here while the same keystrokes opened
   * the menu in the terminal.
   */
  it("opens after a space, not only at the start of the block", async () => {
    const { host, ta } = mountEditing("nota");
    ta.setSelectionRange(4, 4);

    typeChar(ta, " ");
    typeChar(ta, "/");
    await settle();

    expect(rowsOf(host)).toContain("/statsWorkspace stats");
  });

  it("stays silent inside a path or a URL", async () => {
    const { host, ta } = mountEditing("http:/");
    ta.setSelectionRange(6, 6);

    typeChar(ta, "/");
    await settle();

    expect(rowsOf(host)).toEqual([]);
  });
});
