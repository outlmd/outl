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
