import { For, JSX, Show } from "solid-js";

import { isImportStamp } from "@outl/shared/markdown";

import { SwipeRow } from "./SwipeRow";

/**
 * The label column is `w-[104px]`, written as a literal at both call
 * sites on purpose. Tailwind's JIT scans source text, so a class built
 * by interpolation (`w-[${LABEL_W}px]`) is never generated and the
 * column silently collapses to its content — the same trap that
 * killed `text-[11px]` on the old chips. A constant would read better
 * and would not survive the build.
 */

export interface PropertyFieldListProps {
  /** `(key, value)` pairs, already filtered and in the backend's order. */
  properties: Array<[string, string]>;
  /** A write is in flight; every row is inert until it lands. */
  busy: boolean;
  onEdit: (key: string, value: string) => void;
  /** Swipe-to-delete. The sheet turns this into an empty-value write. */
  onDelete: (key: string) => void;
  onAdd: () => void;
}

/**
 * A block's or page's properties as a **field list**: label column,
 * value column, chevron.
 *
 * The shape is the argument. A `key:: value` property is a named
 * attribute with a value, which every mobile app renders as a
 * two-column row — iOS Settings, Notion's property panel, a contact
 * card. This sheet used to render `key` hugging the left edge with the
 * value pushed hard right, so the values of two rows with different
 * key lengths landed in different places and the list read as ragged
 * text rather than as a table of fields.
 *
 * The pill the outline used to draw was the sharper version of the
 * same mistake: a pill is a *tag*, a flat label you tap to filter.
 * Drawing a field as one promises an action the tap does not perform.
 * So the pill went back to meaning tag only, and editing moved here,
 * where a row that looks like a form row opens a form.
 *
 * Presentational: no store, no command, no workspace. The sheet owns
 * what a tap means, which is what lets this file be driven by tests
 * that mount it alone.
 */
export function PropertyFieldList(props: PropertyFieldListProps): JSX.Element {
  /**
   * An import's leftovers (`created`, `edited`, `roam-uid`) are listed
   * so they can be *deleted*, not so they can be read — on a migrated
   * graph they are on every block. The outline hides them outright;
   * here they stay, one step quieter than a property the user wrote.
   */
  const stamp = (key: string, value: string) => isImportStamp(key, value);

  return (
    <div class="mx-4 my-2 overflow-hidden rounded-xl border border-(--color-outl-border)/50">
      <For each={props.properties}>
        {([key, value], i) => (
          <SwipeRow
            leftActionLabel="Delete"
            onSwipeLeft={() => props.onDelete(key)}
          >
            <button
              type="button"
              disabled={props.busy}
              aria-label={`Edit ${key}`}
              onClick={() => props.onEdit(key, value)}
              // `min-h-[44px]` is the floor, not the height: a value
              // that wraps grows the row instead of being clipped.
              class="flex min-h-[44px] w-full items-center gap-3 bg-(--color-outl-bg-elev) px-4 py-2.5 text-left active:bg-(--color-outl-border)/30 disabled:opacity-50"
              classList={{
                "border-t border-(--color-outl-border)/40": i() > 0,
              }}
            >
              <span
                class="w-[104px] shrink-0 truncate font-mono text-[13px] text-(--color-outl-fg-dimmer)"
              >
                {key}
              </span>
              <span
                class="min-w-0 flex-1 truncate text-[15px]"
                classList={{
                  "text-(--color-outl-fg-dim)": stamp(key, value),
                  "text-(--color-outl-fg)": !stamp(key, value),
                }}
              >
                {value}
              </span>
              {/* The "this opens something" affordance iOS taught. It
                  carries nothing the label does not already say, so it
                  stays out of the accessibility tree. */}
              <span
                aria-hidden="true"
                class="shrink-0 text-[15px] leading-none text-(--color-outl-fg-dimmer)/70"
              >
                ›
              </span>
            </button>
          </SwipeRow>
        )}
      </For>

      <Show when={props.properties.length === 0}>
        <div class="bg-(--color-outl-bg-elev) px-4 py-6 text-center text-[14px] text-(--color-outl-fg-dim)">
          No properties yet.
        </div>
      </Show>

      {/* Last, not first. A list you add to reads top-to-bottom and
          ends with the way to grow it; leading with the action buries
          the data it acts on. */}
      <button
        type="button"
        disabled={props.busy}
        aria-label="Add property"
        onClick={props.onAdd}
        class="flex min-h-[44px] w-full items-center gap-3 bg-(--color-outl-bg-elev) px-4 py-2.5 text-left active:bg-(--color-outl-border)/30 disabled:opacity-50"
        classList={{
          "border-t border-(--color-outl-border)/40":
            props.properties.length > 0,
        }}
      >
        <span
          aria-hidden="true"
          class="w-[104px] shrink-0 text-[17px] leading-none text-(--color-outl-accent)"
        >
          +
        </span>
        <span class="min-w-0 flex-1 text-[15px] font-medium text-(--color-outl-accent)">
          Add property
        </span>
      </button>
    </div>
  );
}
