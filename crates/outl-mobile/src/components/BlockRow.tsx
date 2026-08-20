import { For, JSX, Show, onCleanup, onMount } from "solid-js";
import type { BlockNode } from "@outl/shared/api/types";
import {
  BlockProperties,
  MarkdownInline,
  QuoteWrap,
  isBlockQuoted,
  splitQuote,
  stripQuoteFromTokens,
} from "@outl/shared/markdown";
import { HighlightedCode, detectFence } from "@outl/shared/highlight";

function detectFenceText(text: string) {
  return detectFence(text);
}
import {
  autoClosePair,
  autoDeletePair,
  autoPairBracket,
} from "@outl/shared/autocomplete";
import {
  choosePasteRoute,
  utf16OffsetToCharOffset,
} from "@outl/shared/paste";
import { rawTextWithTodo } from "@outl/shared/outline";
import { transformerFor } from "@outl/shared/plugins/transformer-registry";
import { createLongPress } from "../lib/long-press";
import { haptic } from "../lib/haptics";
import { parkCaret } from "../lib/textarea";
import { PluginFence } from "./PluginFence";
import { SwipeRow } from "./SwipeRow";

interface BlockRowProps {
  block: BlockNode;
  depth: number;
  editingId: string | null;
  /**
   * Lazy accessor for the draft signal. Receiving a getter instead
   * of `string` means only the block that's *actually* in edit
   * subscribes to `draft()` changes — the other 199 rows in a
   * 200-block outline ignore each keystroke. Without this, typing
   * one character re-runs a reactive effect in every BlockRow.
   */
  draftText: () => string;
  onStartEdit: (id: string, initialText: string) => void;
  onDraftChange: (text: string) => void;
  onCommitEdit: () => void;
  onToggleTodo: (id: string) => void;
  onDelete: (id: string) => void;
  onIndent: (id: string) => void;
  onOutdent: (id: string) => void;
  onCreateAfter: (id: string) => void;
  /**
   * Zoom in on this block (Roam/Workflowy focus). Tapping a plain
   * bullet dot makes the block the outline root. Optional so a client
   * that doesn't support zoom can omit it (the dot falls back to its
   * mark-as-TODO tap).
   */
  onFocusBlock?: (id: string) => void;
  /** Open the block's contextual menu (long-press gesture). */
  onContextMenu: (id: string) => void;
  /**
   * Flip the block's collapsed flag. Implemented by the parent so
   * the persistence path (Tauri → sidecar) is shared with every
   * other block-mutating action and the parent can re-render with
   * the fresh `PageView`.
   */
  onToggleCollapse: (id: string, next: boolean) => void;
  onRefClick?: (target: string) => void;
  onTagClick?: (tag: string) => void;
  /** External `[label](url)` link tap — opens in the system browser. */
  onLinkClick?: (href: string) => void;
  /** Commit a `key:: value` property edit; empty value clears it. */
  onSetProperty?: (blockId: string, key: string, value: string) => void;
  onTextareaMount?: (el: HTMLTextAreaElement) => void;
  /**
   * Called when the user pastes outline-shaped markdown into this
   * block's textarea. The frontend has already detected via
   * `looksLikeOutline` that the clipboard payload deserves a
   * full-on tree conversion; the parent wires this up to the Tauri
   * `paste_markdown_at` command and refreshes the page on resolve.
   * `caret` is a `char` offset into the host block's text.
   */
  onPasteMarkdown?: (blockId: string, caret: number, text: string) => void;
}

/**
 * One row of the outline. Handles read-mode (rendered markdown) and
 * edit-mode (textarea), TODO checkbox, swipe-to-delete, long-press
 * to toggle TODO, and renders children recursively.
 */
const INDENT_PX = 22;

