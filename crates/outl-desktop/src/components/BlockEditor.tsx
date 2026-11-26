import { createEffect, onMount } from "solid-js";

import { autoDeletePair, autoPairBracket } from "@outl/shared/autocomplete";
import { caretExit, caretNavKey } from "@outl/shared/caret-nav";
import { detectFence } from "@outl/shared/highlight";
import {
  charOffsetToUtf16Offset,
  choosePasteRoute,
  type PasteEditorUpdate,
  utf16OffsetToCharOffset,
} from "@outl/shared/paste";

import { moveOnce, setEditorOwnsHold } from "../lib/move-gate";
import { readOsClipboard } from "../lib/os-clipboard";
import { insertMoveFor } from "../lib/shortcuts";
import { appState, setAppState } from "../lib/store";
import type { EditorCallbacks } from "./block-callbacks";
import { createBlockSuggest } from "./block-suggest";
import {
  BlockSuggestPopup,
  EmojiSuggestPopup,
  RefSuggestPopup,
  SlashCommandPopup,
} from "./SuggestPopups";

/**
 * Edit mode for one block: the textarea, its chords, and the four
 * inline autocomplete popups anchored to it.
 *
 * Mounted only while the row is being edited (it is the truthy branch
 * of `<BlockRow />`'s `<Show>`), so "on mount" and "the user entered
 * Insert on this block" are the same instant — which is why the focus
 * and the pending caret intent are applied in `onMount` rather than in
 * an effect watching an `editing` flag.
 *
 * The draft signal is **not** owned here. `<BlockRow />` holds it, so
 * it keeps the lifetime it has always had — one signal per row, not one
 * per edit session.
 */
