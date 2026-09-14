import { NamespaceMentions, NestedPages } from "@outl/shared/namespace";

import { appState } from "../lib/store";

import { InlineBacklinks } from "./InlineBacklinks";

export interface PageSectionsProps {
  /** Open a page by slug — the host owns navigation (`handleRefClick`). */
  onOpenPage: (slug: string) => void;
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
 */
export function PageSections(props: PageSectionsProps) {
  return (
    <>
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
    </>
  );
}
