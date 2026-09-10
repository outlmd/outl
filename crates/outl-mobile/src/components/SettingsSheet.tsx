import { type JSX, Show, createEffect, createSignal } from "solid-js";
import {
  clearCountsInStore,
  isOrderLocked,
  lockOrderInStore,
  orderedMiddleActions,
  orderedMiddleFromStore,
  unlockOrderInStore,
} from "@outl/shared/toolbar";

import { createSheetDrag } from "../lib/sheet-drag";
import { haptic } from "../lib/haptics";

/**
 * `<SettingsSheet />` — mobile's preferences surface.
 *
 * The first one: every other sheet in this client (`DevicesSheet`,
 * `TemplateSheet`, `RemindersSheet`, `PropertiesSheet`) shows workspace
 * *content*, so a preference had nowhere to live and the toolbar lock
 * (#271) could not be offered at all. Chrome mirrors `TemplateSheet`
 * (same drag-dismiss hook, blurred card, safe-area padding) so it reads
 * as one of the family rather than a new kind of screen.
 *
 * Everything here is **per-device UI state in `localStorage`**, not
 * workspace data: none of it goes through the op log, because two
 * devices disagreeing about their own toolbar layout is not a conflict
 * to reconcile (root `CLAUDE.md` invariant 7). That is also why this
 * sheet calls no Tauri command.
 */
export function SettingsSheet(props: {
  open: boolean;
  onClose: () => void;
}): JSX.Element {
  const drag = createSheetDrag(() => props.onClose());
  const [locked, setLocked] = createSignal(false);
  const [resetDone, setResetDone] = createSignal(false);

  // Re-read on every open. `<Show>` gates the markup, not the
  // component: `Journal` mounts this once and toggles `open`, so a
  // plain `createSignal(isOrderLocked())` in the body would read the
  // store a single time, at app start. `TemplateSheet` and
  // `RemindersSheet` carry the same effect for the same reason.
  createEffect(() => {
    if (!props.open) return;
    setLocked(isOrderLocked());
    setResetDone(false);
  });

  function toggleLock() {
    haptic("light");
    if (locked()) {
      unlockOrderInStore();
      setLocked(false);
      return;
    }
    // Freeze what the user is looking at today, not the cold-start
    // order — "lock" means "keep this", and the row they have learned
    // is the MFU one their taps produced.
    lockOrderInStore(orderedMiddleFromStore());
    setLocked(true);
  }

  function resetOrder() {
    haptic("light");
    clearCountsInStore();
    // A locked bar renders its frozen snapshot, so wiping the counts
    // alone would change nothing on screen. Re-freeze on the cold-start
    // order instead: the user asked for the default back, and they did
    // not ask to be unlocked.
    if (locked()) lockOrderInStore(orderedMiddleActions({}));
    // The bar isn't on screen while this sheet is, and it only
    // re-reads at the next keyboard, so nothing visible confirms the
    // tap. Say so in the row itself rather than leaving the user
    // wondering whether the button did anything.
    setResetDone(true);
  }

  return (
    <Show when={props.open}>
      <div
        class="outl-fade-in fixed inset-0 z-[55] bg-black/40 backdrop-blur-md"
        onClick={props.onClose}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-[55] flex flex-col"
        style={{
          "padding-bottom": "max(env(safe-area-inset-bottom), 16px)",
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging()
            ? "none"
            : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div class="mx-3 mb-2 overflow-hidden rounded-2xl bg-(--color-outl-bg-elev)/95 shadow-[var(--shadow-capsule)] backdrop-blur-2xl">
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

          <div class="px-4 pb-1 pt-1">
            <span class="text-[13px] font-semibold uppercase tracking-wide text-(--color-outl-fg-dim)">
              Keyboard toolbar
            </span>
          </div>

          <button
            type="button"
            role="switch"
            aria-checked={locked()}
            onClick={toggleLock}
            class="flex w-full items-center gap-3 border-t border-(--color-outl-border)/30 px-4 py-3.5 text-left active:bg-(--color-outl-border)/30"
          >
            <span class="min-w-0 flex-1">
              <span class="block text-[16px] font-medium text-(--color-outl-fg)">
                Lock button order
              </span>
              <span class="mt-0.5 block text-[13px] leading-snug text-(--color-outl-fg-dim)">
                Keep the buttons where they are. outl still counts which
                ones you use, it just stops rearranging them.
              </span>
            </span>
            <Switch on={locked()} />
          </button>

          <button
            type="button"
            onClick={resetOrder}
            class="flex w-full flex-col items-start gap-0.5 border-t border-(--color-outl-border)/30 px-4 py-3.5 text-left active:bg-(--color-outl-border)/30"
          >
            <span class="text-[16px] font-medium text-(--color-outl-accent)">
              Reset button order
            </span>
            <span class="text-[13px] leading-snug text-(--color-outl-fg-dim)">
              <Show
                when={resetDone()}
                fallback="Forget the usage counts and go back to the original layout."
              >
                Order reset. It applies the next time the keyboard opens.
              </Show>
            </span>
          </button>

          <p class="border-t border-(--color-outl-border)/30 px-4 py-3 text-[12px] leading-snug text-(--color-outl-fg-dim)/80">
            Takes effect the next time the keyboard opens.
          </p>
        </div>

        <button
          type="button"
          onClick={props.onClose}
          class="mx-3 rounded-2xl bg-(--color-outl-bg-elev)/95 py-3.5 text-center text-[16px] font-semibold text-(--color-outl-accent) shadow-[var(--shadow-capsule)] backdrop-blur-2xl active:bg-(--color-outl-border)/30"
        >
          Done
        </button>
      </div>
    </Show>
  );
}

/** iOS-style switch. Presentational: the enclosing row owns the tap and
 *  the `role="switch"` semantics, so this is `aria-hidden` and never a
 *  second focus stop for the same control. */
function Switch(props: { on: boolean }): JSX.Element {
  return (
    <span
      aria-hidden="true"
      class="relative block h-[31px] w-[51px] shrink-0 rounded-full transition-colors duration-200"
      classList={{
        "bg-(--color-outl-accent)": props.on,
        "bg-(--color-outl-border)": !props.on,
      }}
    >
      <span
        class="absolute top-[2px] block h-[27px] w-[27px] rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,0.3)] transition-[left] duration-200"
        style={{ left: props.on ? "22px" : "2px" }}
      />
    </span>
  );
}
