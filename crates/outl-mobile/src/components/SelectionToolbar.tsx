import { JSX, Show } from "solid-js";

interface SelectionToolbarProps {
  open: boolean;
  /** Number of blocks in the active range — shown as "`N` selected"
   *  and used to disable the batch ops on an empty/stale selection. */
  count: number;
  onGrowUp: () => void;
  onGrowDown: () => void;
  onIndent: () => void;
  onOutdent: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
  onCopy: () => void;
  onDelete: () => void;
  /** Exit the selection without acting on it — the explicit, always-
   *  visible way out (RFC 0254 phase 3's "obvious exit" requirement). */
  onDone: () => void;
}

/**
 * Floating batch-operation bar for a touch-native block range
 * selection (RFC 0254 phase 3). Mirrors the desktop's
 * `<BatchToolbar />` — same actions, same "fires the handler the
 * gesture would" principle — but bottom-docked (thumb reach) and adds
 * a "Copy" button the desktop doesn't need (there, `y`/`Y` covers it;
 * mobile has no keyboard chord for anything).
 *
 * `▲`/`▼` grow the range by exactly one visible row — the discrete
 * equivalent of the desktop's `Shift+↑`/`Shift+↓` (`SelectRangeUp` /
 * `SelectRangeDown`) for a user who wants that instead of tapping a
 * distant row directly.
 */
export function SelectionToolbar(props: SelectionToolbarProps): JSX.Element {
  return (
    <Show when={props.open}>
      <div
        class="fixed inset-x-3 z-40 flex items-center gap-1 overflow-x-auto rounded-2xl bg-(--color-outl-bg-elev)/95 px-2 py-2 shadow-[var(--shadow-capsule)] backdrop-blur-2xl"
        style={{ bottom: "max(env(safe-area-inset-bottom), 16px)" }}
      >
        <span class="shrink-0 px-2 text-[13px] font-medium text-(--color-outl-fg)">
          {props.count} selected
        </span>
        <span
          aria-hidden="true"
          class="mx-0.5 h-5 w-px shrink-0 bg-(--color-outl-border)"
        />
        <ToolbarButton label="▲" title="Extend selection up" onClick={props.onGrowUp} />
        <ToolbarButton
          label="▼"
          title="Extend selection down"
          onClick={props.onGrowDown}
        />
        <span
          aria-hidden="true"
          class="mx-0.5 h-5 w-px shrink-0 bg-(--color-outl-border)"
        />
        <ToolbarButton label="Indent" onClick={props.onIndent} />
        <ToolbarButton label="Outdent" onClick={props.onOutdent} />
        <ToolbarButton label="Move ↑" onClick={props.onMoveUp} />
        <ToolbarButton label="Move ↓" onClick={props.onMoveDown} />
        <ToolbarButton label="Copy" onClick={props.onCopy} />
        <ToolbarButton label="Delete" onClick={props.onDelete} danger />
        <span
          aria-hidden="true"
          class="mx-0.5 h-5 w-px shrink-0 bg-(--color-outl-border)"
        />
        <button
          type="button"
          onClick={props.onDone}
          class="shrink-0 rounded-xl bg-(--color-outl-accent) px-3 py-1.5 text-[14px] font-semibold text-white active:opacity-70"
        >
          Done
        </button>
      </div>
    </Show>
  );
}

function ToolbarButton(props: {
  label: string;
  title?: string;
  onClick: () => void;
  danger?: boolean;
}) {
  return (
    <button
      type="button"
      title={props.title}
      aria-label={props.title ?? props.label}
      onClick={props.onClick}
      class="shrink-0 rounded-lg px-2.5 py-1.5 text-[14px] font-medium text-(--color-outl-fg) active:bg-(--color-outl-accent)/[0.12]"
      classList={{
        "text-(--color-outl-destructive) active:bg-(--color-outl-destructive)/[0.12]":
          props.danger,
      }}
    >
      {props.label}
    </button>
  );
}
