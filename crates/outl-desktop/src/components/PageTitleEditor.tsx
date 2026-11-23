import { Show, createSignal } from "solid-js";

import type { PageMeta, PageView } from "@outl/shared/api/types";

import { renamePageFlow } from "../lib/page-actions";
import { PageTitle } from "./PageTitle";

/**
 * The page title, and the door to renaming it.
 *
 * Renaming used to live only behind the sidebar's hover `✎`, and the
 * sidebar is closed on every fresh launch — so the name a user reads at
 * the top of the page was the one thing on screen they could not act
 * on. Clicking it is the gesture every outliner trains, and clicking an
 * inert `<h1>` did nothing, so a single click costs nothing to take.
 *
 * Four rules, each bought by something in this header:
 *
 * - **A journal has no door.** Its slug *is* its date, the backend
 *   answers the rename with `CannotRenameJournal`, and `journalHeading`
 *   renders two derived spans with no single editable string. Offering
 *   it and then refusing is the defect `UX.md` section 3 names.
 * - **An unchanged name writes nothing.** A rename rewrites `[[links]]`
 *   across the workspace; it is not a keystroke to spend on a no-op.
 * - **The keyboard is taken, first.** The field sits inside
 *   `<main data-outl-region="outline">`, so without `stopPropagation`
 *   the Normal-mode dispatcher reads a typed title as chords (`d`
 *   deletes, `c` folds).
 * - **Pending is a mark, never a state.** The reply is a whole
 *   `PageView`, so the rename takes a round trip — and the title must
 *   not collapse, spin or unmount while it runs. The header already
 *   reserves its eyebrow's row height (`min-h-5`) because a collapsing
 *   row shifted this very title; a spinner in place of the `<h1>` is
 *   the same shift with a different cause (`UX.md` section 2).
 *
 * The commit calls `renamePageFlow` and nothing else. That is the one
 * desktop path into `@outl/shared`'s rename-or-merge flow, which owns
 * the order of events, the merge offer and every sentence either of
 * them produces.
 */
export function PageTitleEditor(props: {
  page: PageMeta;
  /** A reply landed: hand it to whoever owns navigation. */
  onRenamed: (view: PageView) => void;
}) {
  const [editing, setEditing] = createSignal(false);
  const [draft, setDraft] = createSignal("");
  const [pending, setPending] = createSignal(false);

  /** The name the user reads, which is what a rename starts from. */
  const label = () => props.page.title || props.page.slug;
  const renameable = () => props.page.kind !== "journal";

  let input: HTMLInputElement | undefined;
  let affordance: HTMLButtonElement | undefined;

  /** Back to the title, with the caret on it — the editor interrupted
   *  something, so it hands focus back rather than dropping it. The
   *  microtask is what lets the `<Show>` put the button back first. */
  const restoreFocus = () => queueMicrotask(() => affordance?.focus());

  function openEditor() {
    // Not while one is in flight. The editor closes on commit, so the
    // button comes back under the pointer while the round trip runs,
    // and a second open seeds its draft from `props.page` — which
    // still holds the *old* name, because the reply has not landed.
    // Committing that sends a second rename against the slug the first
    // one is retiring, and the user gets "page not found" for a page
    // they are looking at. A workspace-wide link rewrite on a heavily
    // linked page is long enough to click twice in.
    if (!renameable() || pending()) return;
    setDraft(label());
    setEditing(true);
    queueMicrotask(() => {
      input?.focus();
      // Selected, so typing replaces rather than appends.
      input?.select();
    });
  }

  function cancel() {
    setEditing(false);
    restoreFocus();
  }

  function commit() {
    // Enter and Escape both close the editor and a blur fires behind
    // them, so the closed state is what makes the late call a no-op.
    if (!editing()) return;
    // Read the draft before clearing it: the input is gone by the time
    // an async rename resolves.
    const name = draft().trim();
    const from = label();
    setEditing(false);
    // An empty name is a cancel, not an error — the user opened the
    // field and changed their mind, and the backend would refuse it
    // with a message that reads like a bug.
    if (!name || name === from) {
      restoreFocus();
      return;
    }
    setPending(true);
    // The call is made *inside* the chain: as an argument it would run
    // before the promise exists, so a synchronous throw would escape
    // past the handlers and out of the blur callback.
    void Promise.resolve()
      .then(() => renamePageFlow(props.page, name))
      .then((view) => {
        if (view) props.onRenamed(view);
        // No page: the merge offer was declined, or the backend refused
        // and the sentence is already on `<ErrorToast />`. Either way
        // this is still the page on screen, so the caret comes back to
        // its title — and nothing is said twice here (`UX.md` section
        // 3: one condition, one sentence).
        else restoreFocus();
      })
      .finally(() => setPending(false));
  }

  function onKey(e: KeyboardEvent) {
    // First, before anything can read it as a chord.
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      cancel();
    } else if (e.key === "Enter" && !e.isComposing) {
      e.preventDefault();
      commit();
    }
  }

  return (
    <Show
      when={editing()}
      fallback={
        <Show when={renameable()} fallback={<PageTitle page={props.page} />}>
          <span
            class="inline-flex items-baseline"
            aria-busy={pending() ? "true" : undefined}
          >
            <button
              ref={affordance}
              type="button"
              data-outl-page-title
              data-outl-header-stop
              aria-label={`Rename page "${label()}"`}
              title="Click to rename this page"
              onClick={openEditor}
              // `cursor-text` and a hover wash are the whole
              // affordance: a title that looks like a button stops
              // reading as the page's name. The negative margin keeps
              // the padded hover box from moving the title's left edge.
              class="-mx-1 cursor-text rounded px-1 text-left hover:bg-(--color-outl-fg)/[0.06]"
            >
              <PageTitle page={props.page} />
            </button>
            <Show when={pending()}>
              <span
                aria-hidden="true"
                title="Renaming…"
                class="ml-2 h-1.5 w-1.5 shrink-0 animate-pulse self-center rounded-full bg-(--color-outl-accent)"
              />
            </Show>
          </span>
        </Show>
      }
    >
      <input
        ref={input}
        type="text"
        value={draft()}
        aria-label="Page name"
        onInput={(e) => setDraft(e.currentTarget.value)}
        // The same 120ms grace the property editor takes: a click that
        // lands outside the field gets to be what it was before the
        // blur commits over it.
        onBlur={() => setTimeout(commit, 120)}
        onKeyDown={onKey}
        class="-mx-1 w-full rounded bg-transparent px-1 ring-1 ring-(--color-outl-accent)/40 outline-none"
      />
    </Show>
  );
}
