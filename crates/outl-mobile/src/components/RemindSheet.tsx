import { For, Show, createEffect, createSignal } from "solid-js";

import { formatRemindRule, parseRemindRule } from "@outl/shared/api/commands";
import type { RemindFields } from "@outl/shared/api/types";

import { createSheetDrag } from "../lib/sheet-drag";
import { useKeyboardInset } from "../lib/viewport";
import {
  REPEAT_CHOICES,
  clockValue,
  defaultRemindFields,
  parseClock,
  repeatLabel,
  todayDateValue,
  withUntil,
} from "../lib/remind-fields";

/**
 * "Remind me…" — authoring a block's `remind::` rule with controls
 * instead of a `window.prompt` for the rule language.
 *
 * The prompt it replaces was not a shortcut nobody got around to
 * finishing: `docs/reminders.md` argued for it, because the grammar is
 * richer than a clock (`3pm every 1h until DONE`) and a picker that
 * can only express the anchor hides the repeat. That argument is
 * right about pickers and wrong about phones — a raw prompt for a DSL
 * is easy to miss, easy to cancel, and reads as "I tapped it and
 * nothing happened" ([#572](https://github.com/outlmd/outl/issues/572)).
 *
 * So this sheet has both halves, and the second one is what keeps the
 * argument honest: **Edit as text** carries the rule verbatim, and a
 * rule the controls cannot express opens there rather than being
 * quietly simplified into one they can.
 *
 * **No grammar is written here.** The rule text is read by
 * `parseRemindRule` and written by `formatRemindRule`, both over
 * `outl_md::remind`, which is the single owner in both directions —
 * two calls per session, on open and on save, never per keystroke.
 * The schedule (*when* this fires) is `outl_actions::reminders`' and
 * is not computed on this side at all.
 */
