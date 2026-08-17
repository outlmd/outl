import { For, JSX, Show } from "solid-js";
import type {
  Backlink,
  BacklinksOrder,
  TodoState,
} from "@outl/shared/api/types";
import { MarkdownInline } from "@outl/shared/markdown";
import { sameCrumbTrail } from "@outl/shared/outline";

interface BacklinksSectionProps {
  backlinks: Backlink[];
  onJump: (link: Backlink) => void;
  /** Current sort direction (issue #142). */
  order: BacklinksOrder;
  /** Flip newest ⇄ oldest — the host persists it + applies the re-sorted view. */
  onToggleOrder: () => void;
}

/**
 * Backlinks panel rendered below the outline. References are grouped
 * by their source page (parity with desktop + TUI): one card per
 * source page, its title as the card header, then one row per
 * referencing block. A row buried in a nested outline shows its
 * ancestor breadcrumb as dimmed context; consecutive rows in the same
 * branch collapse the trail (shown once, then implied). Tapping a row
 * — or the card header — jumps to the source page.
 *
 * Renders even when the list is empty so newcomers discover the
 * bidirectional-linking feature exists. Without the empty state,
 * a freshly-created page looks identical to a page that has no
 * graph at all — and the user has no idea pages CAN cite each
 * other until they happen to land on one that already has refs.
 */
export function BacklinksSection(props: BacklinksSectionProps): JSX.Element {
  /**
   * Group backlinks by their source page. `null` source (orphan
   * blocks with no enclosing page) collapse under a synthetic key.
   * Mirrors the desktop's `groupedBySource`.
   */
  function groupedBySource(): Array<{
    key: string;
    title: string;
    icon: string;
    entries: Backlink[];
  }> {
    const groups = new Map<
      string,
      { key: string; title: string; icon: string; entries: Backlink[] }
    >();
    for (const b of props.backlinks) {
      const key = b.source_page?.slug ?? "__orphan__";
      const title = b.source_page?.title ?? "untitled";
      const fallback = b.source_page?.kind === "journal" ? "📅" : "📄";
      const icon = b.source_page?.icon || fallback;
      const existing = groups.get(key);
      if (existing) existing.entries.push(b);
      else groups.set(key, { key, title, icon, entries: [b] });
    }
    return [...groups.values()];
  }

  return (
    <section class="mx-3 mt-6">
      <header class="mb-2 flex items-center gap-2 px-2 text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
        <svg
          width="14"
          height="14"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          stroke-linejoin="round"
          aria-hidden="true"
        >
          <path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71" />
          <path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71" />
        </svg>
        <p class="text-[12px] font-medium uppercase tracking-wider">
          Linked from {props.backlinks.length}
        </p>
        {/* Direction toggle (issue #142): ↓ newest-on-top, ↑ oldest. */}
        <Show when={props.backlinks.length > 0}>
          <button
            type="button"
            onClick={() => props.onToggleOrder()}
            class="ml-auto flex items-center gap-1 rounded-full px-2 py-0.5 text-[12px] font-medium active:opacity-60"
          >
            <span aria-hidden="true">
              {props.order === "newest" ? "↓" : "↑"}
            </span>
            <span>{props.order === "newest" ? "Newest" : "Oldest"}</span>
          </button>
        </Show>
      </header>
      <Show
        when={props.backlinks.length > 0}
        fallback={
          <div class="overflow-hidden rounded-2xl bg-(--color-ios-card) px-4 py-5 text-center dark:bg-(--color-iosd-card)">
            <p class="text-[13px] text-(--color-ios-text-secondary) dark:text-(--color-iosd-text-secondary)">
              No backlinks yet.
            </p>
            <p class="mt-1 text-[12px] text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)">
              Pages that link here with{" "}
              <code class="font-mono text-(--color-ios-accent) dark:text-(--color-iosd-accent)">
                [[this page]]
              </code>{" "}
              will appear in this section.
            </p>
          </div>
        }
      >
        <div class="space-y-3">
          <For each={groupedBySource()}>
            {(group) => (
              <div class="overflow-hidden rounded-2xl bg-(--color-ios-card) dark:bg-(--color-iosd-card)">
                {/* Card header: source page. Tapping it jumps to the
                    page (via the group's first referencing block). */}
                <button
                  type="button"
                  onClick={() => props.onJump(group.entries[0])}
                  class="flex w-full items-center gap-2 px-4 pt-3 pb-1 text-left active:opacity-60"
                >
                  <span aria-hidden="true">{group.icon}</span>
                  <span class="flex-1 text-[13px] font-medium text-(--color-ios-accent) dark:text-(--color-iosd-accent)">
                    {group.title}
                  </span>
                  <span class="text-[12px] text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)">
                    {group.entries.length}
                  </span>
                </button>
                <For each={group.entries}>
                  {(link, idx) => {
                    // Breadcrumb of ancestor blocks as dimmed context,
                    // collapsed against the previous row in the same
                    // branch (shown once, then implied).
                    const prev =
                      idx() > 0 ? group.entries[idx() - 1] : null;
                    const showCrumbs =
                      link.ancestors.length > 0 &&
                      (!prev ||
                        !sameCrumbTrail(prev.ancestors, link.ancestors));
                    const crumbTrail = link.ancestors
                      .map((c) => c.text)
                      .join(" › ");
                    return (
                      <button
                        type="button"
                        onClick={() => props.onJump(link)}
                        class="block w-full text-left active:opacity-60"
                        classList={{
                          "border-t border-(--color-ios-divider)/40 dark:border-(--color-iosd-divider)/40":
                            idx() > 0,
                        }}
                      >
                        <div class="px-4 py-3">
                          <Show when={showCrumbs}>
                            <p class="mb-1 truncate text-[12px] text-(--color-ios-text-tertiary) dark:text-(--color-iosd-text-tertiary)">
                              {crumbTrail}
                            </p>
                          </Show>
                          <div class="flex items-start gap-2">
                            <Show when={link.todo !== null}>
                              <BacklinkCheckbox todo={link.todo} />
                            </Show>
                            <p
                              class="flex-1 text-[15px] leading-snug"
                              classList={{
                                "text-(--color-ios-text-tertiary) line-through dark:text-(--color-iosd-text-tertiary)":
                                  link.todo === "DONE",
                              }}
                            >
                              <MarkdownInline
                                tokens={link.source_block.tokens}
                              />
                            </p>
                          </div>
                        </div>
                      </button>
                    );
                  }}
                </For>
              </div>
            )}
          </For>
        </div>
      </Show>
    </section>
  );
}

