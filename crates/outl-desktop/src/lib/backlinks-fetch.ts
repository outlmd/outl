/**
 * The page's backlinks as a **fetch**: which slices they fill, when
 * they are asked for, and how many times one burst is allowed to ask.
 *
 * Split out of `store.ts` rather than grown there — the same axis
 * `cursor-region.ts` was split on, and for the same reason: that file
 * is the app's state *shape*, and this is an async policy with four
 * rules of its own (coalesce, last answer wins, only the open page's
 * answer, and only an answer the sort direction did not move under).
 * `backlinks-effect.ts` is deliberately not that file either:
 * it owns **whether the question changed**, which is
 * `@outl/shared`'s `createBacklinksKey`, and this owns what happens
 * once somebody asks.
 *
 * It imports the store and the store imports nothing back, so the
 * dependency stays one-way.
 */

import { pageBacklinks } from "@outl/shared/api/commands";
import { backlinksKeyOf, sameBacklinksKey } from "@outl/shared/namespace";

import type { BacklinksKey } from "@outl/shared/namespace";
import type {
  Backlink,
  NamespaceChild,
  PageBacklinks,
} from "@outl/shared/api/types";

import { appState, setAppState } from "./store";

/**
 * Every backlinks-derived slice, emptied together.
 *
 * Navigation must clear all of them: the fetch's failure is swallowed,
 * and leaving page A's rows under page B is worse than an empty
 * section — the nested-page rows are navigation buttons.
 * `backlinksOrder` is a display preference, not page data, so it is
 * carried over rather than reset.
 *
 * **Only a navigation may call this.** It is what unmounts the section
 * (`lateSectionVisible` hides a section with no rows), so a *refetch*
 * that cleared first would make the section vanish and come back on
 * every edit — the defect commit `4817abd2` exists to fix.
 */
export function emptyBacklinks() {
  return {
    backlinks: [] as Backlink[],
    backlinksOrder: appState.backlinksOrder,
    namespaceChildren: [] as NamespaceChild[],
    namespaceMentions: [] as Backlink[],
    namespaceMentionsTotal: 0,
  };
}

/** The same slices, filled from a reply. One owner, so a new field
 *  cannot be wired into one call site and forgotten in the other. */
export function backlinksState(r: PageBacklinks) {
  return {
    backlinks: r.backlinks,
    backlinksOrder: r.backlinks_order,
    namespaceChildren: r.namespace_children,
    namespaceMentions: r.namespace_backlinks,
    namespaceMentionsTotal: r.namespace_backlinks_total,
  };
}

/**
 * The page asked for, and the chain answering it.
 *
 * `queued` is the **latest** question, overwritten rather than appended
 * to, because a burst's earlier answers are all superseded by its last
 * one. `draining` is the chain running them, so a second caller joins
 * it instead of starting a parallel scan.
 */
let queued: BacklinksKey | undefined;
let draining: Promise<void> | null = null;

