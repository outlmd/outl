import { For, Show } from "solid-js";

import { backlinkEditTargetFor } from "@outl/shared/backlink-edit";
import {
  CrumbTrail,
  crumbTrailText,
  EmbeddedSubtree,
  MarkdownInline,
  MarkdownTable,
} from "@outl/shared/markdown";
import { embedOnlyHandle } from "@outl/shared/outline";
import type { Backlink, BlockNode } from "@outl/shared/api/types";

import { backlinkRowByKey, startBacklinkEdit } from "../lib/cursor-region";
import { appState } from "../lib/store";
import { BacklinkRowEditor } from "./BacklinkRowEditor";

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
        const node = () => r().node;
        const todo = () => (r().depth === 0 ? link().todo : node().todo);
        const selected = () => props.cursor() === props.rowKey;
        const embedded = () => {
          const handle = embedOnlyHandle(node().tokens);
          return handle ? appState.embeds[handle] : undefined;
        };
        // `null` for an orphan: the row stays read-only.
        const editTarget = () => backlinkEditTargetFor(link(), node());
        const editing = () => {
          const ed = appState.editingBacklink;
          return ed !== null && (ed.key ?? ed.blockId) === props.rowKey
            ? editTarget()
            : null;
        };
        const rowText = () => (
          <button
            type="button"
            // Editing in place, rather than opening the source page,
            // which stays on the group header. No `onMouseEnter`: a
            // pointer crossing the section is not a cursor move.
            onClick={() => {
              const target = editTarget();
              if (target) startBacklinkEdit(target, props.rowKey);
            }}
            class={`block min-w-0 flex-1 rounded px-1 py-0.5 text-left text-sm leading-snug opacity-90 hover:bg-(--color-outl-fg)/5 hover:opacity-100 ${
              todo() === "DONE" ? "line-through opacity-60" : ""
            }`}
          >
            <Show
              when={node().table}
              fallback={
                <MarkdownInline
                  tokens={node().tokens}
                  variant="inline"
                  embeds={appState.embeds}
                  pageEmbeds={appState.pageEmbeds}
                />
              }
            >
              {/* Inert table: the row's own click is the edit gesture,
                  and a table has no cell editor by design (#329). */}
              <MarkdownTable
                  table={node().table!}
                  variant="inline"
                  embeds={appState.embeds}
                  pageEmbeds={appState.pageEmbeds}
                />
            </Show>
          </button>
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
              <div
                class="truncate px-1 pt-0.5 text-xs opacity-40"
                title={crumbTrailText(link().ancestors)}
              >
                <CrumbTrail crumbs={link().ancestors} />
              </div>
            </Show>
            <div class="flex items-start">
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
            {/* A block that is only `!((blk-…))` expands its source,
                like the outline's `BlockBody`. Read through an
                accessor: the resolve lands after the row is mounted. */}
            <Show when={embedded()?.children?.length}>
              <EmbeddedSubtree
                nodes={embedded()!.children}
                embeds={appState.embeds}
                pageEmbeds={appState.pageEmbeds}
              />
            </Show>
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
 * count of references the header states. Always expanded — the source
 * page's `collapsed` flag is not read, which is the embed's behaviour
 * too. The rail is a plain border token, not the accent: the accent
 * means "the cursor is here".
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
              />
            </div>
          );
        }}
      </For>
    </li>
  );
}