/**
 * Read-only checkbox marker for a backlink whose source block carries
 * a TODO/DONE state. Mirrors the visual treatment used by
 * `BulletOrCheckbox` in `BlockRow.tsx`, but is presentational only —
 * to toggle the task the user opens the source page.
 */
function BacklinkCheckbox(props: { todo: TodoState | null }): JSX.Element {
  return (
    <span
      aria-hidden="true"
      class="mt-[3px] flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full border-[1.5px]"
      classList={{
        "border-(--color-ios-accent) bg-(--color-ios-accent) dark:border-(--color-iosd-accent) dark:bg-(--color-iosd-accent)":
          props.todo === "DONE",
        // Same three-state vocabulary as `BulletOrCheckbox`.
        "border-(--color-ios-accent) bg-transparent dark:border-(--color-iosd-accent)":
          props.todo === "DOING",
        "border-(--color-ios-text-secondary) bg-transparent dark:border-(--color-iosd-text-secondary)":
          props.todo !== "DONE" && props.todo !== "DOING",
      }}
    >
      <Show when={props.todo === "DOING"}>
        <span class="h-[8px] w-[8px] rounded-full bg-(--color-ios-accent) dark:bg-(--color-iosd-accent)" />
      </Show>
      <Show when={props.todo === "DONE"}>
        <svg
          width="10"
          height="10"
          viewBox="0 0 24 24"
          fill="none"
          stroke="white"
          stroke-width="3.5"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <path d="M5 12l4 4 10-10" />
        </svg>
      </Show>
    </span>
  );
}
