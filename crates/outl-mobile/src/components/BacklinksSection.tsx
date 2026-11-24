import { For, JSX, Show, createEffect, createMemo, onCleanup } from "solid-js";
import type {
  Backlink,
  BacklinksOrder,
  PageKind,
  TodoState,
} from "@outl/shared/api/types";
import {
  type BacklinkRow,
  backlinkEditTargetFor,
  flattenBacklinkRows,
} from "@outl/shared/backlink-edit";
import {
  CrumbTrail,
  type EmbedMap,
  MarkdownInline,
  MarkdownTable,
  type PageEmbedMap,
} from "@outl/shared/markdown";
import { rawTextWithTodo, sameCrumbTrail } from "@outl/shared/outline";
import { PageGlyph } from "@outl/shared/icons";
import { createLongPress } from "../lib/long-press";
import { BacklinkRowEditor } from "./BacklinkRowEditor";
import { BlockContextMenu } from "./BlockContextMenu";
import { BlockDeleteConfirm } from "./JournalDeleteDialogs";
import { createEmbedResolver } from "./Journal.embeds";
import {
  type BacklinkCommit,
  createBacklinkRowOps,
} from "./BacklinksSection.row-ops";
import { buildBacklinkContextActions } from "./Journal.context-actions";

interface BacklinksSectionProps {
  backlinks: Backlink[];
  onJump: (link: Backlink) => void;
  /** Current sort direction (issue #142). */
  order: BacklinksOrder;
  /** Flip newest ⇄ oldest — the host persists it + applies the re-sorted view. */
  onToggleOrder: () => void | Promise<void>;
  /**
   * A mutation through one of these rows landed, on the page the citing
   * block lives on.
   *
   * The section reports the fact and decides nothing about it: whether
   * the reply may be installed as the open page's view is
   * `backlinkCommitPlan`'s answer, and the only place holding both page
   * ids is the host (`Journal.backlinks.ts`). That is also why the
   * section does not take the open page's id.
   *
   * **Resolving late is the contract.** The re-ask is an
   * O(blocks-in-workspace) scan (invariant 15) and the editor stays
   * open until this settles, so a committed row never renders the
   * pre-edit text it still holds.
   */
  onEditCommitted: (commit: BacklinkCommit) => void | Promise<void>;
  /**
   * A refusal the user has to read. Nothing was written and retrying
   * the same id cannot succeed, so this is a message, never a Retry.
   */
  onError: (message: string) => void;
}

/**
 * Backlinks panel rendered below the outline. References are grouped
 * by their source page (parity with desktop + TUI): one card per
 * source page, its title as the card header, then one row per
 * referencing block. A row buried in a nested outline shows its
 * ancestor breadcrumb as dimmed context; consecutive rows in the same
 * branch collapse the trail (shown once, then implied).
 *
 * **A row is a place you can work, not just a destination.** Tapping it
 * opens the client's own editor over the citing block; a long press
 * opens the outline's own `<BlockContextMenu>` with the structural ops,
 * because mobile has no chord surface at all. Either way the mutation
 * is addressed to that block's **source** page. The card header is what
 * still opens the page, and the checkbox toggles the task there.
 *
 * None of that is decided here. What a gesture targets is
 * `@outl/shared/backlink-edit`'s, what each op does is
 * `./BacklinksSection.row-ops`'s (including the one-write-at-a-time
 * rule), which rows the sheet offers is `buildBacklinkContextActions`'s,
 * and what a reply may touch is the host's. This file is the chrome:
 * the grouping, the breadcrumb, the row, and where the sheet hangs.
 *
 * Renders even when the list is empty so newcomers discover the
 * bidirectional-linking feature exists. Without the empty state,
 * a freshly-created page looks identical to a page that has no
 * graph at all — and the user has no idea pages CAN cite each
 * other until they happen to land on one that already has refs.
 */
