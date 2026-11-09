/**
 * Trash panel — every block deleted out of this workspace, and the way
 * back (issue #287).
 *
 * Opened from the `🗑` button in the chrome toggle bar; closed with
 * `Esc`.
 *
 * **No chord yet, and that is a file-size blocker rather than a
 * decision.** A chord needs an `Action` variant, and
 * `outl_shortcuts::support`'s exhaustive `match` is at its frozen
 * 600-line baseline, so the arm cannot be added until that file is
 * split. Declaring what all three clients do with a new `Action` is the
 * mechanism invariant 12 exists to force, not a price worth avoiding —
 * `TimelinePanel` has carried the same gap for the same reason. So the
 * button is the affordance for now, the TUI opens the same surface
 * through `/trash` (its command registry is its button), and
 * `Capability::Trash` is where the pair is recorded, because that
 * catalog is precisely for a feature no chord reaches.
 *
 * **Distinct from `Cmd+Z`.** Undo walks this session's mutations
 * (`outl_actions::history`); this reaches a deletion from any device, on
 * any day, because it reads the op log rather than a stack.
 *
 * **Nothing here decides what can be restored, or what to say about
 * it.** `entry.refusal` is `outl_actions::trash::refusal_for`'s own
 * verdict, worded by the catalog for `Surface::Desktop`, and a refused
 * row is rendered, not hidden: a user looking for a block needs to find
 * it and be told why it cannot come back, not fail to find it at all.
 *
 * The wording is per surface because the *way out* is. This panel used
 * to print `outl doctor` and `outl block tree <ULID>` verbatim — a
 * binary the user may not have, against an id this panel does not show
 * (invariant 2). `outl_actions::refusal::Remedy` owns that answer now,
 * and a surface with no way out says so instead of prescribing one.
 *
 * ## A refusal can go stale between the list and the click
 *
 * The verdict depends on the live tree, so a peer can trash the parent a
 * row would land under after this list was fetched. The row's `refusal`
 * is therefore a filter, never a promise:
 *
 * 1. `restoreTrashEntry` is allowed to reject even on a row that looked
 *    actionable, and the rejection carries the **same verdict** the row
 *    would have shown — both come off `trash::refusal_for`. Not the same
 *    *sentence*: a row is worded for this surface and carries the way
 *    out, while the mutation's error is the bare diagnosis, which is why
 *    `attemptNotice` asks whether the row is refused rather than
 *    comparing the two strings.
 * 2. That sentence lands *in the row* (`attemptNotice`), not in a toast:
 *    a toast scrolls away from the thing it is about, and the user is
 *    still looking at a list of rows.
 * 3. The listing then refetches, and **the listing alone decides whether
 *    the row is still restorable**. The attempt's notice is additive and
 *    is cleared by that refetch and by closing the panel. Folding the two
 *    together gave this component a third state the backend never
 *    returned, and since nothing cleared it, a row stayed un-restorable
 *    after a peer put its parent back — forever, across reopens.
 *
 * The cost is one extra `list_trash` after a failed restore, which is
 * the same read the panel already does on open.
 */

import { For, Show, createEffect, createResource, createSignal } from "solid-js";

import { listTrash, restoreTrashEntry, type TrashEntry } from "@outl/shared/api/trash";
import type { PageView } from "@outl/shared/api/types";

import { appState, setAppState } from "../lib/store";
import { focusFirstRowOnOpen, rovingKeydown } from "../lib/keyboard-reach";

export interface TrashPanelDeps {
  applyView: (view: PageView) => void;
  setError: (msg: string) => void;
}

/**
 * "3 days ago" for a deletion's wall clock.
 *
 * Rendered, never sorted on: `deleted_at_ms` comes from whichever device
 * did the deleting, so a peer with a skewed clock can legitimately show
 * a time that reads out of order. The row order is the backend's, which
 * cannot depend on a clock. Same rule `TimelinePanel`'s `formatAt`
 * follows.
 */
function formatWhen(ms: number | null): string {
  if (ms === null) return "";
  const seconds = Math.round((Date.now() - ms) / 1000);
  if (seconds < 90) return "just now";
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  const days = Math.round(hours / 24);
  if (days < 30) return `${days}d ago`;
  return new Date(ms).toLocaleDateString();
}

