import { For, JSX, Show } from "solid-js";
import { createSheetDrag } from "../lib/sheet-drag";
import { haptic } from "../lib/haptics";
import { armKeyboard } from "../lib/keyboard-keeper";

export interface BlockContextAction {
  id: string;
  label: string;
  /** SF Symbol-equivalent SVG path (24×24 viewBox). */
  iconPath: string;
  /** When `true`, label + icon turn destructive red. */
  destructive?: boolean;
  /**
   * `true` for an action that ends with the cursor in a text field.
   * The keyboard then has to be raised from inside the tap's own
   * user-gesture window, which is *before* `onSelect` is deferred
   * below — iOS ignores a `.focus()` that lands any later
   * (`lib/keyboard-keeper.ts`).
   */
  armsKeyboard?: boolean;
  /** Hidden entirely when this returns false. Lets callers
   *  conditionally suppress (e.g. "Move up" on the first block). */
  enabled?: () => boolean;
  onSelect: () => void;
}

interface BlockContextMenuProps {
  open: boolean;
  actions: BlockContextAction[];
  onClose: () => void;
}

/**
 * Bottom-sheet contextual menu shown after a long-press on a block.
 * Inspired by iOS's `UIContextMenu` and Bear's block menu — a stack
 * of `{icon, label}` rows on a blurred sheet, plus a "Cancel" pill
 * separated by a gap (classic iOS action sheet pattern).
 *
 * Drag-to-dismiss uses the same hook as `PageSwitcher` / `Calendar`
 * so the gesture feels identical across every sheet in the app.
 */
export function BlockContextMenu(props: BlockContextMenuProps): JSX.Element {
  const drag = createSheetDrag(() => props.onClose());

  function fire(action: BlockContextAction) {
    // Before anything is deferred: this is the last moment that still
    // counts as the user's tap, and the keyboard only comes up for a
    // focus that happens here.
    if (action.armsKeyboard) armKeyboard();
    haptic(action.destructive ? "warning" : "light");
    props.onClose();
    // Defer the actual mutation so the sheet collapse animation
    // doesn't visibly stutter while the workspace round-trip runs.
    queueMicrotask(action.onSelect);
  }

  return (
    <Show when={props.open}>
      <div
        class="outl-fade-in fixed inset-0 z-[55] bg-black/40 backdrop-blur-md"
        onClick={props.onClose}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-[55] flex max-h-dvh flex-col"
        style={{
          // Anchored to the bottom with no ceiling, the card simply grew
          // upward: a block with every action enabled ran past the top of
          // the screen and the first row sat under the status bar, out of
          // reach. The list scrolls instead, and the top inset keeps the
          // first row clear of the notch rather than merely on screen.
          "padding-top": "max(env(safe-area-inset-top), 12px)",
          "padding-bottom": "max(env(safe-area-inset-bottom), 16px)",
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging()
            ? "none"
            : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Actions card */}
        <div class="mx-3 mb-2 flex min-h-0 flex-col overflow-hidden rounded-2xl bg-(--color-outl-bg-elev)/95 shadow-[var(--shadow-capsule)] backdrop-blur-2xl">
          {/* Grab handle integrated at the top of the card so the
              gesture target stays close to the user's thumb. */}
          <span
            class="block py-2"
            style={{ "touch-action": "none" }}
            onPointerDown={drag.onPointerDown}
            onPointerMove={drag.onPointerMove}
            onPointerUp={drag.onPointerUp}
            onPointerCancel={drag.onPointerCancel}
            aria-label="Drag to close"
            role="button"
          >
            <span
              aria-hidden="true"
              class="mx-auto block h-1 w-10 rounded-full bg-(--color-outl-border)"
            />
          </span>
          <div class="ios-scroll min-h-0 flex-1 overflow-y-auto">
          <For each={props.actions}>
            {(action) => (
              <Show when={action.enabled?.() ?? true}>
                <button
                  type="button"
                  onClick={() => fire(action)}
                  class="flex w-full items-center justify-between gap-3 border-t border-(--color-outl-border)/30 px-4 py-3.5 text-left active:bg-(--color-outl-border)/30"
                  classList={{
                    "text-(--color-outl-destructive)":
                      action.destructive,
                  }}
                >
                  <span class="text-[16px] font-medium">{action.label}</span>
                  <svg
                    width="20"
                    height="20"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                    stroke-linejoin="round"
                    aria-hidden="true"
                  >
                    <path d={action.iconPath} />
                  </svg>
                </button>
              </Show>
            )}
          </For>
          </div>
        </div>
        {/* Cancel pill — separated by a gap, classic iOS action sheet
            convention. Always white text on a slightly different bg
            tier so it reads as "this is the dismiss option". */}
        <button
          type="button"
          onClick={props.onClose}
          class="mx-3 rounded-2xl bg-(--color-outl-bg-elev)/95 py-3.5 text-center text-[16px] font-semibold text-(--color-outl-accent) shadow-[var(--shadow-capsule)] backdrop-blur-2xl active:bg-(--color-outl-border)/30"
        >
          Cancel
        </button>
      </div>
    </Show>
  );
}
