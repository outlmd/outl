import { For, Show } from "solid-js";

import { CHROME_TOUR_KEY, dismissOnce } from "@outl/shared/polish";

import { createSheetDrag } from "../lib/sheet-drag";
import { CHROME_TOUR } from "../lib/chrome-tour";

/**
 * "What's what" — one screen naming the phone's own controls
 * ([#574](https://github.com/outlmd/outl/issues/574)).
 *
 * Shown once, on the first launch past onboarding, and reachable
 * afterwards from Settings. Two rules it has to keep, both from the
 * issue and from [`UX.md`](../../../UX.md):
 *
 * - **Nothing here has to be completed before the user can write.**
 *   Every way out closes it: the Got it button, the backdrop, a drag
 *   on the handle. There is no step, no next, no progress.
 * - **Dismissal is per device**, like every other first-run surface —
 *   it is "I have read this", which is view state and must never
 *   become an Op (root `CLAUDE.md` invariant 7).
 *
 * It opens on mount rather than behind the journal's load gate, and
 * that is deliberate twice over: `loaded` is the sole gate on the
 * skeleton, so reading it out here would put a load state around
 * something that is not the outline (`Journal.load-gate.test.ts` fails
 * on exactly that) — and it buys nothing anyway, since the only launch
 * that shows this sheet is a device's first, which has no history to
 * replay.
 *
 * The words are `lib/chrome-tour.ts`'s; the one thing decided here is
 * that *reaching the end is not what dismisses it*. `onClose` marks it
 * read however it was closed, because a user who swiped this away has
 * read as much of it as they intend to, and showing it again next
 * launch would be the app not listening.
 */
export function GuideSheet(props: { open: boolean; onClose: () => void }) {
  const drag = createSheetDrag(() => close());

  function close() {
    dismissOnce(CHROME_TOUR_KEY);
    props.onClose();
  }

  return (
    <Show when={props.open}>
      <div class="outl-fade-in fixed inset-0 z-[57] bg-black/40 backdrop-blur-md" onClick={close} />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-[57] flex max-h-dvh flex-col rounded-t-2xl bg-(--color-outl-bg-elev) pt-3 shadow-2xl"
        style={{
          "padding-bottom": "max(var(--safe-area-inset-bottom), 12px)",
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging() ? "none" : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label="What's what"
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
        <h2 class="px-4 pt-2 pb-1 text-[17px] font-semibold">What's what</h2>
        <p class="px-4 pb-3 text-[13px] text-(--color-outl-fg-dim)">
          The buttons around your notes. You can open this again from Settings.
        </p>
        <div class="ios-scroll min-h-0 flex-1 overflow-y-auto px-4">
          <For each={CHROME_TOUR}>
            {(row) => (
              <div class="border-t border-(--color-outl-border)/30 py-3" data-tour={row.id}>
                <div class="text-[15px] font-medium text-(--color-outl-fg)">{row.title}</div>
                <div class="text-[14px] leading-snug text-(--color-outl-fg-dim)">{row.body}</div>
              </div>
            )}
          </For>
        </div>
        <div class="px-4 pt-3">
          <button
            type="button"
            onClick={close}
            class="min-h-12 w-full rounded-xl bg-(--color-outl-accent) text-[16px] font-semibold text-(--color-outl-accent-fg) active:opacity-80"
          >
            Got it
          </button>
        </div>
      </div>
    </Show>
  );
}
