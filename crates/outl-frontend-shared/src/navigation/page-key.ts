/**
 * "Did the open page actually change?" — asked once, by this module.
 *
 * ## The bug this exists to stop
 *
 * Every client holds the open page as one object and replaces it on
 * **every reply**, mutations included: `applyPageState` on the desktop,
 * `applyView` on mobile. So `page` is a new `PageMeta` after each
 * created block, each deleted block, each committed edit — carrying the
 * same slug, the same title, the same id.
 *
 * A tracked source written straight off that object therefore fires on
 * every keystroke-commit, and **`on()` does not stop it**. Solid's `on`
 * makes a dependency *explicit*; it never compares the value it read
 * against the previous one. The shape
 *
 * ```ts
 * createEffect(on(() => appState.page?.slug, (slug) => …))  // ✗
 * ```
 *
 * reads as "once per navigation" and is not. It is how the desktop's
 * backlinks section came to unmount itself on every created or deleted
 * block — an `O(blocks-in-workspace)` scan, several times per edit, to
 * answer a question whose answer could not have changed — and how its
 * sidebar came to rewrite `localStorage` synchronously on every commit.
 * Neither had a failing test, because neither had a test: the question
 * was asked inline, in a component, where nothing could reach it.
 *
 * A `createMemo` does compare, so this is a memo with a name:
 *
 * ```ts
 * const slug = createPageKey(() => appState.page, (p) => p.slug);
 * createEffect(on(slug, (slug) => …));                      // ✓
 * ```
 *
 * `undefined` while no page is open, which also keeps a
 * `createResource` sourced from it idle.
 *
 * ## When **not** to reach for it
 *
 * When the effect must run on every reply and only *behave* differently
 * on a real change. The desktop's `installPageChangeEffect` is that
 * case: it has to re-run when the outline lands to repair a selection
 * that no longer exists, so it tracks the outline too and keeps its own
 * `lastPageId`. A key would have silenced the re-run it depends on.
 * The distinction is "should this run?" versus "what should this do?",
 * and only the first belongs here.
 *
 * `backlinks-key.ts` is the one application of this with a shape of its
 * own, because a backlinks reply depends on two fields rather than one.
 */
import { createMemo, type Accessor } from "solid-js";

/**
 * A field of the open page, as a source that fires only when that
 * field's **value** changes.
 *
 * `select` is never called with a null page: the result is `undefined`
 * while none is open, so a caller does not repeat that check.
 */
export function createPageKey<P extends object, K>(
  page: Accessor<P | null | undefined>,
  select: (page: P) => K,
  equals: (a: K | undefined, b: K | undefined) => boolean = Object.is,
): Accessor<K | undefined> {
  return createMemo(
    () => {
      const p = page();
      return p ? select(p) : undefined;
    },
    undefined,
    { equals },
  );
}