export function BacklinksSection(props: BacklinksSectionProps): JSX.Element {
  // Every write, every piece of state and the one-write-at-a-time rule
  // live in the sibling module — see `./BacklinksSection.row-ops`.
  // Nothing about *which* block on *which* page is decided here.
  const ops = createBacklinkRowOps({
    onEditCommitted: (commit) => props.onEditCommitted(commit),
    onError: (message) => props.onError(message),
  });

  // One flattening per fetch, never per render (invariant 15). `byLink`
  // hands each card entry its own rows; `allRows` is what the delete
  // confirmation resolves against, across subtrees.
  const byLink = createMemo(() => {
    const m = new Map<Backlink, BacklinkRow[]>();
    for (const l of props.backlinks) m.set(l, flattenBacklinkRows([l]));
    return m;
  });
  const allRows = () => [...byLink().values()].flat();

  // These rows are other pages' blocks, so the open page's embed map
  // (`Journal.tsx`) never holds their handles: a child that is only
  // `!((blk-…))` rendered the raw handle. The panel resolves its own, in
  // its own resolver, so neither map can replace the other. The "slug"
  // is the set of rows, which is what changes the question being asked.
  const embedResolver = createEmbedResolver(() =>
    props.backlinks.map((l) => l.block_id).join(","),
  );
  createEffect(() => {
    const roots = props.backlinks.map((l) => l.source_block);
    if (roots.length > 0) embedResolver.resolve(roots);
  });

  /**
   * Group backlinks by their source page. `null` source (orphan
   * blocks with no enclosing page) collapse under a synthetic key.
   * Mirrors the desktop's `groupedBySource`.
   */
  function groupedBySource(): Array<{
    key: string;
    title: string;
    icon?: string;
    kind?: PageKind;
    entries: Backlink[];
  }> {
    const groups = new Map<
      string,
      { key: string; title: string; icon?: string; kind?: PageKind; entries: Backlink[] }
    >();
    for (const b of props.backlinks) {
      const key = b.source_page?.slug ?? "__orphan__";
      const title = b.source_page?.title ?? "untitled";
      const icon = b.source_page?.icon;
      const kind = b.source_page?.kind;
      const existing = groups.get(key);
      if (existing) existing.entries.push(b);
      else groups.set(key, { key, title, icon, kind, entries: [b] });
    }
    return [...groups.values()];
  }

  return (
    <section class="mx-3 mt-6">
      <header class="mb-2 flex items-center gap-2 px-2 text-(--color-outl-fg-dim)">
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
            // The card takes one write at a time, and this is a write:
            // a reorder lands a second `PageBacklinks` into the same
            // rows a commit's re-ask is about to replace. Guarded here
            // rather than in the host because the host does not hold
            // `pending` and this component holds both it and the
            // handler — recording an exception to a rule this card
            // states is worse than honouring it.
            onClick={() => void ops.runReorder(props.onToggleOrder)}
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
          <div class="overflow-hidden rounded-2xl bg-(--color-outl-bg-elev) px-4 py-5 text-center">
            <p class="text-[13px] text-(--color-outl-fg-dim)">
              No backlinks yet.
            </p>
            <p class="mt-1 text-[12px] text-(--color-outl-fg-dimmer)">
              Pages that link here with{" "}
              <code class="font-mono text-(--color-outl-accent)">
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
              <div class="overflow-hidden rounded-2xl bg-(--color-outl-bg-elev)">
                {/* Card header: source page. Tapping it jumps to the
                    page (via the group's first referencing block) —
                    the one gesture here that still navigates. */}
                <button
                  type="button"
                  data-backlink-group={group.key}
                  onClick={() => props.onJump(group.entries[0])}
                  class="flex w-full items-center gap-2 px-4 pt-3 pb-1 text-left active:opacity-60"
                >
                  <PageGlyph icon={group.icon} kind={group.kind} class="self-center" />
                  <span class="flex-1 text-[13px] font-medium text-(--color-outl-accent)">
                    {group.title}
                  </span>
                  <span class="text-[12px] text-(--color-outl-fg-dimmer)">
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
                    const rows = () => byLink().get(link) ?? [];
                    return (
                      <>
                        <For each={rows()}>
                          {(row) => (
                            <BacklinkRowView
                              row={row}
                              ops={ops}
                              embeds={embedResolver.embeds()}
                              pageEmbeds={embedResolver.pageEmbeds()}
                              bordered={idx() > 0 && row.depth === 0}
                              crumbs={row.depth === 0 && showCrumbs}
                            />
                          )}
                        </For>
                      </>
                    );
                  }}
                </For>
              </div>
            )}
          </For>
        </div>
      </Show>

      {/* Mounted only while open, unlike the outline's, so a test (and
          a screen reader) can tell "no sheet" from "an empty sheet" —
          which is exactly what an orphan row offers. */}
      <Show when={ops.menuRow()} keyed>
        {(row) => (
          <div data-backlink-menu>
            <BlockContextMenu
              open
              onClose={ops.closeMenu}
              actions={buildBacklinkContextActions(row.link, row.node, {
                toggleTodo: () => void ops.toggleRowTodo(row),
                createBelow: () => void ops.createBelow(row),
                createAbove: () => void ops.createAbove(row),
                indent: () => void ops.indent(row),
                outdent: () => void ops.outdent(row),
                moveUp: () => void ops.moveUp(row),
                moveDown: () => void ops.moveDown(row),
                delete: () => void ops.requestDelete(row),
              })}
            />
          </div>
        )}
      </Show>

      {/* The same prompt the outline raises, and the same wording — the
          case for it is stronger here, since the subtree about to go
          with the block is not on this screen at all. */}
      <BlockDeleteConfirm
        pending={ops.deletePrompt()}
        onCancel={ops.cancelDelete}
        onConfirm={(id) => {
          // Resolved against the live rows, not held from when the
          // prompt opened: the dialog closes itself before confirming,
          // and a sync may have replaced the rows meanwhile.
          const row = allRows().find((r) => r.key === id);
          if (row) void ops.deleteRow(row);
        }}
      />
    </section>
  );
}

