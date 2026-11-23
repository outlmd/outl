import { Show } from "solid-js";
import type { BlockNode } from "@outl/shared/api/types";

/**
 * The row's left gutter: the fold triangle and the bullet / checkbox.
 *
 * Both are the same kind of thing — a fixed-width tap target that sits
 * *outside* the block body (and outside the quote chrome), reserving its
 * slot even when it has nothing to draw so the bullet column never
 * shifts between siblings. They moved out of `BlockRow.tsx` together for
 * that reason: a change to one column's width is a change to the other's
 * offset.
 *
 * Presentational only. Neither knows what a tap means — the caller
 * decides whether `onToggle` folds, extends a range selection, or marks
 * a task.
 */

export function CollapseTriangle(props: {
  visible: boolean;
  collapsed: boolean;
  onToggle: () => void;
}) {
  // Always reserve the slot — even on leaves — so the bullet column
  // stays put regardless of whether a sibling has children. Width
  // matches the bullet (`w-[26px]`).
  return (
    <Show
      when={props.visible}
      fallback={<span aria-hidden="true" class="w-[18px] shrink-0" />}
    >
      <button
        type="button"
        aria-label={props.collapsed ? "Expand block" : "Collapse block"}
        aria-expanded={!props.collapsed}
        onClick={(e) => {
          e.stopPropagation();
          props.onToggle();
        }}
        class="relative z-10 -my-1.5 flex h-[30px] w-[18px] shrink-0 items-center justify-center text-(--color-outl-fg-dimmer)"
      >
        <span aria-hidden="true" class="text-[10px] leading-none">
          {props.collapsed ? "▶" : "▼"}
        </span>
      </button>
    </Show>
  );
}

export function BulletOrCheckbox(props: {
  todo: BlockNode["todo"];
  onToggle: () => void;
  /** Zoom into this block (Roam/Workflowy). When set, a tap on the
   *  plain bullet dot focuses; TODO toggling stays in the long-press
   *  menu. When `undefined`, the dot marks TODO as before. */
  onFocus?: () => void;
}) {
  // Apple HIG: minimum tap target is 44×44. We hit ~36×30 here so we
  // stay visually compact in dense outlines but no longer demand
  // pixel-perfect taps. The visual dot/checkbox keeps its old size
  // — the surrounding `<button>` is what grows.
  return (
    <Show
      when={props.todo !== null}
      fallback={
        <button
          type="button"
          // The one `<button>` in a row that a long press may pass
          // through — `BlockBody.pressedInteractive` keys on this
          // attribute, and reaching for the bullet is how a finger says
          // "act on this block". Both branches carry it: the dot and
          // the checkbox are the same control wearing two states.
          data-block-bullet
          aria-label={props.onFocus ? "Zoom into block" : "Mark as TODO"}
          onClick={(e) => {
            e.stopPropagation();
            // Bullet dot zooms when the client supports it; otherwise it
            // keeps its legacy mark-as-TODO behaviour. TODO stays
            // reachable via the long-press context menu regardless.
            if (props.onFocus) props.onFocus();
            else props.onToggle();
          }}
          class="group/bullet relative z-10 -my-1.5 -ml-2 flex h-[30px] w-[26px] shrink-0 items-center justify-center"
        >
          <span
            aria-hidden="true"
            class="h-1.5 w-1.5 rounded-full bg-(--color-outl-fg-dimmer) transition-transform group-active/bullet:scale-150"
          />
        </button>
      }
    >
      <button
        type="button"
        data-block-bullet
        aria-label={
          props.todo === "DONE"
            ? "Clear task state"
            : props.todo === "DOING"
              ? "Mark as done"
              : "Mark as doing"
        }
        onClick={(e) => {
          e.stopPropagation();
          props.onToggle();
        }}
        class="relative z-10 -my-[13px] -ml-[8px] flex h-[44px] w-[44px] shrink-0 items-center justify-center"
      >
        <span
          class="flex h-[18px] w-[18px] items-center justify-center rounded-[4px] border-[1.5px] transition-colors"
          classList={{
            "border-(--color-outl-todo-done-fg) bg-(--color-outl-todo-done-fg)":
              props.todo === "DONE",
            // Theme task colours, as on desktop and in the TUI: open
            // work (TODO, DOING) in todo-open, finished in todo-done.
            // DOING adds a small dot so it reads as underway.
            "border-(--color-outl-todo-open-fg) bg-transparent":
              props.todo === "DOING" || props.todo === "TODO",
          }}
        >
          <Show when={props.todo === "DOING"}>
            <span
              aria-hidden="true"
              class="h-[8px] w-[8px] rounded-[2px] bg-(--color-outl-todo-open-fg)"
            />
          </Show>
          <Show when={props.todo === "DONE"}>
            <svg
              width="12"
              height="12"
              viewBox="0 0 24 24"
              fill="none"
              stroke="var(--color-outl-bg)"
              stroke-width="3.5"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <path d="M5 12l4 4 10-10" />
            </svg>
          </Show>
        </span>
      </button>
    </Show>
  );
}