export function RemindSheet(props: {
  open: boolean;
  /** The block's current `remind::` value; `""` when it carries none. */
  rule: string;
  /** The rule to write. `""` clears the property, which is how a
   *  reminder is removed without deleting the block. */
  onSubmit: (rule: string) => void;
  onCancel: () => void;
  /** Backend refusals (an unsendable field set) go to the caller's toast. */
  onMessage?: (text: string) => void;
}) {
  const drag = createSheetDrag(() => props.onCancel());
  const keyboardInset = useKeyboardInset();
  const [fields, setFields] = createSignal<RemindFields>(defaultRemindFields());
  // The "Edit as text" escape, and the one state that is not a field:
  // a rule the controls cannot express opens here with its own text.
  const [raw, setRaw] = createSignal("");
  const [rawMode, setRawMode] = createSignal(false);
  const [saving, setSaving] = createSignal(false);

  createEffect(() => {
    if (!props.open) return;
    const current = props.rule.trim();
    setSaving(false);
    setRaw(current);
    if (!current) {
      setFields(defaultRemindFields());
      setRawMode(false);
      return;
    }
    void parseRemindRule(current)
      .then((parsed) => {
        if (parsed) {
          setFields(parsed);
          setRawMode(false);
        } else {
          // A rule that does not parse is a typo to fix in place, the
          // same stance `parse_remind` takes. Dropping the user into
          // the controls would show them a 09:00 that has nothing to
          // do with what their block says.
          setRawMode(true);
        }
      })
      .catch(() => setRawMode(true));
  });

  const patch = (next: Partial<RemindFields>) => setFields({ ...fields(), ...next });

  async function save() {
    if (saving()) return;
    setSaving(true);
    try {
      props.onSubmit(rawMode() ? raw().trim() : await formatRemindRule(fields()));
    } catch (e) {
      props.onMessage?.(e instanceof Error ? e.message : String(e));
    } finally {
      setSaving(false);
    }
  }

  const rowClass =
    "flex min-h-12 items-center justify-between gap-3 border-t border-(--color-outl-border)/30 px-4";
  const pickerClass =
    "rounded-lg bg-(--color-outl-border)/40 px-2 py-1.5 text-[15px] text-(--color-outl-fg)";

  return (
    <Show when={props.open}>
      <div
        class="outl-fade-in fixed inset-0 z-[56] bg-black/40 backdrop-blur-md"
        onClick={props.onCancel}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-[56] max-h-[85vh] overflow-y-auto rounded-t-2xl bg-(--color-outl-bg-elev) pt-3 shadow-2xl"
        style={{
          "padding-bottom": `max(var(--safe-area-inset-bottom), ${keyboardInset()}px, 12px)`,
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging() ? "none" : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
        role="dialog"
        aria-label="Remind me"
      >
        <span
          class="mx-auto block h-3 w-16 cursor-grab py-1 active:cursor-grabbing"
          style={{ "touch-action": "none" }}
          onPointerDown={drag.onPointerDown}
          onPointerMove={drag.onPointerMove}
          onPointerUp={drag.onPointerUp}
          onPointerCancel={drag.onPointerCancel}
          aria-label="Drag to close"
          role="button"
        >
          <span
            aria-hidden="true"
            class="mx-auto block h-1 w-10 rounded-full bg-(--color-outl-border)"
          />
        </span>
        <h2 class="px-4 pt-2 pb-3 text-[17px] font-semibold">Remind me</h2>

        <Show
          when={!rawMode()}
          fallback={
            <div class="px-4 pb-1">
              <label
                for="remind-raw"
                class="block pb-1 text-[13px] text-(--color-outl-fg-dim)"
              >
                Rule, in its own words — e.g. 3pm every 1h until DONE
              </label>
              <input
                id="remind-raw"
                type="text"
                value={raw()}
                autocapitalize="off"
                autocomplete="off"
                spellcheck={false}
                enterkeyhint="done"
                onInput={(e) => setRaw(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key !== "Enter") return;
                  e.preventDefault();
                  void save();
                }}
                class="min-h-12 w-full rounded-xl bg-(--color-outl-bg) px-3 text-[16px] outline-none"
              />
            </div>
          }
        >
          {/* Starts — the anchor. `now` has no clock of its own, so the
              time picker is hidden rather than left showing a value
              the rule will not carry. */}
          <div class={rowClass}>
            <span class="text-[15px]">Starts</span>
            <div class="flex shrink-0 items-center gap-2">
              <Show when={fields().anchor === "at"}>
                <input
                  type="time"
                  aria-label="Reminder time"
                  value={clockValue(fields().hour, fields().minute)}
                  onChange={(e) => {
                    const at = parseClock(e.currentTarget.value);
                    if (at) patch(at);
                  }}
                  class={pickerClass}
                />
              </Show>
              <button
                type="button"
                aria-pressed={fields().anchor === "now"}
                onClick={() =>
                  patch({ anchor: fields().anchor === "now" ? "at" : "now" })
                }
                class="min-h-11 rounded-lg px-2 text-[15px] font-medium"
                classList={{
                  "bg-(--color-outl-accent) text-(--color-outl-accent-fg)":
                    fields().anchor === "now",
                  "text-(--color-outl-accent)": fields().anchor !== "now",
                }}
              >
                Now
              </button>
            </div>
          </div>

          {/* Repeat. An interval the list does not offer keeps its own
              label rather than reading as "Once" — see `repeatLabel`. */}
          <div class={rowClass}>
            <label for="remind-repeat" class="text-[15px]">
              Repeat
            </label>
            <select
              id="remind-repeat"
              value={String(fields().every_minutes ?? "")}
              onChange={(e) => {
                const v = e.currentTarget.value;
                patch({ every_minutes: v === "" ? null : Number(v) });
              }}
              class={`${pickerClass} shrink-0`}
            >
              <For each={REPEAT_CHOICES}>
                {(choice) => (
                  <option value={String(choice.minutes ?? "")}>{choice.label}</option>
                )}
              </For>
              <Show
                when={
                  fields().every_minutes !== null &&
                  !REPEAT_CHOICES.some((c) => c.minutes === fields().every_minutes)
                }
              >
                <option value={String(fields().every_minutes)}>
                  {repeatLabel(fields().every_minutes)}
                </option>
              </Show>
            </select>
          </div>

          {/* Until — only for a rule that repeats. A stop clause on a
              single shot is a control with nothing to stop. */}
          <Show when={fields().every_minutes !== null}>
            <div class={rowClass}>
              <label for="remind-until" class="text-[15px]">
                Until
              </label>
              <div class="flex shrink-0 items-center gap-2">
                <select
                  id="remind-until"
                  value={fields().until}
                  onChange={(e) =>
                    setFields(
                      withUntil(
                        fields(),
                        e.currentTarget.value as "done" | "time" | "date",
                        todayDateValue(),
                      ),
                    )
                  }
                  class={pickerClass}
                >
                  <option value="done">It's done</option>
                  <option value="time">A time today</option>
                  <option value="date">A date</option>
                </select>
                <Show when={fields().until === "time"}>
                  <input
                    type="time"
                    aria-label="Stop at"
                    value={clockValue(fields().until_hour ?? 0, fields().until_minute ?? 0)}
                    onChange={(e) => {
                      const at = parseClock(e.currentTarget.value);
                      if (at) patch({ until_hour: at.hour, until_minute: at.minute });
                    }}
                    class={pickerClass}
                  />
                </Show>
                <Show when={fields().until === "date"}>
                  <input
                    type="date"
                    aria-label="Stop on"
                    value={fields().until_date ?? todayDateValue()}
                    onChange={(e) => patch({ until_date: e.currentTarget.value })}
                    class={pickerClass}
                  />
                </Show>
              </div>
            </div>
          </Show>
        </Show>

        {/* The escape hatch, both ways: the controls cover the common
            rule, and this covers the rest of the grammar. */}
        <button
          type="button"
          onClick={async () => {
            if (rawMode()) {
              // Back to the controls means reading the text the user
              // may have changed, not restoring what was there before.
              const parsed = await parseRemindRule(raw().trim()).catch(() => null);
              if (!parsed) {
                props.onMessage?.("That rule isn't one the pickers can show. Keeping the text.");
                return;
              }
              setFields(parsed);
              setRawMode(false);
              return;
            }
            setRaw(await formatRemindRule(fields()).catch(() => raw()));
            setRawMode(true);
          }}
          class="min-h-12 w-full border-t border-(--color-outl-border)/30 px-4 text-left text-[15px] text-(--color-outl-accent) active:bg-(--color-outl-border)/20"
        >
          {rawMode() ? "Use the pickers" : "Edit as text"}
        </button>

        <div class="flex items-center gap-3 px-4 pt-4">
          {/* Clearing the property is the only way to stop a block
              nagging without deleting it, so it is a button rather
              than an empty value the user has to discover. */}
          <button
            type="button"
            onClick={() => props.onSubmit("")}
            disabled={props.rule.trim() === ""}
            class="min-h-12 rounded-xl px-2 text-[16px] font-medium text-(--color-outl-destructive) active:opacity-70 disabled:opacity-30"
          >
            Remove
          </button>
          <span class="flex-1" />
          <button
            type="button"
            onClick={props.onCancel}
            class="min-h-12 rounded-xl px-3 text-[16px] font-medium text-(--color-outl-accent) active:bg-(--color-outl-border)/30"
          >
            Cancel
          </button>
          <button
            type="button"
            disabled={saving() || (rawMode() && raw().trim() === "")}
            onClick={() => void save()}
            class="min-h-12 rounded-xl bg-(--color-outl-accent) px-4 text-[16px] font-semibold text-(--color-outl-accent-fg) active:opacity-80 disabled:opacity-40"
          >
            Save
          </button>
        </div>
      </div>
    </Show>
  );
}
