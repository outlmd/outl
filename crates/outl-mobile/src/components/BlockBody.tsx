import { Show, onCleanup } from "solid-js";
import type { BlockNode } from "@outl/shared/api/types";
import {
  BlockProperties,
  MarkdownInline,
  MarkdownTable,
  QuoteWrap,
  isBlockQuoted,
  splitQuote,
  stripQuoteFromTokens,
} from "@outl/shared/markdown";
import { HighlightedCode, detectFence } from "@outl/shared/highlight";
import { transformerFor } from "@outl/shared/plugins/transformer-registry";
import { createLongPress } from "../lib/long-press";
import { BulletOrCheckbox, CollapseTriangle } from "./BlockMarkers";
import { EditableTextarea } from "./BlockTextarea";
import { PluginFence } from "./PluginFence";

function detectFenceText(text: string) {
  return detectFence(text);
}

/** Horizontal step per outline level. */
export const INDENT_PX = 22;

/**
 * Left padding of a row at `depth`. `BlockRow` positions its
 * parent→children guide line against the same number, which is why
 * this is a function and not two copies of `16 + depth * INDENT_PX`
 * in two files.
 */
export function rowPadLeft(depth: number): number {
  return 16 + depth * INDENT_PX;
}

/**
 * One row's own content: the gesture layer plus the read / edit
 * branch. The recursion, the swipe affordance and the haptics live in
 * `BlockRow`; what is here is "what does this row show, and what does
 * a touch on it mean".
 *
 * The gesture rules are the load-bearing part. Three of them answer
 * the same question — *who already handled this touch?* — and they do
 * not collapse into one: a press that starts on a ref must not arm the
 * long-press timer (`pressedInteractive`), a click that a child
 * already consumed must not fall through to "start editing"
 * (`onClick`), and while a range selection is active every touch on
 * the row means "extend the range to here" regardless of what it
 * landed on.
 */
export function BlockBody(props: {
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
  /** Backspace pressed while the textarea is already empty
   *  (`DeleteEmptyBlock`, RFC 0254 phase 4b) — the caller decides
   *  whether that's an immediate delete or a confirm prompt. */
  onDeleteEmpty?: () => void;
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
  /** RFC 0254 phase 3 — see `BlockRowProps.selectionMode`. */
  selectionMode?: boolean;
  /** Is *this* block inside the active range? The parent already
   *  resolved membership via `selectionSet.has(id)`; `BlockBody`
   *  never sees the set itself. */
  selected?: boolean;
  /** Fired for every tap on this row while `selectionMode` is true,
   *  in place of `onStartEdit` / the bullet / the checkbox / the
   *  collapse triangle — a tap anywhere on a row means "extend the
   *  range to here" while selecting, full stop. */
  onSelectTap?: () => void;
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
  // `disabled` suppresses the whole gesture while a range selection
  // is active: opening the context menu mid-batch-op would let the
  // user fire a single-block action (delete, indent, …) that only
  // touches the long-pressed row while the toolbar implies it acts
  // on the whole range — a correctness trap, not just a UX wrinkle.
  const longPress = createLongPress({
    onLongPress: () => props.onLongPress(),
    disabled: () => props.selectionMode ?? false,
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
    // While a selection is active, every tap on the row body (or any
    // interactive span whose own handler we've turned off below — see
    // the `selectionMode ? undefined : …` wiring on `<MarkdownInline>`)
    // means "extend the range to here", not "start editing" or
    // "follow this ref". The bullet / checkbox / collapse triangle
    // have their own `onSelectTap` override at their call sites below,
    // since those buttons `stopPropagation` unconditionally and would
    // never let this handler see the click at all.
    if (props.selectionMode) {
      props.onSelectTap?.();
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

  const padLeft = () => rowPadLeft(props.depth);

  return (
    <div
      // `data-block-id` lets the drag-and-drop drop handler resolve which
      // block a dropped file landed on (`document.elementFromPoint` →
      // `.closest("[data-block-id]")`). This div wraps only the block's own
      // row (bullet + body); children render in a sibling container, so a
      // point over a child resolves to the nearest child's id, not this one.
      data-block-id={props.block.id}
      class="group flex items-start gap-2.5 rounded-md py-[5px] pr-4 transition-colors"
      classList={{
        // Unmistakable, matches the desktop's 18%-opacity Visual
        // highlight (same accent, same idea) so a user who moves
        // between clients recognises the state on sight.
        "bg-(--color-outl-accent)/[0.16]":
          (props.selectionMode ?? false) && (props.selected ?? false),
      }}
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
          // While selecting, every tap on this row — including the
          // fold triangle — extends the range instead of folding.
          // These two buttons `stopPropagation` unconditionally, so
          // the override has to live here rather than in the shared
          // `onClick` above, which would never see the event.
          if (props.selectionMode) {
            props.onSelectTap?.();
            return;
          }
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
              if (props.selectionMode) {
                props.onSelectTap?.();
                return;
              }
              props.onToggleTodo();
            }}
            // In selection mode the bullet always extends the range —
            // routed through `onFocus` so `<BulletOrCheckbox>` takes
            // that branch regardless of whether zoom (`onFocusBlock`)
            // is wired for this row.
            onFocus={
              props.selectionMode
                ? () => props.onSelectTap?.()
                : props.onFocusBlock
            }
          />
        );
        const bodyDiv = (
          <div class="min-w-0 flex-1">
            <Show
              when={props.editing}
              fallback={(() => {
                // Both renders ask this: the shared components only
                // `stopPropagation` a tap when the handler is set, so
                // withholding it while selecting is what turns a tap
                // into "extend the range" instead of "follow this ref".
                const tap = (h?: (v: string) => void) =>
                  props.selectionMode ? undefined : h;
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
                // No `onEdit` on purpose: the tap bubbles to this
                // row's handler (focus, or extend the range), exactly
                // like a tap on prose.
                if (props.block.table) {
                  return (
                    <MarkdownTable
                      table={props.block.table}
                      onRefClick={tap(props.onRefClick)}
                      onTagClick={tap(props.onTagClick)}
                      onLinkClick={tap(props.onLinkClick)}
                    />
                  );
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
                      "text-(--color-outl-fg-dimmer) line-through":
                        props.block.todo === "DONE",
                    }}
                  >
                    <Show
                      when={bodyLength > 0}
                      fallback={
                        <span class="italic text-(--color-outl-fg-dimmer)">
                          Empty block
                        </span>
                      }
                    >
                      <MarkdownInline
                        tokens={tokens}
                        onRefClick={tap(props.onRefClick)}
                        onTagClick={tap(props.onTagClick)}
                        onLinkClick={tap(props.onLinkClick)}
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
                      chipClass="rounded-full bg-(--color-outl-border)/40 px-2 py-0.5 text-[11px] text-(--color-outl-fg-dim)"
                      inputClass="rounded-full border border-(--color-outl-accent)/50 bg-(--color-outl-bg-elev) px-2 py-0.5 text-[11px] text-(--color-outl-fg) outline-none"
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
                onDeleteEmpty={props.onDeleteEmpty}
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
              chromeClass="rounded-r-md border-l-2 border-(--color-outl-fg-dim)/40 bg-(--color-outl-fg-dim)/[0.05] pl-2"
            >
              {bodyDiv}
            </QuoteWrap>
          </>
        );
      })()}
    </div>
  );
}
