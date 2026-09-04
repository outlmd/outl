import { Show } from "solid-js";

import type { BlockNode } from "@outl/shared/api/types";
import { visualRangeSet } from "@outl/shared/outline";

import type { ActionHandlers } from "../lib/shortcuts";
import { appState } from "../lib/store";

/**
 * Floating batch-operation toolbar for multi-block selection.
 *
 * Appears whenever a contiguous block range is selected — whether the
 * user reached it via vim's `V` or the non-vim `Shift+↑/↓` entry — so
 * the batch ops are discoverable without knowing the chords. It fires
 * the **same** `action-handlers` the keyboard dispatcher does (passed
 * in as `handlers`), so button and chord can never drift: the range
 * walking, bottom-up ordering, and `g v` capture all live once in
 * `action-handlers.ts`.
 *
 * Delete confirms first when any selected block has nested children —
 * a batch delete of collapsed subtrees is the one destructive path a
 * user can trigger without seeing what they're about to lose.
 */
export function BatchToolbar(props: { handlers: ActionHandlers }) {
  const rangeSet = () =>
    visualRangeSet(
      appState.visualAnchorId,
      appState.selectedBlockId,
      appState.outline,
    );
  const count = () => rangeSet()?.size ?? 0;
  const active = () => appState.mode === "vim-visual" && count() > 0;

  /** Does any block in the range carry children (a nested subtree the
   *  batch delete would take with it, possibly hidden behind a fold)? */
  function anyHasDescendants(): boolean {
    const set = rangeSet();
    if (!set) return false;
    const has = (nodes: BlockNode[]): boolean =>
      nodes.some(
        (n) => (set.has(n.id) && n.children.length > 0) || has(n.children),
      );
    return has(appState.outline);
  }

  const run = (kind: keyof ActionHandlers) => () => {
    void props.handlers[kind]?.();
  };

  function onDelete() {
    if (anyHasDescendants()) {
      const ok = window.confirm(
        `Delete ${count()} block(s)?\n\n` +
          `Some of them have nested children that will be deleted too.`,
      );
      if (!ok) return;
    }
    void props.handlers.DeleteRange?.();
  }

  return (
    <Show when={active()}>
      <div class="fixed top-3 left-1/2 z-30 flex -translate-x-1/2 items-center gap-1 rounded-lg border border-(--color-outl-border) bg-(--color-outl-bg-elev) p-1 shadow-lg">
        <span class="px-2 text-sm font-medium text-(--color-outl-fg)">
          {count()} selected
        </span>
        <span class="mx-1 h-5 w-px bg-(--color-outl-border)" />
        <BatchButton label="Indent" title="Indent range (>)" onClick={run("IndentVisualRange")} />
        <BatchButton
          label="Outdent"
          title="Outdent range (<)"
          onClick={run("OutdentVisualRange")}
        />
        <BatchButton label="↑" title="Move range up (⌘⇧↑)" onClick={run("MoveVisualRangeUp")} />
        <BatchButton label="↓" title="Move range down (⌘⇧↓)" onClick={run("MoveVisualRangeDown")} />
        <BatchButton label="Delete" title="Delete range (Delete)" onClick={onDelete} danger />
        <span class="mx-1 h-5 w-px bg-(--color-outl-border)" />
        <BatchButton label="Done" title="Clear selection (Esc)" onClick={run("ExitInsert")} />
      </div>
    </Show>
  );
}

function BatchButton(props: {
  label: string;
  title: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      title={props.title}
      aria-label={props.title}
      onClick={props.onClick}
      class="rounded-md px-2 py-1 text-sm text-(--color-outl-fg) hover:bg-(--color-outl-accent)/[0.12]"
      classList={{
        "text-(--color-outl-destructive) hover:bg-(--color-outl-destructive)/[0.12]":
          props.danger,
      }}
    >
      {props.label}
    </button>
  );
}
