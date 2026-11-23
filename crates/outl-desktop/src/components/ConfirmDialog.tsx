import { Show, createEffect, onCleanup, onMount } from "solid-js";

import { answerConfirm, pendingConfirm, registerConfirmSurface } from "../lib/confirm";
import { ModalCard } from "./ModalCard";

/**
 * The surface that answers whatever `askConfirm` is asking.
 *
 * Mounted once at the shell, drawing nothing until a flow asks —
 * `lib/confirm.ts` owns the queue, the promise and why this replaced
 * `window.confirm`.
 *
 * Three rules this component keeps, and nothing else:
 *
 * - **The acting button carries the tone.** A `destructive` request
 *   gets a solid `destructive` fill with its paired foreground, which
 *   is the signal a native dialog could not give at all. Never `warn`
 *   (`DESIGN.md` → Roles), and never a hex: the fill and the text on
 *   it are two palette fields, measured as a pair.
 * - **Cancel is where focus lands.** The safe answer is one Return
 *   away and the dangerous one is a deliberate move, which is the same
 *   reasoning behind the phone's sustained long-press.
 * - **The caret goes back.** A question interrupts something, so on
 *   close focus returns to whatever held it — when that element still
 *   exists. It often will not: the title editor closes itself before
 *   asking, on purpose, so two surfaces never own the keyboard at
 *   once.
 */
export function ConfirmDialog() {
  let cancelBtn: HTMLButtonElement | undefined;
  let actBtn: HTMLButtonElement | undefined;
  let opener: HTMLElement | null = null;

  // Says there is something here to answer with, so `askConfirm` never
  // hands a caller a promise nothing can settle.
  onMount(() => onCleanup(registerConfirmSurface()));

  createEffect(() => {
    if (!pendingConfirm()) return;
    opener = document.activeElement as HTMLElement | null;
    // After render: the buttons are created by the `<Show>` this effect
    // is reading the condition of.
    queueMicrotask(() => cancelBtn?.focus());
  });

  function close(ok: boolean) {
    const back = opener;
    opener = null;
    answerConfirm(ok);
    if (back?.isConnected) back.focus();
  }

  function onKey(e: KeyboardEvent) {
    // The dialog owns the keyboard while it is up, so the Normal-mode
    // dispatcher on `window` never sees these. That also means Escape
    // has to be handled here and not left to `<ModalCard />`, whose
    // listener this very call keeps the event away from.
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      close(false);
      return;
    }
    if (e.key !== "Tab") return;
    // Two stops, so the trap is the whole focus order: Tab must not
    // walk out of a modal into the page behind it.
    const stops = [cancelBtn, actBtn].filter((b): b is HTMLButtonElement => !!b);
    if (stops.length === 0) return;
    e.preventDefault();
    const at = stops.indexOf(document.activeElement as HTMLButtonElement);
    const to = (at + (e.shiftKey ? -1 : 1) + stops.length) % stops.length;
    stops[to].focus();
  }

  return (
    <Show when={pendingConfirm()}>
      {(req) => (
        <ModalCard
          open
          role="alertdialog"
          label={req().label}
          onClose={() => close(false)}
        >
          <div onKeyDown={onKey} class="px-4 pt-4 pb-3">
            {/* `whitespace-pre-line`: the sentences carry their own
             *  paragraph breaks, which a native dialog rendered and a
             *  `<p>` would otherwise collapse. */}
            <p class="whitespace-pre-line text-[13px] leading-snug text-(--color-outl-fg)">
              {req().message}
            </p>
            <div class="mt-4 flex items-center justify-end gap-2">
              <button
                ref={cancelBtn}
                type="button"
                onClick={() => close(false)}
                class="rounded-md px-3 py-1.5 text-[13px] text-(--color-outl-fg-dim) hover:bg-(--color-outl-fg)/10 hover:text-(--color-outl-fg)"
              >
                Cancel
              </button>
              <button
                ref={actBtn}
                type="button"
                onClick={() => close(true)}
                class="rounded-md px-3 py-1.5 text-[13px] font-medium"
                classList={{
                  "bg-(--color-outl-destructive) text-(--color-outl-destructive-fg) hover:opacity-90":
                    req().destructive === true,
                  "bg-(--color-outl-accent) text-(--color-outl-accent-fg) hover:opacity-90":
                    req().destructive !== true,
                }}
              >
                {req().confirmLabel}
              </button>
            </div>
          </div>
        </ModalCard>
      )}
    </Show>
  );
}
