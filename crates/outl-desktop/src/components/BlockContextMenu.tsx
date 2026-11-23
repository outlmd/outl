import { For, Show, createEffect, onCleanup } from "solid-js";

/**
 * One row of the bullet's right-click menu.
 *
 * Mirrors the shape mobile's long-press sheet uses
 * (`BlockContextAction`), minus the two fields that only mean
 * something on a phone: there is no SF-Symbol glyph in a desktop
 * popover, and no keyboard to arm before the tap is deferred.
 */
export interface BlockMenuAction {
  id: string;
  label: string;
  /** Draws the row in the destructive colour. */
  destructive?: boolean;
  onSelect: () => void;
}

/**
 * The right-click menu on a block's bullet.
 *
 * It takes a **list of rows**, never a component per item: the user's
 * ask was a menu more options could be added to, so "Copy block
 * markdown" or "Remind me…" later is one entry in the array
 * `<BlockRow />` builds and nothing here changes.
 *
 * It is not `<SuggestPopup />` in disguise. That one is caret-bound —
 * it renders inside the block editor's wrapper, driven by what is
 * being typed — which is why it is private to `SuggestPopups.tsx`.
 * This one is pointer-bound and opens on a row nobody is editing.
 * What the two do share is the floating-surface styling, copied
 * literally so they read as the same material.
 *
 * Labels come from the vocabulary mobile's sheet already ships
 * ("Copy block ref", not "Copy reference"): one action, one label, on
 * every client.
 *
 * Positioning is the caller's. The menu is `absolute`, so it anchors
 * to whichever `relative` ancestor it is rendered inside — the outline
 * row, in `<BlockRow />`.
 */
export function BlockContextMenu(props: {
  open: boolean;
  actions: BlockMenuAction[];
  onClose: () => void;
}) {
  let list: HTMLUListElement | undefined;

  /**
   * Dismissal: a click anywhere else, or Escape.
   *
   * Both listeners sit on the **document, in the capture phase**,
   * because the menu can be open while focus is still in a textarea
   * or nowhere at all — a handler bound to the menu itself would only
   * ever see the events it does not need to act on.
   *
   * Escape also stops propagating. `installShortcuts` listens on
   * `window` in the bubble phase and reads Escape as "leave this
   * mode", and closing a menu must not also drop the user out of
   * Visual.
   */
  createEffect(() => {
    if (!props.open) return;
    const onPointerDown = (e: MouseEvent) => {
      if (e.target instanceof Node && list?.contains(e.target)) return;
      props.onClose();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      props.onClose();
    };
    document.addEventListener("mousedown", onPointerDown, true);
    document.addEventListener("keydown", onKeyDown, true);
    onCleanup(() => {
      document.removeEventListener("mousedown", onPointerDown, true);
      document.removeEventListener("keydown", onKeyDown, true);
    });
  });

  /** Arrow keys walk the rows once focus is inside. Nothing is focused
   *  on open, on purpose: the menu can be summoned while another block
   *  is being edited, and stealing focus there would blur the textarea
   *  and commit a draft the user did not finish. Tab reaches the rows. */
  function onListKeyDown(e: KeyboardEvent) {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const rows = [...(list?.querySelectorAll("button") ?? [])];
    if (rows.length === 0) return;
    e.preventDefault();
    const at = rows.indexOf(document.activeElement as HTMLButtonElement);
    const step = e.key === "ArrowDown" ? 1 : -1;
    const next = at === -1 ? 0 : (at + step + rows.length) % rows.length;
    rows[next].focus();
  }

  return (
    <Show when={props.open && props.actions.length > 0}>
      <ul
        ref={list}
        role="menu"
        aria-label="Block actions"
        onKeyDown={onListKeyDown}
        class="absolute top-full left-0 z-30 mt-1 min-w-44 rounded-pop border border-(--color-outl-border)/60 bg-(--color-outl-bg-elev) p-1 text-[13px] shadow-pop"
      >
        <For each={props.actions}>
          {(action) => (
            <li role="none">
              <button
                type="button"
                role="menuitem"
                data-menu-action={action.id}
                onClick={(e) => {
                  // The row below is `<BlockRow />`'s outer click
                  // handler, which would re-select the block the menu
                  // is already addressed to.
                  e.stopPropagation();
                  props.onClose();
                  action.onSelect();
                }}
                class={`flex w-full items-center rounded-md px-2 py-1.5 text-left hover:bg-(--color-outl-accent)/15 focus:bg-(--color-outl-accent)/15 focus:outline-none ${
                  action.destructive
                    ? "text-(--color-outl-destructive)"
                    : "text-(--color-outl-fg)"
                }`}
              >
                {action.label}
              </button>
            </li>
          )}
        </For>
      </ul>
    </Show>
  );
}
