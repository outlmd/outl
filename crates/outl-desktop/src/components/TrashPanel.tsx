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
 * A refusal can go stale between the list and the click; the rules for
 * that (and why the row's own verdict wins over an attempt's notice)
 * live in `@outl/shared/trust`'s `trash.ts`.
 */

import { For, Show } from "solid-js";

import type { TrashEntry } from "@outl/shared/api/trash";
import { createTrashRestore, deletedAgo } from "@outl/shared/trust";
import type { PageView } from "@outl/shared/api/types";

import { appState, setAppState } from "../lib/store";
import { focusFirstRowOnOpen, rovingKeydown } from "../lib/keyboard-reach";

export interface TrashPanelDeps {
  applyView: (view: PageView) => void;
  setError: (msg: string) => void;
}

export function TrashPanel(props: TrashPanelDeps) {
  // Gated on the panel being open, for the reason `TasksPanel` gives:
  // the component is mounted for the app's whole life.
  const t = createTrashRestore({
    open: () => appState.trashOpen,
    onView: (v) => props.applyView(v),
    onError: (m) => props.setError(m),
  });
  const data = t.rows;

  const close = () => setAppState("trashOpen", false);

  let list: HTMLDivElement | undefined;
  focusFirstRowOnOpen(() => appState.trashOpen, data, () => list, "[data-trash-row]");

  async function restore(entry: TrashEntry) {
    if (await t.restore(entry)) close();
  }

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
                        if (entry.refusal || t.busy() === entry.id) return;
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
                        {deletedAgo(entry.deleted_at_ms)}
                        <Show when={entry.blocks > 1}>
                          {" · "}
                          {entry.blocks} blocks
                        </Show>
                      </p>
                      {/* One line, because only one of the two can be
                          set: `notice` yields to the row's own
                          verdict. Printed verbatim either way — the
                          backend owns the whole sentence, "cannot
                          restore" included.

                          `||` rather than `??`, so a refusal that is
                          somehow the empty string still falls through
                          to the attempt, which is what the two
                          separate `Show`s did. */}
                      <Show when={entry.refusal || t.notice(entry)}>
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
                        disabled={t.busy() === entry.id}
                        class="shrink-0 rounded border border-(--color-outl-border) px-2 py-1 text-xs hover:bg-(--color-outl-bg) disabled:opacity-50"
                        onClick={() => void restore(entry)}
                      >
                        {t.busy() === entry.id ? "Restoring…" : "Restore"}
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
