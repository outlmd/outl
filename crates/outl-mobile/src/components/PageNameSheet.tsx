import { Show, createEffect, createSignal } from "solid-js";

import { createSheetDrag } from "../lib/sheet-drag";
import { useKeyboardInset } from "../lib/viewport";

/**
 * One-line text entry in a bottom sheet, prefilled and selected so a
 * single keystroke replaces it. Used to rename a page: the page
 * switcher's actions menu opens it with the current title.
 *
 * Return submits, the same way it commits a field everywhere else. An
 * empty or unchanged name submits nothing: the button is disabled, so
 * there is no write to cancel. The touch targets are 48pt tall, past
 * the 44pt floor in `docs/mobile-ux.md`.
 */
export function PageNameSheet(props: {
  open: boolean;
  /** What the sheet is for, e.g. `Rename page`. */
  title: string;
  /** The value the field opens with. */
  initial: string;
  submitLabel: string;
  onSubmit: (name: string) => void;
  onCancel: () => void;
}) {
  const [text, setText] = createSignal("");
  const keyboardInset = useKeyboardInset();
  const drag = createSheetDrag(() => props.onCancel());
  let input: HTMLInputElement | undefined;

  createEffect(() => {
    if (!props.open) return;
    setText(props.initial);
    // Next frame: the sheet has mounted and iOS accepts the focus.
    requestAnimationFrame(() => {
      input?.focus();
      input?.select();
    });
  });

  const value = () => text().trim();
  const canSubmit = () => value() !== "" && value() !== props.initial.trim();

  function submit() {
    if (!canSubmit()) return;
    props.onSubmit(value());
  }

  return (
    <Show when={props.open}>
      <div
        class="outl-fade-in fixed inset-0 z-[56] bg-black/40 backdrop-blur-md"
        onClick={props.onCancel}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-[56] max-h-[80vh] overflow-y-auto rounded-t-2xl bg-(--color-outl-bg-elev) px-4 pt-3 shadow-2xl"
        style={{
          "padding-bottom": `max(env(safe-area-inset-bottom), ${keyboardInset()}px, 12px)`,
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging() ? "none" : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label={props.title}
      >
        <span
          class="mx-auto block h-3 w-16 cursor-grab py-1 active:cursor-grabbing"
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
        <h2 class="px-1 pt-2 pb-3 text-[17px] font-semibold">{props.title}</h2>
        <input
          ref={input}
          type="text"
          aria-label={props.title}
          value={text()}
          autocapitalize="off"
          autocomplete="off"
          spellcheck={false}
          enterkeyhint="done"
          onInput={(e) => setText(e.currentTarget.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              submit();
            } else if (e.key === "Escape") {
              e.preventDefault();
              props.onCancel();
            }
          }}
          class="min-h-12 w-full rounded-xl bg-(--color-outl-bg) px-3 text-[16px] outline-none"
        />
        <div class="grid grid-cols-2 gap-3 pt-4">
          <button
            type="button"
            onClick={props.onCancel}
            class="min-h-12 rounded-xl text-[16px] font-medium text-(--color-outl-accent) active:bg-(--color-outl-border)/30"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={!canSubmit()}
            onClick={submit}
            class="min-h-12 rounded-xl bg-(--color-outl-accent) text-[16px] font-semibold text-(--color-outl-accent-fg) active:opacity-80 disabled:opacity-40"
          >
            {props.submitLabel}
          </button>
        </div>
      </div>
    </Show>
  );
}