type RowOps = ReturnType<typeof createBacklinkRowOps>;

/**
 * One drawn row: a citing block (depth 0) or a descendant shipped with
 * it. Every piece of per-row state is keyed on `row.key`, never on the
 * block id — a block that cites the page and also sits under another
 * citing block is drawn twice, and two rows must not share an editor.
 *
 * A child is its own tap and long-press target, a sibling of the
 * parent's button rather than inside it, so a hold on a child can never
 * reach the citing row's recogniser.
 */
function BacklinkRowView(props: {
  row: BacklinkRow;
  ops: RowOps;
  embeds: EmbedMap;
  pageEmbeds: PageEmbedMap;
  bordered: boolean;
  crumbs: boolean;
}): JSX.Element {
  // Touch has no right-click, so the hold is how this row reaches the
  // structural ops — mobile has no chord surface at all.
  const press = createLongPress({
    onLongPress: () => props.ops.openMenu(props.row),
    disabled: () => props.ops.editing() === props.row.key,
  });
  // Every `page_backlinks` reply hands `<For>` fresh objects, so a peer
  // edit landing under a held finger disposes this row — and the hold
  // would then open a sheet on a row that is no longer on screen.
  onCleanup(press.cancel);
  const busy = () => props.ops.pending() === props.row.key;
  const editing = () => props.ops.editing() === props.row.key;
  const todo = () =>
    props.row.depth === 0 ? props.row.link.todo : props.row.node.todo;
  const editable = () =>
    backlinkEditTargetFor(props.row.link, props.row.node) !== null;
  return (
    <div
      class="px-4 py-3"
      style={
        props.row.depth > 0
          ? { "padding-left": `${16 + props.row.depth * 16}px` }
          : undefined
      }
      // A cross-page write plus an O(blocks-in-workspace) re-ask is long
      // enough that a row which looks idle reads as a tap that did
      // nothing, so the acting row says so. `pointer-events-none` closes
      // the doors back into *this* row. It is the acting row only: `ops`
      // refuses a gesture on every other row for the same window and
      // nothing here shows that. Recorded rather than claimed fixed.
      data-backlink-pending={busy() ? "true" : undefined}
      aria-busy={busy() ? "true" : undefined}
      data-backlink-depth={props.row.depth > 0 ? props.row.depth : undefined}
      classList={{
        "border-t border-(--color-outl-border)/40": props.bordered,
        "pointer-events-none opacity-60": busy(),
      }}
    >
      <Show when={props.crumbs}>
        <p class="mb-1 truncate text-[12px] text-(--color-outl-fg-dimmer)">
          <CrumbTrail crumbs={props.row.link.ancestors} />
        </p>
      </Show>
      <div class="flex items-start gap-2">
        <Show when={todo() !== null}>
          <BacklinkCheckbox
            todo={todo()}
            // No tap while this row is being edited: the task prefix is
            // part of the draft there, so a toggle would race the commit.
            onToggle={
              editing() || !editable()
                ? undefined
                : () => void props.ops.toggleRowTodo(props.row)
            }
          />
        </Show>
        <Show
          when={editing()}
          fallback={
            <button
              type="button"
              data-backlink-row={props.row.node.id}
              data-backlink-key={props.row.key}
              onPointerDown={press.onPointerDown}
              onPointerMove={press.onPointerMove}
              onPointerUp={press.onPointerUp}
              onPointerCancel={press.onPointerUp}
              onClick={() => {
                // A completed hold produces a click too; without this
                // every long press would also drop into Insert.
                if (press.consumedClick()) return;
                props.ops.openEditor(props.row);
              }}
              // `-my-3 py-3` gives the tap the row's full height back
              // without moving the text the padding positions; the
              // floor keeps a one-line child at 44pt.
              class="-my-3 min-h-11 flex-1 py-3 text-left active:opacity-60"
            >
              <p
                class="text-[15px] leading-snug"
                classList={{
                  "text-(--color-outl-fg-dimmer) line-through":
                    todo() === "DONE",
                }}
              >
                <Show
                  when={props.row.node.table}
                  fallback={
                    <MarkdownInline
                      tokens={props.row.node.tokens}
                      variant="inline"
                      embeds={props.embeds}
                      pageEmbeds={props.pageEmbeds}
                    />
                  }
                >
                  <MarkdownTable table={props.row.node.table!} />
                </Show>
              </p>
            </button>
          }
        >
          <BacklinkRowEditor
            text={rawTextWithTodo(props.row.node)}
            onCommit={(text) => void props.ops.commitRow(props.row, text)}
          />
        </Show>
      </div>
    </div>
  );
}