export function BlockRow(props: BlockRowProps): JSX.Element {
  const isEditing = () => props.editingId === props.block.id;
  const hasChildren = () => props.block.children.length > 0;

  return (
    <div class="relative">
      <SwipeRow
        leftActionLabel="Delete"
        onSwipeLeft={() => {
          haptic("warning");
          props.onDelete(props.block.id);
        }}
      >
        <BlockBody
          block={props.block}
          editing={isEditing()}
          draftText={props.draftText}
          depth={props.depth}
          hasChildren={hasChildren()}
          onToggleCollapse={() => {
            haptic("light");
            props.onToggleCollapse(props.block.id, !props.block.collapsed);
          }}
          onStartEdit={() =>
            props.onStartEdit(props.block.id, rawTextWithTodo(props.block))
          }
          onDraftChange={props.onDraftChange}
          onCommitEdit={props.onCommitEdit}
          onToggleTodo={() => {
            haptic("light");
            props.onToggleTodo(props.block.id);
          }}
          onFocusBlock={
            props.onFocusBlock
              ? () => props.onFocusBlock!(props.block.id)
              : undefined
          }
          onLongPress={() => {
            // iOS standard: long-press opens the contextual menu for
            // the block. Toggling TODO stays available as a discrete
            // action inside the menu (and as a tap on the checkbox
            // when the block already has TODO/DONE state).
            haptic("medium");
            props.onContextMenu(props.block.id);
          }}
          onRefClick={props.onRefClick}
          onTagClick={props.onTagClick}
          onLinkClick={props.onLinkClick}
          onTextareaMount={props.onTextareaMount}
          onPasteMarkdown={
            props.onPasteMarkdown
              ? (caret, text) =>
                  props.onPasteMarkdown!(props.block.id, caret, text)
              : undefined
          }
        />
      </SwipeRow>

      <Show when={hasChildren() && !props.block.collapsed}>
        <div class="relative">
          {/* Guide line connecting parent bullet to children */}
          <span
            aria-hidden="true"
            class="absolute top-0 bottom-0 w-px bg-(--color-ios-divider)/35 dark:bg-(--color-iosd-divider)/30"
            style={{ left: `${16 + props.depth * INDENT_PX + 5}px` }}
          />
          <For each={props.block.children}>
            {(child) => (
              <BlockRow
                block={child}
                depth={props.depth + 1}
                editingId={props.editingId}
                draftText={props.draftText}
                onStartEdit={props.onStartEdit}
                onDraftChange={props.onDraftChange}
                onCommitEdit={props.onCommitEdit}
                onToggleTodo={props.onToggleTodo}
                onDelete={props.onDelete}
                onIndent={props.onIndent}
                onOutdent={props.onOutdent}
                onCreateAfter={props.onCreateAfter}
                onToggleCollapse={props.onToggleCollapse}
                onFocusBlock={props.onFocusBlock}
                onContextMenu={props.onContextMenu}
                onRefClick={props.onRefClick}
                onTagClick={props.onTagClick}
                onLinkClick={props.onLinkClick}
                onTextareaMount={props.onTextareaMount}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}

function BlockBody(props: {
  block: BlockNode;
  editing: boolean;
  /** Lazy accessor — only read inside the edit-mode branch so non-
   *  editing rows don't subscribe to `draft()`. */
  draftText: () => string;
  depth: number;
  /** `true` when the block has at least one child. Drives the
   *  triangle marker (▶/▼). */
  hasChildren: boolean;
  /** Flip `block.collapsed`. No-op visually when `hasChildren` is
   *  `false`; the tap target hides itself in that case. */
  onToggleCollapse: () => void;
  onStartEdit: () => void;
  onDraftChange: (text: string) => void;
  onCommitEdit: () => void;
  onToggleTodo: () => void;
  /** Zoom in on this block. When set, a tap on the plain bullet dot
   *  focuses instead of marking TODO. `undefined` keeps the dot's
   *  mark-as-TODO tap. */
  onFocusBlock?: () => void;
  onLongPress: () => void;
  onRefClick?: (target: string) => void;
  onTagClick?: (tag: string) => void;
  /** External `[label](url)` link tap — opens in the system browser. */
  onLinkClick?: (href: string) => void;
  /** Commit a `key:: value` property edit; empty value clears it. */
  onSetProperty?: (blockId: string, key: string, value: string) => void;
  onTextareaMount?: (el: HTMLTextAreaElement) => void;
  /** See `BlockRowProps.onPasteMarkdown`. The parent has already
   *  injected `blockId`; this variant gets the caret + text. */
  onPasteMarkdown?: (caret: number, text: string) => void;
}) {
  /**
   * True when the gesture started inside an interactive child — a
   * page ref (`[[…]]`), tag (`#…`), inline code, link, or any
   * `button`/`[role=button]`. Those need to handle their own taps;
   * we bail before arming the long-press timer or starting an edit
   * so the user actually navigates to the ref instead of opening
   * the textarea on top of it.
   */
  function pressedInteractive(e: PointerEvent): boolean {
    const target = e.target as HTMLElement | null;
    return !!target?.closest("a,button,[role='button'],code,textarea,input");
  }

  // Hold timing / drift tolerance are shared with the page title's
  // gesture — one recogniser, so the two never feel different.
  const longPress = createLongPress({
    onLongPress: () => props.onLongPress(),
  });
  let skipGesture = false;

  function onPointerDown(e: PointerEvent) {
    skipGesture = props.editing || pressedInteractive(e);
    if (skipGesture) return;
    longPress.onPointerDown(e);
  }
  function onPointerMove(e: PointerEvent) {
    if (skipGesture) return;
    longPress.onPointerMove(e);
  }
  function onPointerUp() {
    longPress.onPointerUp();
  }

  function onClick(e: MouseEvent) {
    if (longPress.consumedClick()) {
      return;
    }
    // A tap that landed inside an interactive child has already been
    // handled by that child (`stopPropagation` on the ref/tag span,
    // the checkbox button, etc). Don't fall through into "start
    // edit" — that's how tap-on-ref kept opening the editor.
    if ((e.target as HTMLElement | null)?.closest(
      "a,button,[role='button'],code,textarea,input",
    )) {
      return;
    }
    if (!props.editing) props.onStartEdit();
  }

  // A row can unmount mid-hold (a sync reload repaints the outline);
  // the timer would fire onto a component that no longer exists.
  onCleanup(() => longPress.cancel());

  const padLeft = () => 16 + props.depth * INDENT_PX;

  return (
    <div
      // `data-block-id` lets the drag-and-drop drop handler resolve which
      // block a dropped file landed on (`document.elementFromPoint` →
      // `.closest("[data-block-id]")`). This div wraps only the block's own
      // row (bullet + body); children render in a sibling container, so a
      // point over a child resolves to the nearest child's id, not this one.
      data-block-id={props.block.id}
      class="group flex items-start gap-2.5 py-[5px] pr-4"
      style={{ "padding-left": `${padLeft()}px` }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerUp}
      onClick={onClick}
    >
      <CollapseTriangle
        visible={props.hasChildren}
        collapsed={props.block.collapsed}
        onToggle={() => {
          props.onToggleCollapse();
        }}
      />

      {(() => {
        // Keep the list marker outside the quote chrome so a quote is
        // visually the body of a normal outline block. The
        // CollapseTriangle also stays outside the chrome.
        const bullet = (
          <BulletOrCheckbox
            todo={props.editing ? null : props.block.todo}
            onToggle={() => {
              props.onToggleTodo();
            }}
            onFocus={props.onFocusBlock}
          />
        );
        const bodyDiv = (
          <div class="min-w-0 flex-1">
            <Show
              when={props.editing}
              fallback={(() => {
                const fence = detectFenceText(props.block.text);
                if (fence) {
                  const plainFence = () => (
                    <HighlightedCode
                      language={fence.language}
                      code={fence.body || " "}
                    />
                  );
                  // A plugin content-transformer may claim this fence's
                  // language: render its descriptor inline (text/markdown
                  // or a sandboxed iframe for `rich`), falling back to the
                  // plain highlighted code while it loads or if it declines.
                  const transformer = transformerFor(fence.language);
                  if (transformer) {
                    return (
                      <PluginFence
                        blockId={props.block.id}
                        transformer={transformer}
                        body={fence.body}
                        fallback={plainFence}
                      />
                    );
                  }
                  return plainFence();
                }
                // Chrome lives on the wrapper one level up; here we
                // only strip `> ` from the first Plain token so the
                // marker doesn't double-paint.
                const split = splitQuote(props.block.text);
                const tokens = split.quoted
                  ? stripQuoteFromTokens(props.block.tokens)
                  : props.block.tokens;
                const bodyLength = split.quoted
                  ? split.body.length
                  : props.block.text.length;
                return (
                  <p
                    class="break-words text-[17px] leading-[1.42]"
                    classList={{
                      "text-(--color-ios-text-tertiary) line-through dark:text-(--color-iosd-text-tertiary)":
                        props.block.todo === "DONE",
                    }}
                  >
                    <Show
                      when={bodyLength > 0}
                      fallback={
                        <span class="italic text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)">
                          Empty block
                        </span>
                      }
                    >
                      <MarkdownInline
                        tokens={tokens}
                        onRefClick={props.onRefClick}
                        onTagClick={props.onTagClick}
                        onLinkClick={props.onLinkClick}
                      />
                    </Show>
                    {/* `remind::` was invisible here: the long-press
                        menu wrote the rule and the block looked
                        untouched. Tapping a chip reopens it. */}
                    <BlockProperties
                      properties={props.block.properties}
                      onCommit={(key, value) =>
                        props.onSetProperty?.(props.block.id, key, value)
                      }
                      chipClass="rounded-full bg-(--color-ios-divider)/40 px-2 py-0.5 text-[11px] text-(--color-ios-text-secondary) dark:bg-(--color-iosd-divider)/40 dark:text-(--color-iosd-text-secondary)"
                      inputClass="rounded-full border border-(--color-ios-accent)/50 bg-(--color-ios-card) px-2 py-0.5 text-[11px] text-(--color-ios-text) outline-none dark:bg-(--color-iosd-card) dark:text-(--color-iosd-text)"
                    />
                  </p>
                );
              })()}
            >
          <EditableTextarea
            value={props.draftText()}
            onInput={props.onDraftChange}
            onBlur={props.onCommitEdit}
            onMount={props.onTextareaMount}
            onPaste={props.onPasteMarkdown}
          />
        </Show>
      </div>
        );
        // Tailwind classes are passed as **string literals** so the
        // JIT discovers them at build time — the shared `<QuoteWrap />`
        // just composes the conditional `class=` attribute.
        return (
          <>
            {bullet}
            <QuoteWrap
              quoted={isBlockQuoted(props.block.text)}
              baseClass="flex min-w-0 flex-1"
              chromeClass="rounded-r-md border-l-2 border-(--color-ios-text-secondary)/40 bg-(--color-ios-text-secondary)/[0.05] pl-2 dark:border-(--color-iosd-text-secondary)/40 dark:bg-(--color-iosd-text-secondary)/[0.07]"
            >
              {bodyDiv}
            </QuoteWrap>
          </>
        );
      })()}
    </div>
  );
}

function CollapseTriangle(props: {
  visible: boolean;
  collapsed: boolean;
  onToggle: () => void;
}) {
  // Always reserve the slot — even on leaves — so the bullet column
  // stays put regardless of whether a sibling has children. Width
  // matches the bullet (`w-[26px]`).
  return (
    <Show
      when={props.visible}
      fallback={<span aria-hidden="true" class="w-[18px] shrink-0" />}
    >
      <button
        type="button"
        aria-label={props.collapsed ? "Expand block" : "Collapse block"}
        aria-expanded={!props.collapsed}
        onClick={(e) => {
          e.stopPropagation();
          props.onToggle();
        }}
        class="relative z-10 -my-1.5 flex h-[30px] w-[18px] shrink-0 items-center justify-center text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)"
      >
        <span aria-hidden="true" class="text-[10px] leading-none">
          {props.collapsed ? "▶" : "▼"}
        </span>
      </button>
    </Show>
  );
}

function BulletOrCheckbox(props: {
  todo: BlockNode["todo"];
  onToggle: () => void;
  /** Zoom into this block (Roam/Workflowy). When set, a tap on the
   *  plain bullet dot focuses; TODO toggling stays in the long-press
   *  menu. When `undefined`, the dot marks TODO as before. */
  onFocus?: () => void;
}) {
  // Apple HIG: minimum tap target is 44×44. We hit ~36×30 here so we
  // stay visually compact in dense outlines but no longer demand
  // pixel-perfect taps. The visual dot/checkbox keeps its old size
  // — the surrounding `<button>` is what grows.
  return (
    <Show
      when={props.todo !== null}
      fallback={
        <button
          type="button"
          aria-label={props.onFocus ? "Zoom into block" : "Mark as TODO"}
          onClick={(e) => {
            e.stopPropagation();
            // Bullet dot zooms when the client supports it; otherwise it
            // keeps its legacy mark-as-TODO behaviour. TODO stays
            // reachable via the long-press context menu regardless.
            if (props.onFocus) props.onFocus();
            else props.onToggle();
          }}
          class="group/bullet relative z-10 -my-1.5 -ml-2 flex h-[30px] w-[26px] shrink-0 items-center justify-center"
        >
          <span
            aria-hidden="true"
            class="h-1.5 w-1.5 rounded-full bg-(--color-ios-text-tertiary) transition-transform group-active/bullet:scale-150 dark:bg-(--color-iosd-text-tertiary)"
          />
        </button>
      }
    >
      <button
        type="button"
        aria-label={
          props.todo === "DONE"
            ? "Clear task state"
            : props.todo === "DOING"
              ? "Mark as done"
              : "Mark as doing"
        }
        onClick={(e) => {
          e.stopPropagation();
          props.onToggle();
        }}
        class="relative z-10 -my-1.5 -ml-1 flex h-[30px] w-[30px] shrink-0 items-center justify-center"
      >
        <span
          class="flex h-[20px] w-[20px] items-center justify-center rounded-full border-[1.5px] transition-colors"
          classList={{
            "border-(--color-ios-accent) bg-(--color-ios-accent) dark:border-(--color-iosd-accent) dark:bg-(--color-iosd-accent)":
              props.todo === "DONE",
            // DOING keeps the accent ring and gets a small accent dot
            // instead of the full fill, so a started task reads as
            // "open, underway" at a glance rather than as finished.
            "border-(--color-ios-accent) bg-transparent dark:border-(--color-iosd-accent)":
              props.todo === "DOING",
            "border-(--color-ios-text-secondary) bg-transparent dark:border-(--color-iosd-text-secondary)":
              props.todo === "TODO",
          }}
        >
          <Show when={props.todo === "DOING"}>
            <span
              aria-hidden="true"
              class="h-[9px] w-[9px] rounded-full bg-(--color-ios-accent) dark:bg-(--color-iosd-accent)"
            />
          </Show>
          <Show when={props.todo === "DONE"}>
            <svg
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="white"
              stroke-width="3.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M5 12l4 4 10-10" />
            </svg>
          </Show>
        </span>
      </button>
    </Show>
  );
}

function EditableTextarea(props: {
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
        // Backspace inside an empty `[[]]` or `(())` deletes the
        // whole pair so the user doesn't have to mash four times.
        // We do this in keydown (not input) so we can `preventDefault`
        // before the browser eats the lone `[` to the left of caret.
        if (e.key !== "Backspace") return;
        const ta = e.currentTarget;
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
