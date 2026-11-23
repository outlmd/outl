/**
 * "Are you sure?" as a promise, so the answer can come from a surface
 * the app controls.
 *
 * `@outl/shared/pages/rename-merge` and every flow shaped like it take
 * the confirmation as a callback — `confirmMerge: (existing) =>
 * Promise<boolean>` — precisely so the chrome stays each client's own:
 * a modal card with a focus trap on the desktop, an iOS-style alert on
 * the phone. What is *not* each client's own is the bridge from that
 * callback to a signal a component can render, and it had been written
 * three times: once in `outl-desktop/src/lib/confirm.ts`, and twice in
 * `outl-mobile` (the page switcher's actions and the page title's
 * rename), the third copy added in the same change that noticed.
 *
 * So the queue lives here and the dialog does not. The request payload
 * is a type parameter, because the two clients genuinely differ in what
 * their chrome needs (the desktop carries an accessible `label` and a
 * `destructive` flag; the phone carries a `title`), and forcing one
 * shape would put layout decisions in a shared module.
 *
 * **The words are never written here.** A request carries the sentence
 * it was given, which for rename and merge is
 * `@outl/shared/pages/rename-merge`'s, so neither client invents its
 * own ([`UX.md`](../../../../UX.md) → one condition, one sentence).
 */

import { createSignal, type Accessor } from "solid-js";

/** An open question, plus the resolver the surface answers through. */
export type Pending<Req> = Req & { resolve: (ok: boolean) => void };

export interface ConfirmQueue<Req> {
  /** The open question, or `null`. Read by the client's dialog. */
  pending: Accessor<Pending<Req> | null>;
  /**
   * Ask, and resolve to the answer.
   *
   * A refusal resolves `false`, never a rejection: declining is an
   * outcome, not a failure.
   *
   * A second question replaces the first and **refuses** it. Two cards
   * cannot be on screen at once, and a promise nobody can reach is a
   * flow that never finishes — a refusal writes nothing, which is the
   * safe end of that race.
   */
  ask(req: Req): Promise<boolean>;
  /** Answer the open question. The dialog's to call. */
  answer(ok: boolean): void;
}

export interface ConfirmQueueOptions<Req> {
  /**
   * Asked before a question is queued; `false` refuses it immediately.
   *
   * A flow `await`s a boolean, so **something has to be able to
   * answer**. With no dialog mounted the promise would never settle,
   * and a flow that never finishes is the quietest failure there is:
   * the button appears dead, no error is raised, and the `await` holds
   * whatever is behind it forever. A client whose dialog is mounted for
   * the app's lifetime can leave this out; one that mounts it
   * conditionally owes the check (root `CLAUDE.md` invariant 10: every
   * loser gets a path, and the path has to be reachable).
   */
  canAnswer?: () => boolean;
  /** Called when `canAnswer` refused, for the client to log. */
  onUnanswerable?: (req: Req) => void;
}

/** A fresh queue. One per surface that asks questions. */
export function createConfirmQueue<Req>(
  options: ConfirmQueueOptions<Req> = {},
): ConfirmQueue<Req> {
  const [pending, setPending] = createSignal<Pending<Req> | null>(null);

  function answer(ok: boolean): void {
    const open = pending();
    if (!open) return;
    setPending(null);
    open.resolve(ok);
  }

  function ask(req: Req): Promise<boolean> {
    // Refuse the one already on screen before replacing it, so its
    // caller finishes rather than hanging behind a card nobody sees.
    answer(false);
    if (options.canAnswer && !options.canAnswer()) {
      options.onUnanswerable?.(req);
      return Promise.resolve(false);
    }
    return new Promise<boolean>((resolve) =>
      // The setter-function form: `Req` is a type parameter, so TS
      // cannot rule out a callable request, which the value form would
      // read as an updater.
      setPending(() => ({ ...req, resolve }) as Pending<Req>),
    );
  }

  return { pending, ask, answer };
}
