/**
 * When the page's backlinks are fetched: **on navigation, and only on
 * navigation.**
 *
 * The decision is **not made here**. `createBacklinksKey`
 * (`@outl/shared/namespace`) is the one owner of "has the question
 * changed", and mobile's `createResource` already took it as its
 * source. This client wrote its own answer instead, and that is the
 * whole bug below.
 *
 * ## What the local copy got wrong
 *
 * The effect used to be `on(() => appState.page?.slug, …)`, carrying a
 * comment saying it fires once per navigation. It does not.
 * Solid's `on` makes a dependency *explicit*; it never compares the
 * value it read against the previous one, so the callback runs on every
 * invalidation of what `deps` touched.
 *
 * And every mutation invalidates it. `applyPageState` writes the page
 * as **one object** (`setAppState({ page, parseWarnings, … })`), which
 * `setState` merges key by key — so `page` is assigned a new `PageMeta`
 * and its node fires, same slug or not. (A *path* write,
 * `setAppState("page", …)`, merges into the object already there and
 * would not; that difference is why this never showed up in a store
 * test written the other way.)
 *
 * The result was a blink on every created or deleted block:
 * `emptyBacklinks()` ran, `lateSectionVisible` unmounted the whole
 * section, and it came back a workspace scan later with the same rows
 * it had before. The scan is `O(blocks-in-workspace)` — paid in full,
 * several times per edit, to answer a question whose answer could not
 * have changed (backlinks are *other* pages pointing here).
 *
 * The shared key is a memo with an `equals` of its own, so it compares
 * by value and the effect sees only a question that is genuinely
 * different. Taking it also fixes the half this client never had:
 * `namespace_children` hangs off `title::`, so renaming `os-linux` to
 * `os/linux` changes which pages nest under it while the slug stays
 * put — a slug-keyed fetch never re-asked.
 *
 * The TUI is the third answer to the same question and does not share
 * this code (no Solid, no fetch — it holds `backlink_index` and keeps
 * the old one live until the rebuild lands), but it holds the same
 * rule: the panel never blanks for work that changes nothing.
 */
import { createEffect, on } from "solid-js";

import { pageBacklinks } from "@outl/shared/api/commands";
import { createBacklinksKey } from "@outl/shared/namespace";

import { appState, emptyBacklinks, setAppState, trackBacklinks } from "./store";

export function installBacklinksEffect(): void {
  const key = createBacklinksKey(() => appState.page);

  createEffect(
    on(key, (current) => {
      if (!current) return;
      // Clear first: a failed fetch is swallowed, and the nested-page
      // rows are navigation buttons. Reached only when the question
      // actually changed now, which is the one moment page A's rows
      // under page B's title would be wrong.
      setAppState(emptyBacklinks());
      // Never awaited: `trackBacklinks` runs it behind the page, and
      // the only thing that waits is that section (`UX.md` → Never
      // block a keystroke).
      void trackBacklinks(pageBacklinks(current.slug));
    }),
  );
}
