import { JSX, Show } from "solid-js";

import type { PageBacklinks, PageMeta } from "@outl/shared/api/types";
import { NamespaceMentions, NestedPages } from "@outl/shared/namespace";
import { lateSectionVisible } from "@outl/shared/outline";
import { FirstRunTips } from "@outl/shared/polish";

import { BacklinksSection } from "./BacklinksSection";

export interface PageSectionsProps {
  /**
   * The lazy `page_backlinks` reply for the page on screen, or
   * `undefined` when there is no answer for it yet.
   *
   * `undefined` is the contract, and the host is what makes it true:
   * Solid's `createResource` keeps its last value through a source
   * change, so `Journal.tsx` blanks it while `loading` rather than
   * leaving the **previous** page's rows under the new page.
   */
  backlinks: PageBacklinks | undefined;
  /** Kind of the open page — decides whether an empty backlinks box shows. */
  pageKind: PageMeta["kind"] | undefined;
  /** Flip newest ⇄ oldest; the host persists it and swaps the reply in. */
  onToggleOrder: () => void;
  /** Navigate to a page. `kind` routes journals to the journal opener. */
  onOpenPage: (slug: string, kind: PageMeta["kind"]) => void;
  /** The page holds only empty blocks: show the first-run tips. */
  blank?: boolean;
}

/**
 * Everything that renders **below** the outline, in order — the mobile
 * twin of the desktop's `PageSections`.
 *
 * Two sections that look alike and answer different questions, which
 * is why they sit together here rather than one inside the other:
 *
 * - **Backlinks** — blocks that point *at* this page.
 * - **Nested pages** — pages that live *under* this one's namespace
 *   (`os` → `os/linux`, issue #275).
 *
 * The backlinks box renders even when empty on a regular page, so a
 * newcomer discovers that pages can cite each other; journals stay
 * hidden when empty, because the daily flow is busy enough without an
 * empty box every day. Nested pages render nothing when there are
 * none — there is no concept to discover in an empty list.
 *
 * **"Empty" is an answer, never a guess.** `lateSectionVisible`
 * (`@outl/shared/outline`) owns that rule and the measurement behind
 * it; this is the surface it was written for. Until the scan replies,
 * every section here draws nothing — the box that said "No backlinks
 * yet" over a scan still running was telling the user something it
 * then took back.
 */
export function PageSections(props: PageSectionsProps): JSX.Element {
  return (
    <>
      <Show when={props.blank}>
        <div class="px-5 pt-8">
          <FirstRunTips client="mobile" />
        </div>
      </Show>
      <Show
        when={lateSectionVisible({
          answered: props.backlinks !== undefined,
          count: props.backlinks?.backlinks.length ?? 0,
          // A regular page keeps the discoverability box once the scan
          // says zero; a journal does not.
          keepWhenEmpty: props.pageKind === "page",
        })}
      >
        <BacklinksSection
          backlinks={props.backlinks?.backlinks ?? []}
          order={props.backlinks?.backlinks_order ?? "newest"}
          onToggleOrder={props.onToggleOrder}
          onJump={(link) => {
            const source = link.source_page;
            if (source) props.onOpenPage(source.slug, source.kind);
          }}
        />
      </Show>

      <NestedPages
        class="px-5 pt-4"
        children={props.backlinks?.namespace_children ?? []}
        // Always a regular page: `outl_actions::namespace::descendants`
        // filters journals out, pinned by `a_journal_is_never_nested`.
        onOpen={(slug) => props.onOpenPage(slug, "page")}
      />

      <NamespaceMentions
        class="px-5 pt-4"
        backlinks={props.backlinks?.namespace_backlinks ?? []}
        total={props.backlinks?.namespace_backlinks_total ?? 0}
        onOpen={(slug) => props.onOpenPage(slug, "page")}
      />
    </>
  );
}