export function TrashPanel(props: TrashPanelDeps) {
  // Gated on the panel being open, for the reason `TasksPanel` gives:
  // the component is mounted for the app's whole life, so an ungated
  // `createResource` would fold every deletion's parent trail at boot.
  const open = () => (appState.trashOpen ? true : undefined);

  const [data, { refetch }] = createResource(open, async () => {
    try {
      return await listTrash();
    } catch (e) {
      props.setError(e instanceof Error ? e.message : String(e));
      return [] as TrashEntry[];
    }
  });

  // The id a restore is in flight for, and what the *mutation* said
  // about the last attempt.
  const [busy, setBusy] = createSignal<string | null>(null);
  const [failed, setFailed] = createSignal<{ id: string; why: string } | null>(null);

  const close = () => setAppState("trashOpen", false);

  // **Every opening starts clean.** The component is mounted for the
  // app's whole life, so an attempt notice nothing clears outlives its
  // condition: a row refused because a peer had trashed its parent would
  // still read as refused after that peer put the parent back, on every
  // reopen, forever.
  //
  // On *open* rather than inside the fetcher, because a failed restore
  // refetches on purpose — clearing there would wipe the sentence that
  // refetch exists to accompany. And not in `close()` either: the
  // capsule toggle flips the flag directly, so a close that never calls
  // it would skip the reset.
  createEffect(() => {
    if (appState.trashOpen) setFailed(null);
  });

  let list: HTMLDivElement | undefined;
  focusFirstRowOnOpen(() => appState.trashOpen, data, () => list, "[data-trash-row]");

  async function restore(entry: TrashEntry) {
    setBusy(entry.id);
    setFailed(null);
    try {
      const reply = await restoreTrashEntry(entry.id);
      // The view *is* the landing page, so applying it both refreshes
      // and navigates — the user sees where their block went instead of
      // being told it came back somewhere.
      if (reply.view) props.applyView(reply.view);
      close();
    } catch (e) {
      // Not a toast: the row is what the user is looking at, and the
      // sentence is about that row. Refetching settles it into the
      // refused state the listing should already have shown.
      setFailed({ id: entry.id, why: e instanceof Error ? e.message : String(e) });
      void refetch();
    } finally {
      setBusy(null);
    }
  }

  /**
   * What the last attempt on this row said, if it was this row.
   *
   * Deliberately **not** folded into `entry.refusal`. Whether a row can
   * be restored has one owner — the backend's listing — and an attempt's
   * outcome is a different fact about a moment that has passed. Merging
   * them gave the panel a third state the backend had never returned,
   * and because it outlived every refetch the row stayed un-restorable
   * after the condition cleared.
   */
  const attemptNotice = (entry: TrashEntry) => {
    const late = failed();
    if (late?.id !== entry.id) return null;
    // **Asked of the verdict, not of the prose.** Once the refetch this
    // failure triggered carries a refusal, the row already says why and
    // repeating it would print the same thing twice — so the row's own
    // verdict wins, which is also rule 3 above. This used to compare the
    // two strings for equality, which worked only while they were one
    // `ActionError::to_string()`; the row's sentence now carries the way
    // out this client actually has and the attempt's does not, so a
    // string compare would never match and every refused restore would
    // print its reason twice, in two voices.
    return entry.refusal ? null : late.why;
  };

  return (
    <Show when={appState.trashOpen}>
      <div
        class="fixed inset-0 z-50 flex items-start justify-center bg-black/50 backdrop-blur-sm"
        onClick={close}
      >
        <div
          class="mt-16 max-h-[80vh] w-[680px] max-w-[92vw] overflow-hidden rounded-lg border border-(--color-outl-border) bg-(--color-outl-bg-elev) shadow-2xl"
          onClick={(e) => e.stopPropagation()}
        >
          <header class="flex items-baseline justify-between border-b border-(--color-outl-border) px-5 py-3">
            <h2 class="text-lg font-semibold">Trash</h2>
            <span class="text-xs opacity-50">
              <Show when={data()}>
                {(rows) => <>{rows().length} deleted · </>}
              </Show>
              Esc to close
            </span>
          </header>

          <div
            ref={list}
            data-trash-list
            class="max-h-[64vh] overflow-y-auto px-5 py-3"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                close();
                return;
              }
              rovingKeydown(e, "vertical", { items: "[data-trash-row]" });
            }}
          >
            <Show
              when={(data()?.length ?? 0) > 0}
              fallback={
                <Show when={!data.loading}>
                  <p class="py-6 text-center text-sm opacity-60">
                    Nothing has been deleted in this workspace. A deleted block comes
                    here instead of disappearing, from any of your devices.
                  </p>
                </Show>
              }
            >
              <For each={data()}>
                {(entry) => (
                  <div class="flex items-start gap-3 border-b border-(--color-outl-border)/40 py-2 last:border-b-0">
                    {/* The **row** is the roving-focus item, not the
                        Restore button.

                        Two reasons, and the first was a real hole: a
                        trash holding only refused rows (18 on the
                        reference workspace) has no button at all, so a
                        button-keyed walk found `items.length === 0` and
                        this list's own `Esc` handler — which needs focus
                        inside it — never fired. The second is that
                        `FOCUSABLE` is a selector *list*, so `button`
                        matches on its own tag and no `tabindex` on the
                        button can take it out — the list has to say what
                        its items are, which is what the `items` option
                        below does.

                        `tabindex="0"`, not `-1`: the default
                        `FOCUSABLE` excludes `[tabindex="-1"]`, so a row
                        marked that way is reachable programmatically
                        (which is why `focusFirstRowOnOpen` worked) and
                        invisible to the arrows. */}
                    <div
                      class="min-w-0 flex-1 outline-none focus-visible:ring-1 focus-visible:ring-(--color-outl-accent)"
                      data-trash-row
                      tabindex="0"
                      onKeyDown={(e) => {
                        // The row is the item, so the row carries the
                        // action: a keyboard user never has to reach the
                        // button, which is why it can leave the walk.
                        if (e.key !== "Enter" && e.key !== " ") return;
                        if (entry.refusal || busy() === entry.id) return;
                        e.preventDefault();
                        void restore(entry);
                      }}
                    >
                      <p class="truncate text-sm">
                        <Show when={entry.page} fallback={entry.text}>
                          {(slug) => <>page <code class="font-mono">{slug()}</code></>}
                        </Show>
                      </p>
                      <p class="mt-0.5 text-xs opacity-50">
                        {formatWhen(entry.deleted_at_ms)}
                        <Show when={entry.blocks > 1}>
                          {" · "}
                          {entry.blocks} blocks
                        </Show>
                      </p>
                      {/* Both printed verbatim: the backend owns the
                          whole sentence, "cannot restore" included. */}
                      <Show when={entry.refusal}>
                        {(why) => (
                          <p class="mt-1 text-xs text-(--color-outl-todo-open-fg)">{why()}</p>
                        )}
                      </Show>
                      <Show when={attemptNotice(entry)}>
                        {(why) => (
                          <p class="mt-1 text-xs text-(--color-outl-todo-open-fg)">{why()}</p>
                        )}
                      </Show>
                    </div>
                    <Show
                      when={!entry.refusal}
                      fallback={
                        <span class="shrink-0 pt-0.5 text-xs uppercase opacity-40">
                          Can't restore
                        </span>
                      }
                    >
                      <button
                        type="button"
                        // Out of the **tab** order only. It is out of the
                        // arrow walk because the list passes
                        // `items: "[data-trash-row]"`, not because of
                        // this attribute — `FOCUSABLE`'s `button` arm
                        // would match it either way. The mouse still
                        // clicks it and the row's Enter does the same.
                        tabindex="-1"
                        disabled={busy() === entry.id}
                        class="shrink-0 rounded border border-(--color-outl-border) px-2 py-1 text-xs hover:bg-(--color-outl-bg) disabled:opacity-50"
                        onClick={() => void restore(entry)}
                      >
                        {busy() === entry.id ? "Restoring…" : "Restore"}
                      </button>
                    </Show>
                  </div>
                )}
              </For>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
}
