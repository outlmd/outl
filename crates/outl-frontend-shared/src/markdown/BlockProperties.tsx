import { For, Show, createMemo, type JSX } from "solid-js";

import { BellIcon } from "../icons";
import { attributeFields } from "./properties";

export interface BlockPropertiesProps {
  /** The block's `(key, value)` pairs, alpha-sorted by the backend. */
  properties?: ReadonlyArray<readonly [string, string]>;
  /**
   * Open the properties sheet for this block or page.
   *
   * Omit and the line renders as inert text. It deliberately does
   * **not** fall back to a `<button>` that does nothing: a control
   * that cannot deliver the tap it advertises is the defect the pills
   * had, and a phone gives no hover to discover the difference.
   */
  onOpen?: () => void;
  /** Accessible name for the tap target. */
  label?: string;
  /** Host classes for the line — padding, margins. Not the type scale. */
  rowClass?: string;
}

/**
 * The line's own layout, including its **type scale**.
 *
 * The scale is here and not in a host string because of a cascade
 * fact: mobile's `styles.css` carries an unlayered
 * `button, input, textarea { font-size: inherit }`, which outranks
 * every `@layer utilities` rule, so a `text-*` on the inner
 * `<button>` resolves to nothing and the line paints at the block
 * body's 17px. The container holds the size; the button inherits it.
 * That bug is what this whole shape replaced.
 */
const LINE_LAYOUT =
  "mt-px flex items-center gap-1.5 overflow-hidden whitespace-nowrap text-[12.5px] leading-[1.35]";

/** The tap target inside the line, sized by the line above it. */
const INNER_LAYOUT = "flex min-w-0 items-center gap-1.5 overflow-hidden";

/**
 * A block's `key:: value` properties as an **attribute line** under
 * its text.
 *
 * This shipped as a row of pills, to fix a real problem: `remind::`
 * was invisible in the outline, so a rule written by a chord left the
 * block looking untouched. The pills fixed that and introduced a
 * category error. A pill means *flat label you tap to filter* — that
 * is a tag. A `key:: value` is a **field**: a named attribute with a
 * value. Drawing fields as pills put a row of things that looked like
 * buttons, were not buttons, and could not be told from `#tag`s, under
 * every block of a Roam-imported graph.
 *
 * The shape now is what Things, Linear, Apple Reminders and GitHub
 * mobile do with an item's metadata: one quiet subtitle line, key
 * receding behind value, colour spent only on the field that will
 * actually interrupt you. The pill goes back to meaning one thing.
 *
 * **Editing is not here.** The chips edited a value in place, which
 * is why each one had to be a control. One tap on the line opens the
 * properties sheet instead — a two-column form, which is where
 * editing a field belongs and where creating and deleting one already
 * lived. Presentational: no store, no command.
 */
export function BlockProperties(props: BlockPropertiesProps): JSX.Element {
  const line = createMemo(() => attributeFields(props.properties));

  const Fields = () => (
    <>
      <For each={line().fields}>
        {(field, i) => (
          <>
            <Show when={i() > 0}>
              <span class="text-(--color-outl-border)" aria-hidden="true">
                ·
              </span>
            </Show>
            {/* `fg-dimmer` / `fg-dim`, **not** the dedicated
                `property_key_fg` / `property_value_fg`, and that is a
                recorded divergence rather than an oversight.

                The key matches: every preset sets `property_key_fg`
                to `fg_dimmer`. The value does not — the brand pair
                sets `property_value_fg` to `accent_soft`, so the TUI
                paints *every* property value lavender. That is the
                over-signalling this redesign removed: accent here
                means "this will interrupt you", and a colour spent on
                `status waiting` spends it on nothing.

                So the two clients disagree about that one token on
                purpose, and converging them is a change to the
                **preset**, not to this component — it would restyle
                the TUI for every user. Until someone decides that,
                `property_value_fg` means "the TUI's property value",
                which is narrower than its name (root `CLAUDE.md`
                invariant 13). Recorded in `docs/theming.md`. */}
            <Show
              when={field.live}
              fallback={
                <span class="min-w-0 truncate">
                  <span class="text-(--color-outl-fg-dimmer)">{field.key}</span>{" "}
                  <span class="text-(--color-outl-fg-dim)">{field.value}</span>
                </span>
              }
            >
              {/* The one field that acts on the user: accent, icon, and
                  no key — the icon already says what it is, and the
                  schedule is the part worth the width. An SVG rather
                  than the ⏰ the TUI draws, because `DESIGN.md` keeps
                  emoji out of GUI chrome; the emoji map in
                  `properties.ts` stays for the terminal to mirror. */}
              <span class="flex min-w-0 items-center gap-1 font-medium text-(--color-outl-accent)">
                <BellIcon size={12} />
                <span class="truncate">{field.value}</span>
              </span>
            </Show>
          </>
        )}
      </For>
      <Show when={line().overflow > 0}>
        <span class="text-(--color-outl-border)" aria-hidden="true">
          ·
        </span>
        <span class="shrink-0 text-(--color-outl-fg-dimmer)">
          +{line().overflow} more
        </span>
      </Show>
    </>
  );

  return (
    <Show when={line().fields.length > 0}>
      <div class={`${LINE_LAYOUT} ${props.rowClass ?? ""}`}>
        <Show
          when={props.onOpen}
          fallback={
            <span class={INNER_LAYOUT}>
              <Fields />
            </span>
          }
        >
          <button
            type="button"
            class={`${INNER_LAYOUT} text-left`}
            aria-label={props.label ?? "Properties"}
            onClick={(e) => {
              // The row underneath selects / edits the block. A tap on
              // the attribute line means "show me these fields", never
              // both.
              e.stopPropagation();
              props.onOpen?.();
            }}
          >
            <Fields />
          </button>
        </Show>
      </div>
    </Show>
  );
}
