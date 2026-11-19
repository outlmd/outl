/**
 * The textarea's input handlers vs. an IME (issue #399).
 *
 * A Chinese / Japanese / Korean keyboard types into *marked* text, and
 * two platform facts make that hostile to pair completion: WebKit
 * reports `selectionStart` at the **start** of the marked run while the
 * `input` event fires, and assigning a different string to `ta.value`
 * commits that run as plain text.
 */
import { render } from "solid-js/web";
import { afterEach, describe, expect, it, vi } from "vitest";

import { EditableTextarea } from "./BlockTextarea";

let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  document.body.innerHTML = "";
});

function mount(value: string) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  const onInput = vi.fn();
  dispose = render(
    () => (
      <EditableTextarea value={value} onInput={onInput} onBlur={() => {}} />
    ),
    host,
  );
  const ta = host.querySelector("textarea")!;
  return { ta, onInput };
}

/** One `input` event, as the platform delivers it. The handler reads
 *  only these two fields plus the textarea, never `data`. */
function input(
  ta: HTMLTextAreaElement,
  init: { inputType: string; isComposing: boolean },
) {
  ta.dispatchEvent(new InputEvent("input", { bubbles: true, ...init }));
}

describe("EditableTextarea with an IME", () => {
  it("leaves the block alone while the keyboard composes inside `[[…]]`", () => {
    // `[[|]]` — the user typed `[[`, the pair closed, then they
    // switched to the Pinyin keyboard.
    const { ta, onInput } = mount("[[]]");
    let marked = "";
    for (const next of ["n", "ni", "nih", "niha", "nihao"]) {
      // The keyboard replaces the marked run in place...
      ta.value = ta.value.slice(0, 2) + next + ta.value.slice(2 + marked.length);
      // ...and WebKit reports the caret at its start.
      ta.setSelectionRange(2, 2);
      input(ta, { inputType: "insertCompositionText", isComposing: true });
      marked = next;
    }
    expect(ta.value).toBe("[[nihao]]");
    expect(onInput).toHaveBeenLastCalledWith("[[nihao]]");
  });

  it("leaves it alone on the event that ends the composition", () => {
    // Picking 你好 fires one last `insertCompositionText`, which some
    // WebKit builds report with `isComposing` already false — still
    // caret-at-the-start, so the `inputType` alone has to carry it.
    const { ta } = mount("[[]]");
    ta.value = "[[你好]]";
    ta.setSelectionRange(2, 2);
    input(ta, { inputType: "insertCompositionText", isComposing: false });
    expect(ta.value).toBe("[[你好]]");
  });

  it("still closes an open pair on an ordinary keystroke", () => {
    // The guard above must not cost the ASCII path its closer: a
    // keyboard that lands `[[` without a per-character `beforeinput`
    // (where `autoPairBracket` normally does this) still gets `[[|]]`.
    const { ta, onInput } = mount("[[");
    ta.value = "[[";
    ta.setSelectionRange(2, 2);
    input(ta, { inputType: "insertText", isComposing: false });
    expect(ta.value).toBe("[[]]");
    expect(onInput).toHaveBeenLastCalledWith("[[]]");
    expect(ta.selectionStart).toBe(2);
  });

  it("does not collapse the pair on Backspace mid-composition", () => {
    // Why the Backspace handler needs no composition guard of its own:
    // with marked text between them the `[[` and the `]]` are no longer
    // adjacent, so `autoDeletePair` finds nothing to collapse and the
    // keystroke reaches the keyboard, which drops one pinyin letter.
    const { ta } = mount("[[]]");
    ta.value = "[[n]]";
    ta.setSelectionRange(2, 2);
    const event = new KeyboardEvent("keydown", {
      key: "Backspace",
      bubbles: true,
      cancelable: true,
      isComposing: true,
    });
    ta.dispatchEvent(event);
    expect(event.defaultPrevented).toBe(false);
    expect(ta.value).toBe("[[n]]");
  });
});

