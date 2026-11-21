import { Show } from "solid-js";

import { NamespaceMentions, NestedPages } from "@outl/shared/namespace";
import { FirstRunTips } from "@outl/shared/polish";

import { appState } from "../lib/store";

import { InlineBacklinks } from "./InlineBacklinks";

export interface PageSectionsProps {
  /** Open a page by slug — the host owns navigation (`handleRefClick`). */
  onOpenPage: (slug: string) => void;
  /** The page holds only empty blocks: show the first-run tips. */
  blank?: boolean;
  /** The outline is zoomed into a block, so none of this is on screen
   *  (see below). Passed in rather than read off the store, so this and
   *  the outline body cannot disagree about whether the page is
   *  zoomed — `zoomView` is the one owner of that. */
  zoomed?: boolean;
}

/**
 * Everything that renders **below** the outline, in order.
 *
 * Two sections that look alike and answer different questions, which
 * is the reason they sit together here rather than being nested in
 * each other:
 *
 * - **Backlinks** — blocks that point *at* this page. Toggled by
 *   `Cmd/Ctrl+Shift+B`, derived from referencing blocks.
 * - **Nested pages** — pages that live *under* this one's namespace
 *   (`os` → `os/linux`, `os/linux/debian`, issue #275). Deliberately
 *   **not** gated on `backlinksOpen`: folding the two toggles together
 *   would hide one when the user muted the other. Derived from page
 *   titles.
 *
 * Each section renders nothing when it has nothing to show, so this
 * component is free when a page has neither.
 *
 * **None of it renders while the outline is zoomed into a block.**
 * Every section here answers a question about the *page*, and under a
 * zoom the header is a block — so the page's backlinks sitting beneath
 * a block title read as that block's, which is a wrong answer and not
 * merely a distracting one. Zooming is a request for one subtree and
 * nothing else (Roam / Workflowy "focus"), which is the same reason
 * `<OutlineView />` hides the page's `key:: value` row there.
 * The matching half is `backlinkRowIds()` in `lib/cursor-region.ts`, the
 * one owner of "is this section reachable": neither `j` nor a mutating
 * chord may reach a section that is not drawn.
 */
export function PageSections(props: PageSectionsProps) {
  return (
    <Show when={!props.zoomed}>
      <Show when={props.blank}>
        <div class="mt-8">
          <FirstRunTips client="desktop" />
        </div>
      </Show>
      <InlineBacklinks />
      <NestedPages
        class="mt-6"
        children={appState.namespaceChildren}
        onOpen={props.onOpenPage}
      />
      <NamespaceMentions
        class="mt-6"
        backlinks={appState.namespaceMentions}
        total={appState.namespaceMentionsTotal}
        onOpen={props.onOpenPage}
      />
    </Show>
  );
}
