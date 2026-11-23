/**
 * Where a hold opens the context menu, and where it must not.
 *
 * `pressedInteractive` refuses to arm the gesture inside anything that
 * handles its own touch — a link, a tag, inline `code`, the textarea.
 * The bullet was caught by that net for months because it is a
 * `<button>`, which made the one control a finger reaches for when it
 * means "act on this block" the one control the menu was unreachable
 * from. These pin the exemption and its blast radius: the bullet arms,
 * everything else in the net still does not, and a tap on the bullet
 * still does what it always did.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { BlockNode } from "@outl/shared/api/types";

import { BlockBody } from "./BlockBody";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  dispose = undefined;
  document.body.innerHTML = "";
});

/** A block carrying a `[docs](url)` link, so the row holds one target
 *  the gesture must keep refusing. */
function block(): BlockNode {
  return {
    id: "blk-1",
    text: "see docs",
    todo: null,
    tokens: [
      { kind: "plain", value: "see " },
      { kind: "link", value: "docs", href: "https://example.com" },
    ],
    collapsed: false,
    properties: [],
    children: [],
  } as unknown as BlockNode;
}

function mount(over: { selectionMode?: boolean; todo?: BlockNode["todo"] } = {}) {
  const onLongPress = vi.fn();
  const onFocusBlock = vi.fn();
  const onToggleTodo = vi.fn();
  const host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => (
      <BlockBody
        block={{ ...block(), todo: over.todo ?? null }}
        editing={false}
        draftText={() => "see docs"}
        depth={0}
        hasChildren={false}
        selectionMode={over.selectionMode}
        onToggleCollapse={() => {}}
        onStartEdit={() => {}}
        onDraftChange={() => {}}
        onCommitEdit={() => {}}
        onToggleTodo={onToggleTodo}
        onFocusBlock={onFocusBlock}
        onLinkClick={() => {}}
        onLongPress={onLongPress}
      />
    ),
    host,
  );
  return { host, onLongPress, onFocusBlock, onToggleTodo };
}

const bullet = (host: HTMLElement) =>
  host.querySelector<HTMLElement>("[data-block-bullet]")!;

/** The `[docs](url)` span — a `[role='button']`, which is how
 *  `MarkdownInline` renders a link that has a tap handler. */
const link = (host: HTMLElement) =>
  host.querySelector<HTMLElement>("[role='button']")!;

function down(el: HTMLElement) {
  el.dispatchEvent(
    new PointerEvent("pointerdown", { bubbles: true, clientX: 1, clientY: 1 }),
  );
}

function up(el: HTMLElement) {
  el.dispatchEvent(new PointerEvent("pointerup", { bubbles: true }));
}

/** Past `HOLD_MS` (450), with the margin the page switcher's suite uses. */
const hold = () => new Promise((r) => setTimeout(r, 560));

describe("a hold that starts on the bullet", () => {
  it("opens the context menu", async () => {
    const { host, onLongPress } = mount();

    down(bullet(host));
    await hold();
    up(bullet(host));

    expect(onLongPress).toHaveBeenCalledTimes(1);
  });

  it("opens it from the checkbox too, since that is the same control", async () => {
    const { host, onLongPress } = mount({ todo: "TODO" });

    down(bullet(host));
    await hold();
    up(bullet(host));

    expect(onLongPress).toHaveBeenCalledTimes(1);
  });

  it("does not also zoom on the way out", async () => {
    const { host, onFocusBlock, onLongPress } = mount();

    down(bullet(host));
    await hold();
    up(bullet(host));
    // The click the OS fires after the hold. The bullet
    // `stopPropagation`s it, so the row's own guard never sees it —
    // without `bulletTap` reading `consumedClick()`, this zoomed.
    bullet(host).click();

    expect(onLongPress).toHaveBeenCalledTimes(1);
    expect(onFocusBlock).not.toHaveBeenCalled();
  });

  it("is suppressed while a range selection is active", async () => {
    const { host, onLongPress } = mount({ selectionMode: true });

    down(bullet(host));
    await hold();
    up(bullet(host));

    expect(onLongPress).not.toHaveBeenCalled();
  });
});

describe("a plain tap on the bullet is unchanged", () => {
  it("zooms", () => {
    const { host, onFocusBlock } = mount();

    down(bullet(host));
    up(bullet(host));
    bullet(host).click();

    expect(onFocusBlock).toHaveBeenCalledTimes(1);
  });

  it("cycles task state on a checkbox", () => {
    const { host, onToggleTodo } = mount({ todo: "TODO" });

    down(bullet(host));
    up(bullet(host));
    bullet(host).click();

    expect(onToggleTodo).toHaveBeenCalledTimes(1);
  });
});

describe("a hold elsewhere still opens nothing", () => {
  it("refuses a link", async () => {
    const { host, onLongPress } = mount();

    down(link(host));
    await hold();
    up(link(host));

    expect(onLongPress).not.toHaveBeenCalled();
  });
});
