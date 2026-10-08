import { onCleanup, onMount } from "solid-js";
import {
  autoClosePair,
  autoDeletePair,
  autoPairBracket,
} from "@outl/shared/autocomplete";
import { detectFence } from "@outl/shared/highlight";
import {
  choosePasteRoute,
  utf16OffsetToCharOffset,
} from "@outl/shared/paste";
import { parkCaret } from "../lib/textarea";

/**
 * Edit mode for one block: the textarea and nothing else.
 *
 * Everything in here is about surviving the iOS soft keyboard —
 * `beforeinput` instead of `keydown` because WKWebView does not emit
 * reliable per-character key events, and `parkCaret` twice around every
 * programmatic `value =` because WKWebView resets the caret to the end.
 * That is why it is its own file: the rules are about the platform's
 * text input, not about what an outline row looks like.
 */
export function EditableTextarea(props: {
  value: string;
  onInput: (v: string) => void;
  onBlur: () => void;
  onMount?: (el: HTMLTextAreaElement) => void;
  /**
   * Called when the user pastes outline-shaped markdown. Receives
   * the caret position (in chars) and the verbatim clipboard text.
   * The parent is responsible for `preventDefault` semantics on the
   * paste event — we already do that here when this is set.
   */
  onPaste?: (caret: number, text: string) => void;
  /** See `BlockBody`'s doc — fires on Backspace when the textarea is
   *  already empty, instead of the pair-collapse logic below. */
  onDeleteEmpty?: () => void;
}) {
  let ref!: HTMLTextAreaElement;
  let resizeRaf = 0;

  // Reading `ref.scrollHeight` after writing `ref.style.height` forces
  // a synchronous layout. Doing that on every keystroke makes typing
  // feel sluggish on long pages — coalescing into a single
  // requestAnimationFrame keeps the work to once per frame.
  function autoResize() {
    if (!ref) return;
    if (resizeRaf) return;
    resizeRaf = window.requestAnimationFrame(() => {
      resizeRaf = 0;
      if (!ref) return;
      ref.style.height = "auto";
      ref.style.height = `${ref.scrollHeight}px`;
    });
  }

  onCleanup(() => {
    if (resizeRaf) window.cancelAnimationFrame(resizeRaf);
  });

  onMount(() => {
    autoResize();
    ref.focus();
    // Place cursor at end.
    const len = ref.value.length;
    ref.setSelectionRange(len, len);
    props.onMount?.(ref);
  });

  return (
    <textarea
      ref={ref}
      class="block w-full resize-none border-0 bg-transparent p-0 text-[17px] leading-snug outline-none"
      rows="1"
      value={props.value}
      // Keep iOS QuickType (word prediction + autocorrect) ON — it's
      // the suggestion bar the user actually types with. We used to
      // set `autocorrect="off"` (which also hides that bar) purely to
      // stop iOS Smart Punctuation from silently rewriting `--` → `–`,
      // `...` → `…`, `"foo"` → `“foo”` — disastrous for a markdown
      // outliner where code and CLI snippets are syntax-sensitive.
      // That substitution is now killed natively and precisely in
      // `OutlSwizzle` (smartQuotes/smartDashes/smartInsertDelete forced
      // to `.no` on the private WKContentView), so we get the
      // prediction bar back without the punctuation corruption.
      // `autocapitalize` stays off so typing `const` isn't title-cased
      // to `Const`.
      autocapitalize="off"
      onKeyDown={(e) => {
        if (e.key !== "Backspace") return;
        const ta = e.currentTarget;
        // Backspace on an already-empty block deletes the block
        // itself (`DeleteEmptyBlock`, RFC 0254 phase 4b) — mirrors the
        // desktop's `draft().length === 0` check. Checked first: an
        // empty textarea has no pair to collapse, so this and the
        // pair-collapse branch below never both apply.
        if (ta.value.length === 0 && props.onDeleteEmpty) {
          e.preventDefault();
          props.onDeleteEmpty();
          return;
        }
        // Backspace inside an empty `[[]]` or `(())` deletes the
        // whole pair so the user doesn't have to mash four times.
        // We do this in keydown (not input) so we can `preventDefault`
        // before the browser eats the lone `[` to the left of caret.
        if (ta.selectionStart !== ta.selectionEnd) return; // user is deleting a selection
        const caret = ta.selectionStart ?? 0;
        const completion = autoDeletePair(ta.value, caret);
        if (!completion) return;
        e.preventDefault();
        // `ta.value = …` resets the caret to the end of the text in
        // iOS WKWebView. `parkCaret` (called twice — once before and
        // once after `props.onInput` triggers Solid's `value=`
        // re-binding) keeps the caret where we asked.
        ta.value = completion.value;
        parkCaret(ta, completion.caret);
        props.onInput(completion.value);
        parkCaret(ta, completion.caret);
        autoResize();
      }}
      onBeforeInput={(e) => {
        // Auto-pair `(` / `[` / `{` and step over auto-inserted
        // closers (issue #21) — same Insert-mode behaviour as the
        // TUI. `beforeinput` (not keydown) because iOS soft
        // keyboards don't emit reliable per-character key events;
        // `insertText` with a single-char `data` is the one signal
        // that survives every input method.
        if (e.inputType !== "insertText" || e.isComposing) return;
        const ta = e.currentTarget;
        if (ta.selectionStart !== ta.selectionEnd) return; // typing over a selection
        const caret = ta.selectionStart ?? 0;
        const completion = autoPairBracket(ta.value, caret, e.data ?? "");
        if (!completion) return;
        e.preventDefault();
        // Same caret-reset trap as Backspace above — park twice,
        // around the Solid `value=` re-binding.
        ta.value = completion.value;
        parkCaret(ta, completion.caret);
        props.onInput(completion.value);
        parkCaret(ta, completion.caret);
        autoResize();
      }}
      onInput={(e) => {
        const ta = e.currentTarget;
        const caret = ta.selectionStart ?? ta.value.length;
        const completion = autoClosePair(ta.value, caret);
        if (completion) {
          // Same caret-reset trap as Backspace above. The user just
          // typed the second `[` (or `(`) and we appended the
          // matching closer; without parkCaret the cursor lands at
          // the end (`[[]]_`) instead of the middle (`[[_]]`).
          ta.value = completion.value;
          parkCaret(ta, completion.caret);
          props.onInput(completion.value);
          parkCaret(ta, completion.caret);
        } else {
          props.onInput(ta.value);
        }
        autoResize();
      }}
      onPaste={(e) => {
        // External-clipboard paste, with formatting. `choosePasteRoute`
        // (shared with desktop) decides between rich (text/html → markdown
        // so a Slack/Docs/Notion paste keeps its **bold** + lists),
        // structured (plain outline / multi-paragraph the backend splits),
        // or native (a trivial word / URL stays on the browser splice).
        if (!props.onPaste) return;
        // Inside a fenced code block the whole block is one raw ```lang…```
        // string. Converting a multi-line / outline clipboard would split
        // the fence into sibling blocks and strand the closing ``` on its
        // own line. Let the browser splice the text in literally (newlines
        // preserved), exactly like typing it — `onInput` keeps the draft
        // in sync. (Mirror of the desktop BlockRow guard.)
        if (detectFence(e.currentTarget.value)) return;
        const decision = choosePasteRoute(
          e.clipboardData?.getData("text/html") ?? "",
          e.clipboardData?.getData("text/plain") ?? "",
        );
        if (decision.route === "native") return;
        e.preventDefault();
        // `selectionStart` is a UTF-16 code unit offset; the Rust
        // backend wants a codepoint count. Conversion is a no-op
        // for BMP text but matters when the host block contains
        // emoji or other supplementary-plane characters.
        const ta = e.currentTarget;
        const caret = utf16OffsetToCharOffset(ta.value, ta.selectionStart ?? 0);
        props.onPaste(caret, decision.text);
      }}
      onBlur={props.onBlur}
    />
  );
}