/**
 * Cross-block caret nav, from a hardware keyboard (an iPad, or an
 * Android phone with one attached — the soft keyboard has no arrows).
 *
 * This is the one `keydown` branch in the file, and the contract that
 * matters is *when the key is swallowed*: `preventDefault` only on a
 * step the parent actually took, so at the outline's edge the webview's
 * own caret motion still happens.
 */
describe("EditableTextarea arrow keys", () => {
  function mountWithNav(value: string, taken: boolean) {
    const host = document.createElement("div");
    document.body.appendChild(host);
    const onCaretExit = vi.fn().mockReturnValue(taken);
    dispose = render(
      () => (
        <EditableTextarea
          value={value}
          onInput={() => {}}
          onBlur={() => {}}
          onCaretExit={onCaretExit}
        />
      ),
      host,
    );
    const ta = host.querySelector("textarea")!;
    return { ta, onCaretExit };
  }

  function arrow(
    ta: HTMLTextAreaElement,
    key: string,
    init: KeyboardEventInit = {},
  ): KeyboardEvent {
    const e = new KeyboardEvent("keydown", {
      key,
      bubbles: true,
      cancelable: true,
      ...init,
    });
    ta.dispatchEvent(e);
    return e;
  }

  it("asks the parent, and swallows the key, at the block's edges", () => {
    const { ta, onCaretExit } = mountWithNav("ship it", true);

    ta.setSelectionRange(0, 0);
    const left = arrow(ta, "ArrowLeft");
    expect(onCaretExit).toHaveBeenLastCalledWith({
      to: "previous",
      landing: "end",
    });
    expect(left.defaultPrevented).toBe(true);

    ta.setSelectionRange(7, 7);
    const right = arrow(ta, "ArrowRight");
    expect(onCaretExit).toHaveBeenLastCalledWith({
      to: "next",
      landing: "start",
    });
    expect(right.defaultPrevented).toBe(true);
  });

  it("carries the column out on ↑ / ↓", () => {
    const { ta, onCaretExit } = mountWithNav("ship it", true);
    ta.setSelectionRange(4, 4);
    arrow(ta, "ArrowUp");
    expect(onCaretExit).toHaveBeenLastCalledWith({
      to: "previous",
      landing: { column: 4 },
    });
    arrow(ta, "ArrowDown");
    expect(onCaretExit).toHaveBeenLastCalledWith({
      to: "next",
      landing: { column: 4 },
    });
  });

  it("does not ask while the caret still has room inside the block", () => {
    const { ta, onCaretExit } = mountWithNav("ship it", true);
    ta.setSelectionRange(3, 3);
    const e = arrow(ta, "ArrowLeft");
    expect(onCaretExit).not.toHaveBeenCalled();
    expect(e.defaultPrevented).toBe(false);
  });

  it("leaves the key to the webview when the parent did not take the step", () => {
    // The outline's edge. Swallowing it anyway would make `←` in the
    // page's first block a key that does nothing at all.
    const { ta, onCaretExit } = mountWithNav("ship it", false);
    ta.setSelectionRange(0, 0);
    const e = arrow(ta, "ArrowLeft");
    expect(onCaretExit).toHaveBeenCalledTimes(1);
    expect(e.defaultPrevented).toBe(false);
  });

  it("leaves a modified arrow alone — those chords mean something else", () => {
    const { ta, onCaretExit } = mountWithNav("ship it", true);
    ta.setSelectionRange(0, 0);
    for (const init of [
      { shiftKey: true },
      { altKey: true },
      { metaKey: true },
      { ctrlKey: true },
    ]) {
      expect(arrow(ta, "ArrowLeft", init).defaultPrevented).toBe(false);
    }
    expect(onCaretExit).not.toHaveBeenCalled();
  });

  it("is inert with no parent handler, so a client can opt out", () => {
    const { ta } = mount("ship it");
    ta.setSelectionRange(0, 0);
    const e = new KeyboardEvent("keydown", {
      key: "ArrowLeft",
      bubbles: true,
      cancelable: true,
    });
    ta.dispatchEvent(e);
    expect(e.defaultPrevented).toBe(false);
  });
});
