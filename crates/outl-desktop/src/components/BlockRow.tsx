import { For, Show, createSignal } from "solid-js";

import type { BlockNode, TodoState } from "@outl/shared/api/types";
import { QuoteWrap, isBlockQuoted } from "@outl/shared/markdown";
import { rawTextWithTodo } from "@outl/shared/outline";

import { appState, setAppState } from "../lib/store";
import { BlockBody } from "./BlockBody";
import { BlockEditor } from "./BlockEditor";
import type { BlockCallbacks } from "./block-callbacks";

/** The row's contract with `<OutlineView />`. Lives in its own module
 *  (`./block-callbacks`) because `<BlockBody />` and `<BlockEditor />`
 *  need it too; re-exported here so existing imports keep resolving. */
export type { BlockCallbacks } from "./block-callbacks";

/**
 * One outline block. Renders read-only by default; flips to a
 * textarea editor when `editing === true`. Mouse and keyboard
 * interactions route through the shared `BlockCallbacks` so the
 * parent (`OutlineView`) owns the Tauri-side state mutations.
 *
 * What stays in this file is the **row chrome**: the selection /
 * Visual / pending-cut / drop-target states, the indent guides, the
 * fold chevron, the bullet-vs-checkbox gesture split, and the quote
 * wrapper. The two things inside that chrome are `<BlockBody />`
 * (read mode) and `<BlockEditor />` (edit mode), and the row never
 * looks inside either.
 */
