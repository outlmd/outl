import { Show, createEffect, createSignal } from "solid-js";

import { createSheetDrag } from "../lib/sheet-drag";
import { useKeyboardInset } from "../lib/viewport";

/**
 * Write one line into today's journal without leaving the page on
 * screen. The bottom bar's `+` opens this from anywhere except today
 * itself, where the `+` adds a block in place instead.
 *
 * Return adds it, the same way it ends a block everywhere else; an
 * empty field adds nothing. The sheet stays open after a send so a
 * burst of thoughts is one gesture each, and closes on Done.
 */
export function CaptureSheet(props: {
  open: boolean;
  /** "Today, October 12", so it is clear where the line goes. */
  target: string;
  onClose: () => void;
  onSubmit: (text: string) => Promise<boolean>;
}) {
  const [text, setText] = createSignal("");
  const [sending, setSending] = createSignal(false);
  const keyboardInset = useKeyboardInset();
  // Same drag-down-to-close as every other sheet.
  const drag = createSheetDrag(() => props.onClose());
  let input: HTMLTextAreaElement | undefined;

  createEffect(() => {
    if (!props.open) return;
    setText("");
    // Next frame: the sheet has mounted and iOS accepts the focus.
    requestAnimationFrame(() => input?.focus());
  });

  async function send() {
    const value = text().trim();
    if (!value || sending()) return;
    setSending(true);
    const ok = await props.onSubmit(value);
    setSending(false);
    if (ok) setText("");
    input?.focus();
  }

  return (
    <Show when={props.open}>
      <div class="outl-fade-in fixed inset-0 z-50 bg-black/30 backdrop-blur-sm" onClick={props.onClose} />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-50 rounded-t-2xl bg-(--color-outl-bg-elev) px-4 pt-3 shadow-2xl"
        style={{
          "padding-bottom": `max(var(--safe-area-inset-bottom), ${keyboardInset()}px, 12px)`,
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging() ? "none" : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label="Add to today's journal"
      >
        <span
          class="mx-auto -mt-1 mb-1 block h-3 w-16 cursor-grab py-1 active:cursor-grabbing"
          style={{ "touch-action": "none" }}
          onPointerDown={drag.onPointerDown}
          onPointerMove={drag.onPointerMove}
          onPointerUp={drag.onPointerUp}
          onPointerCancel={drag.onPointerCancel}
          aria-label="Drag to close"
          role="button"
        >
          <span aria-hidden="true" class="mx-auto block h-1 w-10 rounded-full bg-(--color-outl-border)" />
        </span>
        <div class="flex items-center justify-between">
          <span class="text-[13px] font-medium text-(--color-outl-fg-dim)">{props.target}</span>
          <button
            type="button"
            onClick={props.onClose}
            class="py-1 font-medium text-(--color-outl-accent) active:opacity-60"
          >
            <span class="text-[15px]">Done</span>
          </button>
        </div>
        <div class="mt-2 flex items-end gap-2">
          <textarea
            ref={input}
            rows={2}
            value={text()}
            onInput={(e) => setText(e.currentTarget.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.shiftKey && !e.isComposing) {
                e.preventDefault();
                void send();
              }
            }}
            placeholder="What's on your mind?"
            class="min-h-[52px] flex-1 resize-none rounded-xl bg-(--color-outl-bg) px-3 py-2.5 text-[17px] leading-snug outline-none placeholder:text-(--color-outl-fg-dimmer)"
          />
          <button
            type="button"
            onClick={() => void send()}
            disabled={!text().trim() || sending()}
            class="h-11 shrink-0 rounded-full bg-(--color-outl-accent) px-4 font-semibold text-(--color-outl-accent-fg) active:opacity-70 disabled:opacity-40"
          >
            <span class="text-[15px]">Add</span>
          </button>
        </div>
      </div>
    </Show>
  );
}
