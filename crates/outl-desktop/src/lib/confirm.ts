/**
 * "Are you sure?" as app state, so the answer can come from a surface
 * the app controls.
 *
 * `window.confirm` used to ask it, and it is the one dialog in the
 * client that can take **no** `--color-outl-*` token: no destructive
 * fill, no palette, no focus rule, no way to say which of the two
 * buttons is the dangerous one. A merge deletes a page and the UI
 * offers no undo for it, so the most destructive action in the product
 * was being asked through the one surface that could not look like it
 * ([`DESIGN.md`](../../../../DESIGN.md) → Roles, `UX.md` →
 * Confirmation and destruction).
 *
 * The split is deliberate: a caller `await`s a boolean and never holds
 * a component, and `<ConfirmDialog />` draws whatever is pending. That
 * is what lets `@outl/shared`'s `confirmMerge: (existing) =>
 * Promise<boolean>` stay a plain callback on both clients while the
 * chrome around it stays each client's own.
 *
 * **The words are never written here.** A request carries the sentence
 * it was given — for rename / merge that is
 * `@outl/shared/pages/rename-merge`'s, so neither client invents its
 * own (`UX.md` → one condition, one sentence).
 *
 * `<ConfirmDialog />` is mounted at the shell for the app's lifetime,
 * which is what makes the promise safe to await: nothing can take the
 * answering surface away while a question is open.
 *
 * **The queue itself is `@outl/shared/confirm`**, because the bridge
 * from an `await`ed boolean to a signal a dialog renders had been
 * written three times — here and twice in `outl-mobile`. What stays
 * desktop-local is this client's request shape and the surface
 * registration below, which exists because the shell is the only
 * place a dialog is mounted here.
 */

import { createConfirmQueue } from "@outl/shared/confirm";

export interface ConfirmRequest {
  /** The sentence the user reads. The caller's, never the dialog's. */
  message: string;
  /** Verb on the acting button: `Merge`, `Delete`. */
  confirmLabel: string;
  /** Accessible name of the dialog itself. */
  label: string;
  /**
   * `true` paints the acting button as a solid `destructive` fill.
   * Reserved for what cannot be undone from inside the product —
   * `warn` is "look at this", and collapsing the two is how a
   * "3 blocks archived" toast ends up the colour of a page deletion.
   */
  destructive?: boolean;
}

let surfaces = 0;

const queue = createConfirmQueue<ConfirmRequest>({
  canAnswer: () => surfaces > 0,
  // A programming error (the shell mounts the dialog for the app's
  // lifetime), so it is logged — but it resolves to the side that
  // writes nothing, because a question nobody can see must not be able
  // to authorise a delete.
  onUnanswerable: (req) =>
    console.error("[confirm] no dialog is mounted; refusing:", req.label),
});

/** The open question, or `null`. Read by `<ConfirmDialog />`. */
export const pendingConfirm = queue.pending;

/**
 * `<ConfirmDialog />` saying it is on screen; returns the unregister.
 *
 * The flows `await` a boolean, so **something has to be able to
 * answer**. With nothing mounted the promise would never settle, and a
 * flow that never finishes is the quietest failure there is: the button
 * appears dead, no error is raised, and the `await` holds whatever is
 * behind it forever. So the unanswerable case is named rather than left
 * to chance (root `CLAUDE.md` invariant 10: every loser gets a path,
 * and the path has to be reachable).
 */
export function registerConfirmSurface(): () => void {
  surfaces += 1;
  return () => {
    surfaces = Math.max(0, surfaces - 1);
  };
}

/**
 * Ask, and resolve to the answer. A refusal resolves `false`, never a
 * rejection: declining is an outcome, not a failure. A second question
 * refuses the first — see `@outl/shared/confirm`.
 */
export const askConfirm = queue.ask;

/** Answer the open question. `<ConfirmDialog />`'s to call. */
export const answerConfirm = queue.answer;
