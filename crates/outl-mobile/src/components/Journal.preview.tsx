/**
 * The page-first boot's half of the journal: today's text, read off
 * disk, painted where the skeleton used to be.
 *
 * Its own sibling for the reason every other `Journal.*` extraction has
 * one — `Journal.tsx` is the largest file in the repo and is being
 * broken up ([issue 265](https://github.com/outlmd/outl/issues/265)), so
 * new behaviour lands beside it rather than inside it.
 *
 * The split axis here is the same one `Journal.boot.ts` uses: this owns
 * a **state that exists only before the workspace does**. Nothing in it
 * can mutate anything, because there is nothing open to mutate.
 */
import { createSignal, onCleanup, Show, type JSX } from "solid-js";

import { onPagePreview, PreviewOutline } from "@outl/shared/boot";

import type { PageView } from "@outl/shared/api/types";
import { dismissBootSplash } from "../lib/splash";

/**
 * Subscribe to the backend's pre-workspace page preview and expose it.
 *
 * Call from the component body or `onMount`: it registers an
 * `onCleanup`, so it needs a Solid owner.
 *
 * **The handler is deliberately unguarded.** The render below is the
 * only reader, and it already sits inside the journal's load gate, so a
 * preview arriving after the workspace opened is simply never shown.
 * Reading that gate here instead would place a read outside the outline
 * section, which `Journal.load-gate.test.ts` pins against — a gate read
 * out in the component body is how chrome the user could already be
 * using ends up behind one command's reply.
 *
 * The listener handle arrives from a promise, so cleanup is armed
 * **synchronously** and disposes a late handle, the rule
 * `Journal.listeners.ts`'s `armCleanup` owns: registering cleanup inside
 * the `.then()` leaks the listener on a fast unmount, invisibly.
 */
function createJournalPreview(): () => PageView | null {
  const [preview, setPreview] = createSignal<PageView | null>(null);
  let unlisten: (() => void) | undefined;
  let cancelled = false;
  void onPagePreview((view) => {
    setPreview(view);
    // **The splash has to go, or the preview is invisible.**
    //
    // `#boot-splash` is `position: fixed` at `z-index: 2147483647` with
    // an opaque background, and it lives outside `#root`. Painting
    // today's journal underneath it delivers nothing: before this call
    // the preview sat behind the logo until the 4s failsafe animation
    // fired, which is every launch this feature was built for.
    //
    // Dismissing here rather than in `finishLoad` alone is the whole
    // point — `finishLoad` runs after the workspace opens, which is the
    // wait the preview exists to fill.
    dismissBootSplash();
  }).then((un) => {
    if (cancelled) un();
    else unlisten = un;
  });
  onCleanup(() => {
    cancelled = true;
    unlisten?.();
  });
  return preview;
}

interface JournalPreviewProps {
  /** Rendered when there is nothing to preview. */
  fallback: JSX.Element;
}

/**
 * Render the preview, or `fallback` when the launch has nothing to show
 * (a fresh install, or a day with no journal yet).
 *
 * It owns the subscription too, rather than taking the preview as a
 * prop: that keeps `Journal.tsx` to one line, and the lifetimes match —
 * this component is mounted exactly while the journal is unloaded,
 * which is exactly how long a preview is worth listening for.
 *
 * Dimmed, because it is read-only and the user should not try to type
 * into it. It is not disabled-looking beyond that: the content is real,
 * and this is the same text the workspace is about to re-project.
 */
export function JournalPreview(props: JournalPreviewProps): JSX.Element {
  const preview = createJournalPreview();
  return (
    <Show when={preview()} fallback={props.fallback}>
      {(p) => (
        <div class="px-3 opacity-80">
          <PreviewOutline outline={p().outline} />
        </div>
      )}
    </Show>
  );
}