export function BlockEditor(props: {
  blockId: string;
  /** The draft, mirroring the wire format (TODO/DONE prefix included)
   *  so the user can type or erase the prefix to flip state. */
  text: () => string;
  setText: (value: string) => void;
  /** The block's current wire text. What the draft is compared against
   *  to decide whether leaving edit mode is a real change or a ghost op
   *  (#213). */
  wireText: () => string;
  /**
   * Leave Insert without writing anything — Esc or blur on an unchanged
   * draft, where there is no op to emit and the row still has to flip
   * back to render mode (the "Esc didn't exit edit mode" bug).
   *
   * Optional because the outline's answer is the default, and it is the
   * **only** thing in here that names where the editing identity lives.
   * A backlink row's lives in `editingBacklink`, so a host editing one
   * passes its own.
   */
  onLeave?: () => void;
  cb: EditorCallbacks;
}) {
  let textareaRef: HTMLTextAreaElement | undefined;

  const suggest = createBlockSuggest({
    textarea: () => textareaRef,
    setText: (value) => props.setText(value),
    commit: () => commit(),
    runPluginCommand: (pluginId, commandId) =>
      props.cb.onRunPluginCommand(pluginId, commandId),
  });

  /**
   * Grow the textarea to fit its current content. Without this the
   * `rows={1}` textarea would clip multi-line blocks (code fences,
   * paragraphs typed with plain `Enter`). Cheap enough to call on
   * every draft change.
   */
  function autoSize() {
    const ta = textareaRef;
    if (!ta) return;
    ta.style.height = "auto";
    ta.style.height = `${ta.scrollHeight}px`;
  }

  // Re-run autoSize when the draft signal changes — Solid's
  // reactivity ties this to keystrokes the textarea fires.
  createEffect(() => {
    props.text();
    autoSize();
  });

  /*
   * Focus the textarea as soon as it exists — whether the user clicked
   * into the block or `<OutlineView />` set `editingId` programmatically
   * after `createBlock` (Cmd+Enter fires the parent to create a sibling,
   * and we want that sibling editable without a second click). The
   * microtask lets Solid finish committing the `<Show>` swap so
   * `textareaRef` is populated by the time we call focus().
   */
  onMount(() => {
    queueMicrotask(() => {
      textareaRef?.focus();
      autoSize();
      // Apply pending caret intent (set by `EnterInsertAtEnd`, or by
      // the Tab / Shift-Tab handler below carrying a codepoint offset
      // across the indent/outdent remount for #477). We do it here,
      // after focus, because the textarea ref is guaranteed populated
      // by this point: `<Show>` mounted it synchronously and the
      // microtask fence drained any pending Solid commits.
      //
      // `intent !== null`, not `intent &&`: a numeric offset of `0`
      // (caret at the very start) is falsy and would silently fall
      // through to "do nothing" under the truthy check.
      const intent = appState.caretIntent;
      if (intent !== null && textareaRef) {
        const length = textareaRef.value.length;
        const pos =
          intent === "end"
            ? length
            : intent === "start"
              ? 0
              : Math.min(charOffsetToUtf16Offset(textareaRef.value, intent), length);
        textareaRef.setSelectionRange(pos, pos);
        setAppState("caretIntent", null);
      }
    });
  });

  /** An arrow-key step's commit is in flight. */
  let caretStepping = false;

  async function commit() {
    // Draft is already wire format (prefix included if any). The
    // backend re-runs `split_todo` on it and re-projects the block
    // with the right `todo` state.
    const raw = props.text();
    const wire = props.wireText();
    if (raw !== wire) {
      // `onCommit` flips `editingBlockId` to null after the Tauri
      // round-trip, so the row will re-render in read-only mode.
      await props.cb.onCommit(props.blockId, raw);
      return;
    }
    // Unchanged text — still need to leave Insert and flip the row
    // back to render mode. Without this an Esc on an unmodified
    // block leaves the textarea visible with raw markdown showing
    // (`**bold**` instead of the rendered **bold**), which is the
    // "Esc didn't exit edit mode" bug.
    if (props.onLeave) props.onLeave();
    else setAppState("editingBlockId", null);
  }

  async function handleKeydown(e: KeyboardEvent) {
    // Cmd/Ctrl+Shift+V — paste WITHOUT formatting: read the clipboard
    // and splice it raw at the caret, no outline / paragraph conversion.
    // (Plain Cmd+V is the native paste event → `handlePaste`, which
    // routes structured content to the backend "with formatting".)
    if (
      (e.metaKey || e.ctrlKey) &&
      e.shiftKey &&
      (e.key === "v" || e.key === "V")
    ) {
      // A host that cannot route a paste to the workspace leaves the
      // keystroke alone rather than eating it — see `EditorCallbacks`.
      const route = props.cb.onPastePlain;
      if (!route) return;
      e.preventDefault();
      const ta = textareaRef;
      if (!ta) return;
      let clip: string;
      try {
        // `os-clipboard` owns the read (the plugin, never
        // `navigator.clipboard`, which pops a native "Paste" button in
        // the macOS WKWebview and inserts nothing). An empty clipboard
        // is said on the status line rather than read as a dead chord.
        clip = await readOsClipboard();
      } catch (err) {
        setAppState("lastError", err instanceof Error ? err.message : String(err));
        return;
      }
      const caretChars = utf16OffsetToCharOffset(ta.value, ta.selectionStart ?? 0);
      applyPaste(await route(props.blockId, caretChars, clip, ta.value));
      return;
    }
    // Move chord (catalog Insert rows). Above `suggest`: an open popup
    // would otherwise take the arrow, and the chord means "move the
    // block", not "pick the next item". `commit()` unmounts the popup
    // with the textarea.
    //
    // Only the three calls below run outside `moveOnce`: they decide
    // the keystroke's fate synchronously. Capturing the caret and
    // writing `caretIntent` run inside it, or a repeat the gate drops
    // would leave a stale intent for the next block that mounts.
    // `stopImmediatePropagation` for the reason the Tab branch gives:
    // `commit()` unmounts this textarea and the `window` dispatcher
    // would resolve the chord against Normal and move a second time.
    const move = insertMoveFor(e);
    if (move) {
      e.preventDefault();
      e.stopImmediatePropagation();
      if (!e.repeat) setEditorOwnsHold(true);
      // Read before the commit: a backlink row's editor unmounts with
      // it, and `props.blockId` is a `<Show>` accessor that throws once
      // its owner is gone.
      const id = props.blockId;
      await moveOnce(async () => {
        const ta = textareaRef;
        if (ta) {
          setAppState(
            "caretIntent",
            utf16OffsetToCharOffset(ta.value, ta.selectionStart ?? ta.value.length),
          );
        }
        await commit();
        if (move === "up") await props.cb.onMoveUp(id);
        else await props.cb.onMoveDown(id);
      });
      return;
    }
    // An open autocomplete popup owns the arrows / Enter / Tab / Esc
    // before any of the block chords below see them.
    if (suggest.handleKey(e)) {
      return;
    }
    if (e.key === "Escape") {
      e.preventDefault();
      await commit();
      return;
    }
    // An arrow key at the edge of the block walks into the neighbour
    // without leaving Insert — the outline reads as one continuous
    // document, which is what the TUI has shipped all along.
    // `@outl/shared/caret-nav` owns *when* (`caretNavKey` is also what
    // decides a modified arrow is a different chord); `./outline-caret-nav`
    // owns *where*.
    const navKey = caretNavKey(e);
    if (navKey && caretStepping) {
      // A held arrow repeats faster than the commit round-trip, and
      // this textarea stays mounted until it lands. Swallow the repeat
      // rather than committing the same draft a second time.
      e.preventDefault();
      e.stopImmediatePropagation();
      return;
    }
    if (navKey) {
      const ta = textareaRef;
      if (!ta) return;
      const exit = caretExit(
        navKey,
        ta.value,
        utf16OffsetToCharOffset(ta.value, ta.selectionStart ?? 0),
        utf16OffsetToCharOffset(ta.value, ta.selectionEnd ?? 0),
      );
      if (!exit) return; // the caret still has room inside this block
      // Resolved before anything is awaited: a `preventDefault` only
      // counts during the synchronous dispatch, and at the outline's
      // edges this keystroke belongs to the browser (`←` in the first
      // block must not drop the user out of Insert).
      const target = props.cb.resolveCaretExit(props.blockId, exit);
      if (!target) return;
      e.preventDefault();
      // Load-bearing, for the same reason as the Tab handler below:
      // `commit()` flips `editingBlockId` to `null`, unmounting this
      // textarea mid-dispatch, so the `window` listener would resolve
      // `ArrowUp` / `ArrowDown` against the catalog's Normal rows
      // (`SelectionUp` / `SelectionDown`) and move the cursor a second
      // time on one keystroke.
      e.stopImmediatePropagation();
      caretStepping = true;
      try {
        await commit();
      } finally {
        caretStepping = false;
      }
      props.cb.openCaretLanding(target);
      return;
    }
    // Plain `Enter` (no modifiers) → commit + create a sibling below.
    // TUI parity: `outl-tui/src/input/mod.rs` — "Plain Enter commits the
    // block and creates a sibling below." `Shift+Enter` (no Cmd/Ctrl) is
    // the soft break: it falls through to the textarea default and inserts
    // a literal `\n` for a multi-line block (issue #119).
    // No `stopImmediatePropagation` here (unlike `Cmd/Ctrl+Shift+Enter`
    // below): with a textarea focused the dispatcher is in Insert mode,
    // and the catalog has no Insert binding for a bare `Enter` (its only
    // `Enter` row is Normal → `OpenRefUnderCursor`), so the window
    // dispatcher no-ops — there is no double-fire to guard against.
    if (
      e.key === "Enter" &&
      !e.shiftKey &&
      !e.metaKey &&
      !e.ctrlKey &&
      !e.altKey
    ) {
      e.preventDefault();
      // Pass the caret so the backend splits the block there instead of
      // always appending an empty sibling (issue #184). `selectionStart`
      // is a UTF-16 offset; the backend expects codepoints.
      const caretChars = utf16OffsetToCharOffset(
        textareaRef?.value ?? props.text(),
        textareaRef?.selectionStart ?? props.text().length,
      );
      await props.cb.onEnter(props.blockId, props.text(), caretChars);
      return;
    }
    // `Cmd/Ctrl+Shift+Enter` is caret-position aware, handled here (not
    // via the global catalog) because only the textarea knows the caret:
    //   - caret at column 0  → create a sibling *before* this block
    //     (vim `O`).
    //   - caret anywhere past column 0 → commit + create a sibling
    //     *below* (`onEnter`).
    // `stopImmediatePropagation` is load-bearing: it stops the webview's
    // default Enter (a literal `\n`) *and* keeps the global `window`
    // shortcut dispatcher from also firing the catalog's
    // commit-and-continue on the same keystroke (the old "Cmd+Shift+Enter
    // breaks the line" double-fire bug). `Cmd+Enter` / `Cmd+T` are still
    // owned by the catalog.
    if (e.key === "Enter" && e.shiftKey && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      e.stopImmediatePropagation();
      const atStart =
        (textareaRef?.selectionStart ?? -1) === 0 &&
        (textareaRef?.selectionEnd ?? -1) === 0;
      if (atStart) await props.cb.onCreateBefore(props.blockId, props.text());
      // Past column 0: this chord means "sibling below", not "split
      // here" — pass the end offset so `split_block` yields an empty
      // sibling below (the pre-#184 behaviour) regardless of the caret.
      else
        await props.cb.onEnter(
          props.blockId,
          props.text(),
          props.text().length,
        );
      return;
    }
    if (e.key === "Tab") {
      e.preventDefault();
      // `stopImmediatePropagation` is load-bearing, for the same reason
      // as `Cmd/Ctrl+Shift+Enter` above: `keydown` is one of Solid's
      // delegated events, so this handler runs from a listener on
      // `document` — one node below the `window` listener
      // `installShortcuts` adds. `commit()` below flips `editingBlockId`
      // to `null`, which unmounts this textarea, so without this the
      // event reaches `window` with nothing focused, `detectMode()`
      // answers `normal`, and the catalog's Normal rows (`Tab` →
      // `IndentBlock`, `Shift+Tab` → `OutdentBlock`) fire a *second*
      // move on the same keystroke. The textarea owns this chord in
      // Insert mode; the dispatcher must not get a turn.
      e.stopImmediatePropagation();
      // Capture the caret before `commit()`: it flips `editingBlockId`
      // to `null` (directly, or via the `onCommit` round-trip), which
      // tears this component down. `onIndent` / `onOutdent` put the
      // moved block straight back into edit mode (#477), mounting a
      // fresh `<BlockEditor />` that reads this back out of
      // `caretIntent` once it lands, so the user's caret survives the
      // remount instead of resetting.
      const ta = textareaRef;
      const caretChars = ta
        ? utf16OffsetToCharOffset(ta.value, ta.selectionStart ?? ta.value.length)
        : null;
      if (caretChars !== null) setAppState("caretIntent", caretChars);
      await commit();
      if (e.shiftKey) await props.cb.onOutdent(props.blockId);
      else await props.cb.onIndent(props.blockId);
      return;
    }
    if (e.key === "Backspace" && props.text().length === 0) {
      e.preventDefault();
      await props.cb.onDeleteEmpty(props.blockId);
      return;
    }
    if (e.key === "Backspace") {
      const ta = textareaRef;
      if (ta) {
        const collapse = autoDeletePair(ta.value, ta.selectionStart ?? 0);
        if (collapse) {
          e.preventDefault();
          props.setText(collapse.value);
          ta.value = collapse.value;
          ta.setSelectionRange(collapse.caret, collapse.caret);
        }
      }
    }
    // Default: let the keystroke through; the bound input handler
    // updates the draft signal.
  }

  /**
   * Auto-pair `(` / `[` / `{` and step over auto-inserted closers
   * (issue #21) — same Insert-mode behaviour as the TUI. Typing the
   * second `[` lands as `[[|]]`, so the `[[` ref flow keeps working
   * without `autoClosePair` (the closer is never doubled).
   * `beforeinput` (not keydown) so layouts that reach brackets via
   * AltGr / Option dead keys are matched by the character actually
   * produced, never by the physical key.
   */
  function handleBeforeInput(e: InputEvent) {
    if (e.inputType !== "insertText" || e.isComposing) return;
    const ta = textareaRef;
    if (!ta) return;
    if (ta.selectionStart !== ta.selectionEnd) return; // typing over a selection
    const completion = autoPairBracket(
      ta.value,
      ta.selectionStart ?? 0,
      e.data ?? "",
    );
    if (!completion) return;
    e.preventDefault();
    props.setText(completion.value);
    ta.value = completion.value;
    ta.setSelectionRange(completion.caret, completion.caret);
    // Setting the value programmatically doesn't fire `onInput`, and
    // the caret may now sit inside a `[[…]]` — refresh the suggester.
    suggest.refresh();
  }

  async function handlePaste(e: ClipboardEvent) {
    const ta = textareaRef;
    if (!ta) return;
    // Inside a fenced code block the whole block is one raw ```lang…```
    // string. Converting a multi-line / outline clipboard "with
    // formatting" would split the fence into sibling blocks and strand
    // the closing ``` on its own line (the "paste jumps to the last
    // line" bug). Let the browser splice the text in literally (newlines
    // preserved), exactly like typing it — `onInput` keeps the draft in
    // sync. Cmd+Shift+V (paste without formatting) already splices raw.
    if (detectFence(ta.value)) return;
    // Same early return for a host that cannot route the paste: the
    // webview splices it literally and the text commits with the draft.
    const route = props.cb.onPasteMarkdown;
    if (!route) return;
    // "Paste with formatting" (Cmd+V). `choosePasteRoute` (shared with
    // mobile) decides between: rich (text/html converted to markdown so a
    // Slack/Docs/Notion paste keeps its **bold** + lists), structured
    // (plain outline / multi-paragraph the backend splits), or native (a
    // trivial word / URL stays on the browser splice). Cmd+Shift+V is the
    // separate "without formatting" path.
    const decision = choosePasteRoute(
      e.clipboardData?.getData("text/html") ?? "",
      e.clipboardData?.getData("text/plain") ?? "",
    );
    if (decision.route === "native") return;
    e.preventDefault();
    const caretChars = utf16OffsetToCharOffset(
      ta.value,
      ta.selectionStart ?? 0,
    );
    applyPaste(
      await route(props.blockId, caretChars, decision.text, ta.value),
    );
  }

  /**
   * Put the paste's result into the textarea the user is still typing
   * in.
   *
   * The draft is seeded once per row and only `startEdit` re-seeds it
   * (see `<BlockRow />`), so re-rendering the outline is not enough: a
   * caret paste rewrote this very block, and a draft still holding the
   * pre-paste string is what `commit()` writes back on the next Esc or
   * blur — the paste disappears from the block and from the `.md`, with
   * nothing said (issue #488).
   *
   * `ta.value` is written directly as well as through `setText`,
   * because the caret has to move in the same frame: Solid's
   * `value={props.text()}` binding lands in a later effect, and
   * `setSelectionRange` against the stale value would clamp the caret
   * to the old text's length.
   */
  function applyPaste(update: PasteEditorUpdate | null) {
    if (!update) return;
    const ta = textareaRef;
    props.setText(update.text);
    if (!ta) return;
    ta.value = update.text;
    ta.setSelectionRange(update.caret, update.caret);
    autoSize();
    // The caret may now sit inside a `[[…]]` the paste brought in.
    suggest.refresh();
  }

  return (
    <div class="relative">
      <textarea
        ref={textareaRef}
        value={props.text()}
        autofocus
        rows={1}
        spellcheck={false}
        data-block-id={props.blockId}
        class="w-full resize-none overflow-hidden bg-transparent text-current outline-none"
        onInput={(e) => {
          props.setText(e.currentTarget.value);
          suggest.refresh();
        }}
        onSelect={() => suggest.refresh()}
        onBlur={() => void commit()}
        onKeyDown={handleKeydown}
        onBeforeInput={handleBeforeInput}
        onPaste={handlePaste}
      />
      <RefSuggestPopup
        items={suggest.suggestions()}
        activeIndex={suggest.suggestIndex()}
        onHover={suggest.setSuggestIndex}
        onPick={suggest.acceptSuggestion}
      />
      <BlockSuggestPopup
        items={suggest.blockSuggestions()}
        activeIndex={suggest.blockIndex()}
        onHover={suggest.setBlockIndex}
        onPick={suggest.acceptBlockSuggestion}
      />
      <EmojiSuggestPopup
        items={suggest.emojiSuggestions()}
        activeIndex={suggest.emojiIndex()}
        onHover={suggest.setEmojiIndex}
        onPick={suggest.acceptEmojiSuggestion}
      />
      <SlashCommandPopup
        items={suggest.slashCommands()}
        activeIndex={suggest.slashIndex()}
        onHover={suggest.setSlashIndex}
        onPick={suggest.acceptSlashCommand}
      />
    </div>
  );
}
