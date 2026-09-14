import { createSignal, For, Show } from "solid-js";

import type { Backlink, NamespaceChild } from "../api/types";

export interface NestedPagesProps {
  /** Rows from `PageBacklinks.namespace_children`, already title-sorted
   *  and depth-annotated by the backend. */
  children: NamespaceChild[];
  /** Open the page behind a row. The client owns navigation. */
  onOpen: (slug: string) => void;
  /** Extra classes for the outer `<section>`, so a client can place it
   *  in its own page chrome without this component knowing about it. */
  class?: string;
}

/**
 * The "Nested pages" section — every page under this one's namespace
 * (`os` → `os/linux`, `os/linux/debian`), issue #275.
 *
 * Lives in `@outl/shared` rather than in a client because it is the
 * pure half: a list, a label, a click. The hierarchy itself was
 * already decided in Rust (`outl_actions::namespace`), which is the
 * point — `depth` and `label` arrive computed, so this component never
 * splits a title on `/` and three clients cannot disagree about what
 * `OS/Linux` or `os//linux` means.
 *
 * Renders nothing when there are no nested pages: an empty section
 * does not earn its space, the same rule the backlinks section
 * follows.
 */
export function NestedPages(props: NestedPagesProps) {
  return (
    <Show when={props.children.length > 0}>
      <section class={props.class}>
        <div class="border-t border-(--color-outl-border) opacity-60" />

        <header class="mt-3 mb-2">
          <span class="text-xs font-semibold uppercase tracking-wide opacity-60">
            Nested pages · {props.children.length}
          </span>
        </header>

        <ul class="space-y-0.5">
          <For each={props.children}>
            {(child) => (
              <li
                // Indent by depth so the tree reads as a tree. Inline
                // rather than a Tailwind class because the depth is
                // data, and `pl-${n}` would need every level enumerated
                // in the safelist to survive a production build.
                style={{ "padding-left": `${(child.depth - 1) * 0.9}rem` }}
              >
                <button
                  type="button"
                  onClick={() => props.onOpen(child.page.slug)}
                  class="flex w-full items-baseline gap-2 rounded px-1 py-0.5 text-left text-sm hover:bg-(--color-outl-fg)/5"
                  title={child.page.title}
                >
                  {/* No journal branch: `descendants` filters journals
                      out, so every row here is a regular page. */}
                  <span aria-hidden="true" class="opacity-60">
                    {child.page.icon || "📄"}
                  </span>
                  <span>{child.label}</span>
                </button>
              </li>
            )}
          </For>
        </ul>
      </section>
    </Show>
  );
}

export interface NamespaceMentionsProps {
  /** Capped sample from `PageBacklinks.namespace_backlinks`. */
  backlinks: Backlink[];
  /** Real count before the cap, from `namespace_backlinks_total`. */
  total: number;
  /** Open the page a mention lives on. */
  onOpen: (slug: string) => void;
  class?: string;
}

/**
 * "Mentions under this namespace" — blocks that reached this page only
 * through a descendant (`#os/linux` arriving at `os`, issue #275).
 *
 * Its own collapsed section, not mixed into Backlinks, because the set
 * has no natural size: on a real workspace `buser` names 448 sources
 * and collects 3,221 more this way. Folding them together turns a
 * readable panel into an unreadable one, and these are a weaker claim —
 * nobody wrote them about *this* page.
 *
 * Collapsed by default and capped server-side, so the count carries the
 * scale and the payload does not. When the list is truncated the header
 * says so rather than implying it is complete.
 */
export function NamespaceMentions(props: NamespaceMentionsProps) {
  const [open, setOpen] = createSignal(false);
  const shown = () => props.backlinks.length;
  return (
    <Show when={props.total > 0}>
      <section class={props.class}>
        <button
          type="button"
          onClick={() => setOpen(!open())}
          class="flex w-full items-baseline gap-2 rounded px-1 py-0.5 text-left text-xs font-semibold uppercase tracking-wide opacity-60 hover:opacity-100"
        >
          <span aria-hidden="true">{open() ? "▾" : "▸"}</span>
          <span>
            Mentions under this namespace · {props.total}
            {shown() < props.total ? ` (showing ${shown()})` : ""}
          </span>
        </button>

        <Show when={open()}>
          <ul class="mt-1 space-y-1 pl-5">
            <For each={props.backlinks}>
              {(link) => (
                <li>
                  <button
                    type="button"
                    onClick={() => {
                      const slug = link.source_page?.slug;
                      if (slug) props.onOpen(slug);
                    }}
                    class="w-full rounded px-1 py-0.5 text-left text-sm hover:bg-(--color-outl-fg)/5"
                  >
                    <span class="opacity-50">
                      {link.source_page?.title ?? "(orphan)"} ·{" "}
                    </span>
                    <span>{link.source_block.text}</span>
                  </button>
                </li>
              )}
            </For>
          </ul>
        </Show>
      </section>
    </Show>
  );
}
