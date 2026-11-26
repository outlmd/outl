import { createMemo, For, Show } from "solid-js";

import { backlinkEditTargetFor } from "@outl/shared/backlink-edit";
import {
  CrumbTrail,
  crumbTrailText,
  isBlockQuoted,
  QuoteWrap,
} from "@outl/shared/markdown";
import type { Backlink, BlockNode } from "@outl/shared/api/types";

import { backlinkRowByKey, startBacklinkEdit } from "../lib/cursor-region";
import { appState } from "../lib/store";
import { BacklinkRowEditor } from "./BacklinkRowEditor";
import type { BodyCallbacks } from "./block-callbacks";
import { BlockBody } from "./BlockBody";
import { FoldChevron } from "./FoldChevron";

/** What one click on a backlink's bullet does next. The toggle walks
 *  the cycle a step at a time, so the label names the *next* stop,
 *  not "done" from wherever it is. */
function backlinkTodoLabel(todo: Backlink["todo"]): string {
  if (todo === "DONE") return "Mark not done";
  if (todo === "DOING") return "Mark done";
  if (todo === "TODO") return "Mark doing";
  return "Mark as TODO";
}

interface RowProps {
  /** `BacklinkRow.key` — unique per drawn row, **never** the block id: a
   *  block that cites the page and sits under another citing block is
   *  drawn twice, and one editor must not open in both places. */
  rowKey: string;
  /** The memoised keyboard cursor (`createMemo(backlinkCursor)`), shared
   *  by every row so a render pays one page walk, not one per row. */
  cursor: () => string | null;
  showCrumbs: boolean;
  onToggle: (link: Backlink, node: BlockNode) => void;
  /** The chevron's click: fold or unfold `node` **on its own page**, so
   *  the fold syncs. `collapse` is the state asked for, never a toggle. */
  onFold: (link: Backlink, node: BlockNode, collapse: boolean) => void;
  /** What a click inside the rendered block does — run a fence, follow
   *  a ref. Built per citing link, because a Run is addressed to the
   *  page the block lives on, not to the page on screen. */
  bodyCallbacks: (link: Backlink) => BodyCallbacks;
  /** The breadcrumb line was activated: open the citing block's page at
   *  its context (`openBacklinkContext`). Only offered when the row has a
   *  `source_page`; an orphan has no page to open. */
  onCrumbClick: (link: Backlink) => void;
}

/**
 * One drawn row of the "Linked from" panel: the citing block or a
 * descendant shown under it.
 *
 * **Looked up by key, not handed an object.** The parent's `<For>`
 * iterates row *keys* (strings), so a refetch — which replaces every
 * object — keeps this component, and with it an open editor and its
 * draft, mounted. Only the text a row reads moves. That is what lets a
 * background refetch land while the user types in a child without
 * re-seeding what they typed.
 */
