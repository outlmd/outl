import { For, JSX, Show } from "solid-js";
import type { BlockNode } from "@outl/shared/api/types";
import { rawTextWithTodo } from "@outl/shared/outline";
import { haptic } from "../lib/haptics";
import { BlockBody, rowPadLeft } from "./BlockBody";
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
  /**
   * RFC 0254 phase 3 — touch-native multi-block selection. `true`
   * while a range selection is active anywhere on the page (entered
   * from a block's long-press menu, "Select blocks"). It changes what
   * a tap on *any* row does, not just this one — every row's tap
   * becomes "extend the range to here" instead of its normal action —
   * so `selectionMode` has to reach every recursive `<BlockRow />`,
   * not just the one the user long-pressed.
   */
  selectionMode?: boolean;
  /** Membership set for the active range (`visualRangeSet` from
   *  `@outl/shared/outline`, memoised once per render by `Journal`) —
   *  a row answers "am I selected?" with `selectionSet?.has(id)` in
   *  O(1) rather than recomputing the range itself. Same convention
   *  the desktop's `<BlockRow />` uses for its own Visual highlight —
   *  see the Vim-parity section of `outl-desktop/CLAUDE.md`, which
   *  memoises the range as a `Set<id>` at the parent for the same
   *  reason. `null`/`undefined` while `selectionMode` is false. */
  selectionSet?: Set<string> | null;
  /** Fired instead of every other tap handler on this row while
   *  `selectionMode` is true — grows or shrinks the range to include
   *  this block. Long-press and swipe are suppressed in the same
   *  state (see `BlockBody`'s `disabled` wiring below), so a batch op
   *  in progress can't be interrupted by a stray context menu or an
   *  accidental swipe-delete pulling one block out of the range. */
  onSelectTap?: (id: string) => void;
}

/**
 * One row of the outline, and every row under it.
 *
 * What stays here is the recursion, the swipe-to-delete affordance and
 * the haptics: this is the component that knows a row has an id and
 * children, so it is where `(id)` is bound onto each callback before
 * handing it down. The row's own content — read-mode markdown,
 * edit-mode textarea, bullet, fold triangle, touch gestures — is
 * `<BlockBody />`.
 *
 * The haptics live on this side on purpose. A buzz is feedback for a
 * *decision* (this delete happened, this TODO flipped), and the
 * decision is made here; `BlockBody` only reports the gesture.
 */
export function BlockRow(props: BlockRowProps): JSX.Element {
  const isEditing = () => props.editingId === props.block.id;
  const hasChildren = () => props.block.children.length > 0;

  const selectionMode = () => props.selectionMode ?? false;
  const selected = () => props.selectionSet?.has(props.block.id) ?? false;

  return (
    <div class="relative">
      <SwipeRow
        leftActionLabel="Delete"
        disabled={selectionMode()}
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
          selectionMode={selectionMode()}
          selected={selected()}
          onSelectTap={
            props.onSelectTap
              ? () => props.onSelectTap!(props.block.id)
              : undefined
          }
          onToggleCollapse={() => {
            haptic("light");
            props.onToggleCollapse(props.block.id, !props.block.collapsed);
          }}
          onStartEdit={() =>
            props.onStartEdit(props.block.id, rawTextWithTodo(props.block))
          }
          onDraftChange={props.onDraftChange}
          onCommitEdit={props.onCommitEdit}
          onSetProperty={props.onSetProperty}
          onDeleteEmpty={() => {
            // Backspace on an already-empty block (RFC 0254 phase 4b,
            // `DeleteEmptyBlock`) — same destructive haptic and the
            // same confirm-if-it-has-children guard as swipe-to-delete
            // above, since an empty block can still carry children.
            haptic("warning");
            props.onDelete(props.block.id);
          }}
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
            class="absolute top-0 bottom-0 w-px bg-(--color-outl-border)/35"
            style={{ left: `${rowPadLeft(props.depth) + 5}px` }}
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
                onPasteMarkdown={props.onPasteMarkdown}
                onSetProperty={props.onSetProperty}
                selectionMode={props.selectionMode}
                selectionSet={props.selectionSet}
                onSelectTap={props.onSelectTap}
              />
            )}
          </For>
        </div>
      </Show>
    </div>
  );
}
