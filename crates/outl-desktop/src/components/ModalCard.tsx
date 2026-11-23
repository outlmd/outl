import { type JSX, Show, createEffect, onCleanup } from "solid-js";
import { Portal } from "solid-js/web";

/**
 * The floating card every desktop modal is, and the one way out of it.
 *
 * The markup was written four times before this existed — the quick
 * switcher, quick capture, the page-action dialog, and this component's
 * first caller — and the copies had already drifted on the parts that
 * are not decoration: one card announced itself to a screen reader and
 * the others did not, and Escape worked only while focus sat in the one
 * field that handled it.
 *
 * So the shell owns three things, not just the class list:
 *
 * - **One elevation.** `bg-elev` is the only floating colour there is
 *   (`DESIGN.md` → Roles), so a modal cannot invent a second step.
 * - **Escape, wherever focus is.** The listener is on `window`, which
 *   covers focus that left the card. A field inside that handles its
 *   own Escape still wins, because `stopPropagation` there keeps the
 *   event from reaching `window` at all — and a field has to do that
 *   anyway, or the Normal-mode dispatcher reads what is being typed as
 *   chords.
 * - **The backdrop is a way out, and only from the backdrop itself.**
 *   The `e.target === e.currentTarget` test is what stops a click that
 *   started on the card closing the thing it landed on.
 *
 * `<Picker />` and `<QuickCapture />` still hold their own copies of
 * the class list: this shell covers the two modals that *ask a
 * question*, and moving a surface with its own result list and its own
 * key handling onto it is a separate change.
 */
export function ModalCard(props: {
  open: boolean;
  /** Accessible name. A modal that announces nothing is a modal a
   *  screen-reader user meets as an unexplained focus jump. */
  label: string;
  /** `alertdialog` for a question that interrupts, the default for the
   *  rest. The difference is real to assistive tech: an alert dialog
   *  is read out on open. */
  role?: "dialog" | "alertdialog";
  onClose: () => void;
  children: JSX.Element;
}): JSX.Element {
  // Tied to `open`, never to mount: callers keep this component alive
  // for their own lifetime and toggle the flag, so a mount-time
  // listener would outlive every close.
  createEffect(() => {
    if (!props.open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      props.onClose();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <Show when={props.open}>
      <Portal>
        <div
          class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
          onClick={(e) => {
            if (e.target === e.currentTarget) props.onClose();
          }}
        >
          <div
            role={props.role ?? "dialog"}
            aria-modal="true"
            aria-label={props.label}
            class="mt-24 w-[520px] max-w-[90vw] overflow-hidden rounded-pop border border-(--color-outl-border)/60 bg-(--color-outl-bg-elev) shadow-pop"
          >
            {props.children}
          </div>
        </div>
      </Portal>
    </Show>
  );
}