/**
 * Checkbox marker for a backlink whose source block carries a
 * TODO/DONE state. Mirrors the visual treatment used by
 * `BulletOrCheckbox` in `BlockRow.tsx`, and it **toggles the task on
 * the source page** — the user no longer has to open that page to tick
 * a box they are already looking at.
 *
 * Without `onToggle` it is the marker alone, which is the honest render
 * for a row that has no page to address a mutation to (an orphan
 * backlink) and for the row being edited, where the task prefix is part
 * of the draft.
 */
function BacklinkCheckbox(props: {
  todo: TodoState | null;
  onToggle?: () => void;
}): JSX.Element {
  return (
    <Show when={props.onToggle} fallback={<CheckboxGlyph todo={props.todo} />}>
      <button
        type="button"
        data-backlink-todo="true"
        aria-label="Toggle task"
        onClick={() => props.onToggle?.()}
        // Padding for the finger, pulled back by the margin so the
        // glyph keeps the baseline it shares with the text beside it.
        class="-m-2 shrink-0 p-2 active:opacity-60"
      >
        <CheckboxGlyph todo={props.todo} />
      </button>
    </Show>
  );
}

/** The marker itself: three states, no behaviour. */
function CheckboxGlyph(props: { todo: TodoState | null }): JSX.Element {
  return (
    <span
      aria-hidden="true"
      class="mt-[3px] flex h-[18px] w-[18px] shrink-0 items-center justify-center rounded-full border-[1.5px]"
      classList={{
        "border-(--color-outl-accent) bg-(--color-outl-accent)":
          props.todo === "DONE",
        // Same three-state vocabulary as `BulletOrCheckbox`.
        "border-(--color-outl-accent) bg-transparent":
          props.todo === "DOING",
        "border-(--color-outl-fg-dim) bg-transparent":
          props.todo !== "DONE" && props.todo !== "DOING",
      }}
    >
      <Show when={props.todo === "DOING"}>
        <span class="h-[8px] w-[8px] rounded-full bg-(--color-outl-accent)" />
      </Show>
      <Show when={props.todo === "DONE"}>
        <svg
          width="10"
          height="10"
          viewBox="0 0 24 24"
          fill="none"
          stroke="var(--color-outl-accent-fg)"
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