/**
 * Re-ask the open page's backlinks — **once per answer, not once per
 * ask.**
 *
 * Every backlinks fetch goes through here, and none of them is awaited
 * on a render or a keystroke path: the scan is `O(blocks-in-workspace)`,
 * and the outline has to stay editable with the caret where the user
 * left it while it runs (`UX.md` → Never block a keystroke). The
 * pending flag it sets is read by `<InlineBacklinks />` alone, so the
 * one thing that waits is the section that has nothing to show yet.
 *
 * One owner for the five call sites (navigation, a peer refresh, the
 * source-task toggle, a text commit through a row, and the nine
 * cross-page chords) because the alternative — each flipping its own
 * flag around its own `.then` — is how a section ends up loading
 * forever on the one path that forgot the `catch`. A failure is
 * swallowed here for the same reason it always was: the rows are
 * navigation affordances, and stale ones are worse than none.
 *
 * ## Why the fetch is built here, and not handed in
 *
 * This used to take the promise (`trackBacklinks(pageBacklinks(slug))`),
 * which puts the IPC call on the wire *before* this function is
 * reached — so there was nothing left to coalesce, and no way to ask
 * for one. Every cross-page gesture commits with
 * `BacklinkRefresh::DropAll`, which drops the cached index, so each ask
 * reads and parses every `.md` in the workspace. A **held** `Alt+↑` or
 * `Tab` on a row is one of those per key repeat: latency was a wash
 * (30.0 / 34.1 ms per keystroke either way) and the two refresh
 * policies were ten whole index rebuilds of CPU apart
 * (`outl-tauri-shared/tests/backlink_commit_cost.rs` carries the
 * numbers, and why the policy is *not* the thing to change — switching
 * it pays 25 ms on the IPC thread for every single gesture to fix the
 * burst case).
 *
 * So the question is asked here and nowhere else, and taking **no
 * argument** is the point: a caller cannot name a page, so it cannot
 * ask for one the user is not on.
 *
 * ## What a burst costs, and what it may not break
 *
 * At most **one scan in flight plus one queued**, whatever the repeat
 * rate. The trailing ask is not an optimisation to drop: each repeat is
 * a real mutation, so the rows genuinely changed and the last answer
 * has to be fetched.
 *
 * - **The last answer wins.** One scan runs at a time, so replies
 *   cannot land out of order, and the queue holds the newest question
 *   rather than the oldest.
 * - **It must be the open page's answer.** `sameBacklinksKey` is asked
 *   against the key the reply was requested with, so a reply for a page
 *   the user has navigated away from — or whose `title::` changed under
 *   it, which is what `namespace_children` hangs off — is dropped
 *   instead of drawn. Stale rows are worse than slow rows, because
 *   nothing on screen says so. `@outl/shared/namespace` owns that
 *   comparison; a second opinion here is how the two drift.
 * - **It must be the direction the user currently wants.** The page key
 *   compares page identity, so it does not cover the sort. The backend
 *   reads `[display] backlinks_order` from `config.toml` *before* its
 *   whole-workspace index build (`commands/page_backlinks.rs`), and
 *   `set_backlinks_order` writes that key and then computes its own
 *   reply — so a scan that read the old direction and answers after the
 *   toggle's answer is a same-page reply sorted the other way round.
 *   Dropping only its `backlinks_order` would be the same lie one field
 *   along: the reply's **rows are sorted by the direction it reports**,
 *   so the new label would land over the old sort. The pair is refused
 *   together, exactly as a wrong-page reply is.
 *
 *   The question is *"did the direction move under this ask"*, not
 *   *"does this reply agree with the store"*. The second wedges: a
 *   `config.toml` edited by another process while the app runs makes
 *   every reply disagree, and the store never catches up, so the rows
 *   would freeze for the rest of the session. The first cannot leave
 *   the rows older than the drop's cause, because the only thing that
 *   moves the direction is the toggle, whose own reply has already
 *   landed — and it scanned *after* the ask being dropped.
 * - **The section does not blink.** `backlinksLoading` stays true for
 *   the whole chain and nothing here empties the rows, so
 *   `lateSectionVisible` keeps them up with the `loading…` mark. Only
 *   {@link emptyBacklinks} takes them away, and only a navigation calls
 *   it.
 *
 * The returned promise settles when the **queue** is empty, not when
 * the caller's own ask lands: a caller that awaits it is waiting for
 * the rows on screen to be final, which is the only useful answer.
 */
export function trackBacklinks(): Promise<void> {
  const key = backlinksKeyOf(appState.page);
  if (!key) return Promise.resolve();
  queued = key;
  // Queue before starting: the drain reads `queued` synchronously.
  draining ??= drainBacklinks();
  return draining;
}

/**
 * Run the queue until it is empty, one scan at a time.
 *
 * The pending flag is raised once for the whole chain and lowered once,
 * which is what keeps a burst from flickering the `loading…` mark on
 * and off; `finally` is what stops a rejection leaving the section
 * loading forever.
 */
async function drainBacklinks(): Promise<void> {
  setAppState("backlinksLoading", true);
  try {
    while (queued !== undefined) {
      const asked = queued;
      // Read here rather than at `trackBacklinks`: an ask queued behind
      // a running scan goes on the wire at *this* line, which is the
      // moment the backend reads `config.toml`.
      const askedOrder = appState.backlinksOrder;
      queued = undefined;
      try {
        const reply = await pageBacklinks(asked.slug);
        const stillWanted =
          sameBacklinksKey(asked, backlinksKeyOf(appState.page)) &&
          askedOrder === appState.backlinksOrder;
        if (stillWanted) {
          setAppState(backlinksState(reply));
        }
      } catch {
        // Swallowed, as it always was — see the note above.
      }
    }
  } finally {
    draining = null;
    setAppState("backlinksLoading", false);
  }
}