export function BlockRow(props: {
  block: BlockNode;
  depth: number;
  editingId: string | null;
  /** Memoised Visual-range membership set built once in
   *  `<OutlineView />` (`createMemo` over outline + anchor + cursor +
   *  mode). `null` outside Visual mode. We pay one DFS for the whole
   *  outline; every row answers `Set.has(id)` in O(1). The previous
   *  shape called `isInVisualRange(...)` per row, which rebuilt
   *  `flattenVisible` from scratch each call — O(N²) on extension. */
  visualSet: Set<string> | null;
  cb: BlockCallbacks;
}) {
  const isEditing = () => props.editingId === props.block.id;
  // Draft mirrors the wire format (with TODO/DONE prefix). Same
  // shape the TUI buffer and the mobile editor use, so users can
  // type / erase the prefix to flip state.
  //
  // It lives here, not in `<BlockEditor />`, because its lifetime is
  // the **row**, not the edit session: the editor is mounted and
  // disposed every time the user enters and leaves Insert, and moving
  // the signal in there would silently re-seed it from `props.block`
  // on each entry. `startEdit` is the one place that re-seeds it.
  const [draft, setDraft] = createSignal<string>(rawTextWithTodo(props.block));

  /** Task state to render the bullet by. Edit mode is the
   *  **raw markdown view** — the `TODO `/`DOING `/`DONE ` prefix is
   *  literally visible in the textarea, so the bullet collapses to the
   *  neutral `•` to avoid showing the same state twice. Read mode is
   *  where the prefix is replaced by `▢` / `▨` / `▣`. */
  function effectiveTodo(): TodoState | null {
    return isEditing() ? null : props.block.todo;
  }

  /** Bullet glyph + click action — folds the task states and none into
   *  a single visual primitive (TUI parity, `▢` / `▨` / `▣` / `•`). */
  function bulletGlyph(): string {
    const t = effectiveTodo();
    if (t === "DONE") return "▣";
    if (t === "DOING") return "▨";
    if (t === "TODO") return "▢";
    return "•";
  }
  function bulletClass(): string {
    const t = effectiveTodo();
    if (t === "DONE") {
      return "text-(--color-outl-todo-done-fg)";
    }
    // DOING shares the open colour: it is unfinished work, and the
    // half-filled glyph already carries the distinction.
    if (t === "TODO" || t === "DOING") {
      return "text-(--color-outl-todo-open-fg)";
    }
    return "text-(--color-outl-fg-dimmer)";
  }
  /** Body styling — DONE blocks render dim + struck-through (TUI
   *  uses theme.todo_done_body which is fg_dimmer + CROSSED_OUT). */
  function bodyClass(): string {
    return props.block.todo === "DONE" ? "line-through opacity-60" : "";
  }

  const isSelected = () => appState.selectedBlockId === props.block.id;
  /** Vim Visual range covers this block. Mutually exclusive with
   *  `isSelected()` rendering-wise: when both are true we apply the
   *  Visual style so the user sees the contiguous band, not a single
   *  bright row at the cursor. */
  const isInVisual = () => props.visualSet?.has(props.block.id) ?? false;
  /** This block is armed for a cut (`Cmd+X` in view mode), waiting
   *  for the paste that will move it. Dim it so the user sees what's
   *  on the block clipboard until they paste or cancel with `Esc`. */
  const isPendingCut = () =>
    appState.blockClipboard?.kind === "cut" &&
    appState.blockClipboard.nodeId === props.block.id;
  /** An OS file drag is hovering this block — highlight it so the user
   *  sees where the dropped file's link will be inserted. */
  const isDropTarget = () => appState.dropTargetBlockId === props.block.id;
  const isInteractive = () => isEditing() || props.block.todo !== null;

  /** Outer row click — select without entering Insert. Lets the
   *  user mouse onto a block and then keyboard-nav from there
   *  (j/k), instead of clicking forcing an edit. The inner text
   *  `<div onClick>` still calls `onStartEdit` to enter edit mode,
   *  but the buttons (bullet, chevron) `stopPropagation()` so they
   *  don't both fire. */
  function selectRow(e: MouseEvent) {
    // The text-click handler stops propagation; this only fires
    // when the user clicked on row chrome (gutter, indent area).
    e.stopPropagation();
    setAppState("selectedBlockId", props.block.id);
  }

  return (
    <div>
      <div
        class={`outl-row group relative flex items-start rounded-sm py-[3px] pr-2 ${
          isPendingCut() ? "opacity-50 " : ""
        }${
          isDropTarget() ? "ring-2 ring-inset ring-(--color-outl-accent) " : ""
        }${
          isInVisual()
            ? "bg-(--color-outl-accent)/[0.18]"
            : isSelected()
              ? "bg-(--color-outl-accent)/[0.06]"
              : "hover:bg-(--color-outl-bg-elev)/30"
        }`}
        data-block-id={props.block.id}
        data-selected={isSelected() ? "true" : "false"}
        data-visual={isInVisual() ? "true" : "false"}
        data-editing={isEditing() ? "true" : "false"}
        data-drop-target={isDropTarget() ? "true" : "false"}
        onClick={selectRow}
      >
        {/* Vertical accent bar for the selected row — Bear-style
         * "this is where you are" indicator. Sits in the row's
         * gutter so it never reflows text. */}
        <Show when={isSelected()}>
          <span
            aria-hidden="true"
            class="absolute top-[4px] bottom-[4px] -left-[2px] w-[3px] rounded-full bg-(--color-outl-accent)"
          />
        </Show>

        {/*
         * Indent guides, one per ancestor level. Hairline at 20 %, and
         * deliberately **not** `.outl-row-chrome`: a guide is a column
         * cue, read as a vertical line down the whole page, so revealing
         * it per row makes it flicker under the pointer and never shows
         * the structure it exists to show. The chevron is per-row chrome
         * and does hide; these do not.
         */}
        <For each={Array.from({ length: props.depth })}>
          {() => (
            <span
              aria-hidden="true"
              class="ml-[10px] w-3 shrink-0 self-stretch border-l border-(--color-outl-border)/20"
            />
          )}
        </For>

        {/* Fold chevron. `.outl-row-chrome` owns its opacity: 0 at
         * rest, 1 on hover / focus-within / selected / visual / editing
         * (see `styles.css`). A chevron on every row at rest is noise;
         * `focus-within` is what keeps it reachable without a mouse. */}
        <button
          type="button"
          class={`outl-row-chrome ml-[6px] mt-[6px] min-w-[16px] select-none whitespace-nowrap text-left text-[9px] font-mono ${
            props.block.children.length === 0 ? "" : "cursor-pointer"
          } disabled:cursor-default`}
          disabled={props.block.children.length === 0}
          onClick={(e) => {
            e.stopPropagation();
            if (props.block.children.length === 0) return;
            void props.cb.onToggleCollapsed(
              props.block.id,
              !props.block.collapsed,
            );
          }}
          aria-label={props.block.collapsed ? "Expand" : "Collapse"}
        >
          {props.block.children.length > 0 ? (
            props.block.collapsed ? (
              <>
                <span>▶</span>
                <span class="ml-1 text-(--color-outl-fg-dimmer)">
                  {props.block.children.length}
                </span>
              </>
            ) : (
              "▼"
            )
          ) : (
            ""
          )}
        </button>

        {/* The list marker stays outside quote chrome so a quoted body
          * remains an ordinary outline block. */}
        {(() => {
          // Bullet gesture split (no collision):
          //   - TODO/DONE blocks render a checkbox marker (▢/▣) → click
          //     toggles the state (checkbox semantics win).
          //   - A neutral `•` bullet has no other job → click zooms into
          //     the block (Roam/Workflowy focus). When zoom isn't wired
          //     (`onFocusBlock` absent) it falls back to the TODO toggle,
          //     preserving the old "click a plain bullet to add TODO".
          const zoomableBullet = props.block.todo === null && !!props.cb.onFocusBlock;
          const bullet = (
            <button
              type="button"
              onClick={(e) => {
                e.stopPropagation();
                if (zoomableBullet) props.cb.onFocusBlock?.(props.block.id);
                else void props.cb.onToggleTodo(props.block.id);
              }}
              {...(isInteractive() ? { "data-todo": "true" } : {})}
              class={`mt-[5px] mr-2 w-3 shrink-0 cursor-pointer select-none text-center text-[13px] leading-none transition-opacity hover:opacity-70 ${bulletClass()}`}
              title={
                props.block.todo === "DONE"
                  ? "Click to uncheck"
                  : props.block.todo === "DOING"
                    ? "Click to mark done"
                    : props.block.todo === "TODO"
                      ? "Click to mark doing"
                      : zoomableBullet
                        ? "Click to zoom in"
                        : "Click to mark as TODO"
              }
              aria-label={
                props.block.todo === "DONE"
                  ? "Mark not done"
                  : props.block.todo === "DOING"
                    ? "Mark done"
                    : props.block.todo === "TODO"
                      ? "Mark doing"
                      : zoomableBullet
                        ? "Zoom in on block"
                        : "Mark as TODO"
              }
            >
              {bulletGlyph()}
            </button>
          );
          // One handler for every "edit this block" affordance (the
          // fence's Edit button, a tap on a table, a click on prose);
          // it was three copies of the same three lines.
          //
          // No explicit focus call: `<BlockEditor />` focuses itself on
          // mount, which is the microtask the old `focusTextarea()`
          // landed in anyway.
          const startEdit = () => {
            setDraft(rawTextWithTodo(props.block));
            props.cb.onStartEdit(props.block.id);
          };
          const body = (
            <div class={`min-w-0 flex-1 leading-snug ${bodyClass()}`}>
              <Show
                when={isEditing()}
                fallback={
                  <BlockBody
                    block={props.block}
                    cb={props.cb}
                    onStartEdit={startEdit}
                  />
                }
              >
                <BlockEditor
                  blockId={props.block.id}
                  text={draft}
                  setText={setDraft}
                  wireText={() => rawTextWithTodo(props.block)}
                  cb={props.cb}
                />
              </Show>
            </div>
          );
          // Tailwind classes are passed as **string literals** so the
          // JIT discovers them at build time — the shared
          // `<QuoteWrap />` just composes the conditional `class=`.
          return (
            <>
              {bullet}
              <QuoteWrap
                quoted={isBlockQuoted(props.block.text)}
                baseClass="flex min-w-0 flex-1"
                chromeClass="rounded-r-md border-l-2 border-(--color-outl-fg-dimmer)/50 bg-(--color-outl-fg-dimmer)/[0.06] pl-2"
              >
                {body}
              </QuoteWrap>
            </>
          );
        })()}
      </div>

      <Show when={!props.block.collapsed && props.block.children.length > 0}>
        <For each={props.block.children}>
          {(child) => (
            <BlockRow
              block={child}
              depth={props.depth + 1}
              editingId={props.editingId}
              visualSet={props.visualSet}
              cb={props.cb}
            />
          )}
        </For>
      </Show>
    </div>
  );
}