function RowBody(props: RowProps) {
  const row = () => backlinkRowByKey(props.rowKey);
  return (
    <Show when={row()}>
      {(r) => {
        const link = () => r().link;
        // One callbacks object per row, shared by the crumb's ref and tag
        // handlers: the factory allocates a closure set per call.
        const crumbCallbacks = createMemo(() => props.bodyCallbacks(link()));
        const node = () => r().node;
        const todo = () => (r().depth === 0 ? link().todo : node().todo);
        const selected = () => props.cursor() === props.rowKey;
        // `null` for an orphan: the row stays read-only.
        const editTarget = () => backlinkEditTargetFor(link(), node());
        const editing = () => {
          const ed = appState.editingBacklink;
          return ed !== null && (ed.key ?? ed.blockId) === props.rowKey
            ? editTarget()
            : null;
        };
        const startEdit = () => {
          const target = editTarget();
          if (target) startBacklinkEdit(target, props.rowKey);
        };
        // **The outline's own renderer**, not a copy of it: fences (and
        // their Run), tables, quotes, prose with line breaks, images,
        // properties and both embed expansions are drawn by the same
        // `<BlockBody />` the page uses. `BacklinkRow.parity.test.ts`
        // fails if a second renderer comes back.
        //
        // Keyed on the node's **text**, not on the node object.
        // `<BlockBody />` decides its shape once, at mount, so a body
        // whose text changed (a Run, a peer edit) has to remount. But a
        // refetch replaces every node, and keying on identity remounted
        // every row on every refetch: each image re-read its asset over
        // IPC and blinked through its loading chip, each `call:` fence
        // asked `listTemplates` again. The shape is a function of the
        // text (tokens and table are parsed from it), and everything
        // else the body reads (properties, embeds) goes through the
        // live `node()` getter. A click edits in place, on the page the
        // block lives on; the source page stays on the group header.
        // No `onMouseEnter`: a pointer crossing the section is not a
        // cursor move.
        const rowText = () => (
          <QuoteWrap
            quoted={isBlockQuoted(node().text)}
            baseClass={`flex min-w-0 flex-1 rounded px-1 py-0.5 text-sm leading-snug hover:bg-(--color-outl-fg)/5 ${
              todo() === "DONE" ? "line-through opacity-60" : ""
            }`}
            chromeClass="rounded-r-md border-l-2 border-(--color-outl-fg-dimmer)/50 bg-(--color-outl-fg-dimmer)/[0.06] pl-2"
          >
            <div data-backlink-body class="min-w-0 flex-1">
              {/* The parameter is load-bearing: `<Show keyed>` only
                  remounts a child function that declares one. */}
              <Show when={`${node().id}\u0000${node().text}`} keyed>
                {(_shape) => (
                  <BlockBody
                    block={node()}
                    cb={props.bodyCallbacks(link())}
                    onStartEdit={startEdit}
                  />
                )}
              </Show>
            </div>
          </QuoteWrap>
        );
        return (
          <div
            data-backlink-row={props.rowKey}
            // The selected state mirrors the outline's BlockRow
            // highlight: 3px accent bar on the left + 6% background.
            class={
              selected()
                ? "relative -ml-3 rounded bg-(--color-outl-accent)/6 pl-3 before:absolute before:left-0 before:top-1 before:bottom-1 before:w-[3px] before:rounded-r before:bg-(--color-outl-accent)"
                : ""
            }
          >
            <Show when={props.showCrumbs}>
              {/* A link to the page this block lives on, at the block the
                  trail names. Orphans (no `source_page`) stay inert text.
                  Enter stops at the row: left to bubble, it would reach
                  the Normal-mode dispatcher and start an edit. Space is
                  not bound, as on a native link (it is the scroll key).
                  The `[[ref]]` / `#tag` inside navigate on their own and
                  do not reach this handler. */}
              <Show
                when={link().source_page}
                fallback={
                  <div
                    class="truncate px-1 pt-0.5 text-xs opacity-40"
                    title={crumbTrailText(link().ancestors)}
                  >
                    <CrumbTrail crumbs={link().ancestors} />
                  </div>
                }
              >
                <div
                  role="link"
                  tabindex="0"
                  data-backlink-crumb
                  class="cursor-pointer truncate rounded px-1 pt-0.5 text-xs opacity-40 hover:bg-(--color-outl-fg)/5 hover:opacity-70 focus-visible:opacity-70"
                  title={crumbTrailText(link().ancestors)}
                  onClick={() => props.onCrumbClick(link())}
                  onKeyDown={(e) => {
                    if (e.key !== "Enter") return;
                    e.preventDefault();
                    e.stopPropagation();
                    props.onCrumbClick(link());
                  }}
                >
                  <CrumbTrail
                    crumbs={link().ancestors}
                    onRefClick={crumbCallbacks().onRefClick}
                    onTagClick={crumbCallbacks().onTagClick}
                  />
                </div>
              </Show>
            </Show>
            {/* `group/blrow` scopes the chevron's reveal to this row. */}
            <div class="group/blrow flex items-start">
              {/* The outline's chevron. Hidden at rest and shown on hover,
                  focus or the cursor, the reveal `.outl-row-chrome` gives
                  an outline row; spelled as `group/blrow` utilities because
                  that class also carries focus mode's dimming. Inert on an
                  orphan: there is no page to write the fold to. */}
              <FoldChevron
                childCount={node().children.length}
                collapsed={node().collapsed}
                enabled={editTarget() !== null}
                onToggle={(collapse) => props.onFold(link(), node(), collapse)}
                class={`mt-[3px] transition-opacity group-hover/blrow:opacity-100 group-focus-within/blrow:opacity-100 ${
                  selected() ? "opacity-100" : "opacity-0"
                }`}
              />
              <button
                type="button"
                data-todo={todo() ?? "none"}
                onClick={() => props.onToggle(link(), node())}
                class={`mt-[2px] mr-2 w-3 shrink-0 cursor-pointer text-center text-[13px] leading-none hover:opacity-70 ${
                  todo() === "DONE"
                    ? "text-(--color-outl-todo-done-fg)"
                    : todo() === "TODO" || todo() === "DOING"
                      ? "text-(--color-outl-todo-open-fg)"
                      : "text-(--color-outl-fg-dimmer)"
                }`}
                title={backlinkTodoLabel(todo())}
                aria-label={backlinkTodoLabel(todo())}
              >
                {todo() === "DONE"
                  ? "▣"
                  : todo() === "DOING"
                    ? "▨"
                    : todo() === "TODO"
                      ? "▢"
                      : "•"}
              </button>
              <Show when={editing()} fallback={rowText()}>
                {/* Mounted only while this row is the one being edited,
                    which seeds its draft once per edit session. */}
                {(target) => (
                  <div class="min-w-0 flex-1 px-1 py-0.5 text-sm leading-snug">
                    <BacklinkRowEditor node={node()} target={target()} />
                  </div>
                )}
              </Show>
            </div>
          </div>
        );
      }}
    </Show>
  );
}

/**
 * A citing block's `<li>`, with its subtree under it.
 *
 * **One `<li>` per citing block**, however deep its subtree: the
 * descendants are `div`s inside it, so the count of `li`s is still the
 * count of references the header states. `childKeys` comes from
 * `flattenBacklinkRows`, which stops under a `collapsed` node, so a fold
 * made here or on the source page hides the same rows. The rail is a
 * plain border token, not the accent: the accent means "the cursor is
 * here".
 */
export function BacklinkRowItem(
  props: RowProps & { childKeys: readonly string[] },
) {
  return (
    <li>
      <RowBody {...props} />
      <For each={props.childKeys}>
        {(key) => {
          const depth = () => backlinkRowByKey(key)?.depth ?? 1;
          return (
            <div
              data-backlink-child={key}
              class="ml-2 border-l border-(--color-outl-border)"
              style={{ "padding-left": `${Math.max(depth() - 1, 0) * 1.25 + 0.5}rem` }}
            >
              <RowBody
                rowKey={key}
                cursor={props.cursor}
                showCrumbs={false}
                onToggle={props.onToggle}
                onFold={props.onFold}
                bodyCallbacks={props.bodyCallbacks}
                onCrumbClick={props.onCrumbClick}
              />
            </div>
          );
        }}
      </For>
    </li>
  );
}
