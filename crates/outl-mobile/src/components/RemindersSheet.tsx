import { For, JSX, Show, createEffect, createSignal } from "solid-js";

import type { PageView, Reminder } from "@outl/shared/api/types";
import {
  clearReminderSnooze,
  formatNextFire,
  groupReminders,
  listReminders,
  openPageBySlug,
  reminderSettings,
  setReminderSettings,
  snoozePresets,
  snoozeReminder,
  markBlockDone,
} from "@outl/shared/api/commands";

import { createSheetDrag } from "../lib/sheet-drag";
import { splitQuietHours, withQuietEnd } from "../lib/quiet-hours";
import { haptic } from "../lib/haptics";

interface RemindersSheetProps {
  open: boolean;
  onClose: () => void;
  /** Toast channel for backend errors. */
  onMessage: (text: string) => void;
  /** Refreshed page view after navigating to a reminder's block. */
  onView: (view: PageView) => void;
  /** Slug of the page currently on screen, so marking a row DONE only
   *  re-renders when the user is actually looking at it. */
  currentSlug: string | null;
}

/**
 * Bottom sheet listing every block with a `remind::`, grouped Today /
 * Tomorrow / This week / Later / Done.
 *
 * The grouping and the "in 3h" column come from `@outl/shared`
 * (`groupReminders`, `formatNextFire`) — the same functions the desktop
 * panel uses — and the instants behind them come from
 * `outl_actions::reminders` in Rust. Nothing about *when* a reminder
 * fires is decided in this file.
 *
 * Snooze writes `Op::SnoozeRemind`, so silencing a nag here also
 * silences it on the user's laptop.
 */
export function RemindersSheet(props: RemindersSheetProps): JSX.Element {
  const drag = createSheetDrag(() => props.onClose());
  const [reminders, setReminders] = createSignal<Reminder[]>([]);
  const [loading, setLoading] = createSignal(false);
  const [enabled, setEnabled] = createSignal(true);
  const [quietHours, setQuietHours] = createSignal("");
  const [busy, setBusy] = createSignal<string | null>(null);
  const [savingSettings, setSavingSettings] = createSignal(false);
  // Fetched, not hardcoded: "tomorrow 9am" is a wall time, so the
  // backend owns resolution and we only render its labels.
  const [presets, setPresets] = createSignal<{ id: string; label: string }[]>([]);

  /**
   * Write both device-local settings at once.
   *
   * Both go on every call because the backend command replaces the
   * pair; sending one and defaulting the other is how flipping the
   * switch would silently wipe a configured quiet window. The UI only
   * moves after the write returns, so a failed save can't leave it
   * claiming a state the config doesn't have.
   */
  async function saveSettings(nextEnabled: boolean, nextQuiet: string) {
    if (savingSettings()) return;
    setSavingSettings(true);
    haptic("light");
    try {
      const saved = await setReminderSettings(nextEnabled, nextQuiet);
      setEnabled(saved.enabled);
      setQuietHours(saved.quiet_hours);
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setSavingSettings(false);
    }
  }

  /**
   * Update one end of the quiet window from a native time picker.
   *
   * The split / join rules (and why a half-filled window saves as
   * empty) live in `lib/quiet-hours`, where they're unit-tested — a
   * silent bug there means the user's quiet hours quietly don't stick.
   */
  function setQuietEnd(which: 0 | 1, value: string) {
    const next = withQuietEnd(quietHours(), which, value);
    setQuietHours(next);
    void saveSettings(enabled(), next);
  }

  /**
   * Mark a reminder's block DONE, which cancels every pending fire.
   *
   * Resolves the block's **own** page id first: this sheet lists the
   * whole workspace, so the open page is usually a different one, and
   * the command uses the page id to render the reply and queue the
   * projection. The refreshed view is only applied when the user is
   * actually looking at that page, so ticking a row never teleports
   * them somewhere else.
   *
   * `markBlockDone`, not `toggleTodo`: a rule can sit on a block with
   * no marker, and toggling that lands on `TODO` and keeps nagging.
   */
  async function markDone(r: Reminder) {
    const target = await openPageBySlug(r.page_slug);
    const after = await markBlockDone(target.page.id, r.block_id);
    if (props.currentSlug === r.page_slug) props.onView(after);
  }

  async function refresh() {
    setLoading(true);
    try {
      const [list, settings, options] = await Promise.all([
        listReminders(),
        reminderSettings(),
        snoozePresets(),
      ]);
      setPresets(options);
      setReminders(list);
      setEnabled(settings.enabled);
      setQuietHours(settings.quiet_hours);
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
      setReminders([]);
    } finally {
      setLoading(false);
    }
  }

  // Re-read every time the sheet opens: a peer's snooze, or the clock
  // simply moving, changes what is due.
  createEffect(() => {
    if (!props.open) return;
    void refresh();
  });

  async function withRow(id: string, run: () => Promise<unknown>) {
    if (busy()) return;
    setBusy(id);
    haptic("light");
    try {
      await run();
      await refresh();
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }

  async function jumpTo(r: Reminder) {
    try {
      // Navigate to the page; the reminder's block is somewhere in it.
      // Scrolling to the exact block would mean reusing `focusBlockId`,
      // which is the *zoom* root on mobile — overloading it here would
      // silently zoom the user into a single bullet.
      const view = await openPageBySlug(r.page_slug);
      props.onView(view);
      props.onClose();
    } catch (e) {
      props.onMessage(e instanceof Error ? e.message : String(e));
    }
  }

  return (
    <Show when={props.open}>
      <div
        class="outl-fade-in fixed inset-0 z-[55] bg-black/40 backdrop-blur-md"
        onClick={props.onClose}
      />
      <div
        class="outl-sheet-up fixed inset-x-0 bottom-0 z-[55] flex flex-col"
        style={{
          "padding-bottom": "max(env(safe-area-inset-bottom), 16px)",
          transform: `translateY(${drag.translateY()}px)`,
          transition: drag.dragging()
            ? "none"
            : "transform 220ms var(--ease-spring-in)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <div class="mx-3 mb-2 overflow-hidden rounded-2xl bg-(--color-outl-bg-elev)/95 shadow-[var(--shadow-capsule)] backdrop-blur-2xl dark:shadow-[var(--shadow-capsule-dark)]">
          <span
            class="block py-2"
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

          <div class="px-4 pb-1 pt-1">
            <span class="text-[13px] font-semibold uppercase tracking-wide text-(--color-outl-fg-dim)">
              Reminders
            </span>
          </div>

          {/* This sheet is the only place mobile can turn delivery on
              (there is no settings screen, and config.toml lives inside
              the iOS sandbox). Saying "it's off" without a switch right
              here would be a dead end. */}
          <div class="flex items-center justify-between gap-3 border-t border-(--color-outl-border)/30 px-4 py-2.5">
            <span class="text-[13px] text-(--color-outl-fg)">
              Notify me on this device
              <Show when={!enabled()}>
                <span class="block text-[11px] text-(--color-outl-fg-dim)">
                  Rules below are tracked either way.
                </span>
              </Show>
            </span>
            <button
              type="button"
              role="switch"
              aria-checked={enabled()}
              aria-label="Notify me on this device"
              disabled={savingSettings()}
              onClick={() => void saveSettings(!enabled(), quietHours())}
              class="relative h-[31px] w-[51px] shrink-0 rounded-full transition-colors disabled:opacity-50"
              classList={{
                "bg-(--color-outl-accent)": enabled(),
                "bg-(--color-outl-border)": !enabled(),
              }}
            >
              <span
                class="absolute top-[2px] h-[27px] w-[27px] rounded-full bg-white shadow transition-all"
                style={{ left: enabled() ? "22px" : "2px" }}
              />
            </button>
          </div>

          {/* Native time pickers rather than the desktop's text field:
              typing "22:00-07:00" on a phone means switching keyboard
              layouts twice, and iOS renders these as a wheel. Clearing
              either end turns quiet hours off. */}
          <Show when={enabled()}>
            <div class="flex items-center justify-between gap-3 border-t border-(--color-outl-border)/30 px-4 py-2.5">
              <span class="text-[13px] text-(--color-outl-fg)">
                Quiet hours
                <span class="block text-[11px] text-(--color-outl-fg-dim)">
                  A fire lands after the window, never dropped.
                </span>
              </span>
              <div class="flex shrink-0 items-center gap-1">
                <input
                  type="time"
                  aria-label="Quiet hours start"
                  disabled={savingSettings()}
                  value={splitQuietHours(quietHours())[0]}
                  onChange={(e) => setQuietEnd(0, e.currentTarget.value)}
                  class="rounded-lg bg-(--color-outl-border)/40 px-2 py-1 text-[13px] text-(--color-outl-fg) disabled:opacity-50"
                />
                <span class="text-[13px] text-(--color-outl-fg-dim)">
                  to
                </span>
                <input
                  type="time"
                  aria-label="Quiet hours end"
                  disabled={savingSettings()}
                  value={splitQuietHours(quietHours())[1]}
                  onChange={(e) => setQuietEnd(1, e.currentTarget.value)}
                  class="rounded-lg bg-(--color-outl-border)/40 px-2 py-1 text-[13px] text-(--color-outl-fg) disabled:opacity-50"
                />
              </div>
            </div>
          </Show>

          <div class="max-h-[60vh] overflow-y-auto">
            <For each={groupReminders(reminders())}>
              {(group) => (
                <>
                  <div class="border-t border-(--color-outl-border)/30 bg-(--color-outl-border)/10 px-4 py-1.5 text-[12px] font-semibold uppercase tracking-wide text-(--color-outl-fg-dim)">
                    {group.label}
                  </div>
                  <For each={group.items}>
                    {(r) => (
                      <div
                        class="border-t border-(--color-outl-border)/30 px-4 py-3"
                        classList={{ "opacity-50": r.done }}
                      >
                        <button
                          type="button"
                          class="flex w-full items-baseline gap-2 text-left"
                          onClick={() => void jumpTo(r)}
                        >
                          <span class="min-w-0 flex-1 truncate text-[16px] text-(--color-outl-fg)">
                            {r.text || "(empty block)"}
                          </span>
                          {/* Overdue reads very differently from
                              upcoming; every task app paints it. */}
                          <span
                            class="shrink-0 text-[12px]"
                            classList={{
                              "text-red-500 font-medium": r.urgency === "overdue",
                              "text-(--color-outl-fg-dim)":
                                r.urgency !== "overdue",
                            }}
                          >
                            {formatNextFire(r.next_fire)}
                          </span>
                        </button>
                        <div class="mt-0.5 flex gap-2 text-[11px] text-(--color-outl-fg-dim)/80">
                          <span class="truncate">{r.page_title}</span>
                          <span class="font-mono">{r.rule}</span>
                        </div>
                        <Show when={!r.done}>
                          <div class="mt-2 flex flex-wrap gap-1.5">
                            <For each={presets()}>
                              {(p) => (
                                <button
                                  type="button"
                                  disabled={busy() === r.block_id}
                                  class="rounded-full bg-(--color-outl-border)/40 px-2.5 py-1 text-[12px] text-(--color-outl-fg) active:opacity-60 disabled:opacity-40"
                                  onClick={() =>
                                    void withRow(r.block_id, () =>
                                      snoozeReminder(r.block_id, p.id),
                                    )
                                  }
                                >
                                  {p.label}
                                </button>
                              )}
                            </For>
                            <Show when={r.snoozed_until}>
                              <button
                                type="button"
                                disabled={busy() === r.block_id}
                                class="rounded-full bg-(--color-outl-border)/40 px-2.5 py-1 text-[12px] text-(--color-outl-fg) active:opacity-60 disabled:opacity-40"
                                onClick={() =>
                                  void withRow(r.block_id, () =>
                                    clearReminderSnooze(r.block_id),
                                  )
                                }
                              >
                                Resume
                              </button>
                            </Show>
                            {/* Completing the task is the real "stop
                                nagging me", and the desktop panel has
                                had it since day one. */}
                            <button
                              type="button"
                              disabled={busy() === r.block_id}
                              class="rounded-full bg-(--color-outl-border)/40 px-2.5 py-1 text-[12px] text-(--color-outl-fg) active:opacity-60 disabled:opacity-40"
                              onClick={() =>
                                void withRow(r.block_id, () => markDone(r))
                              }
                            >
                              Done
                            </button>
                          </div>
                        </Show>
                      </div>
                    )}
                  </For>
                </>
              )}
            </For>

            <Show when={!loading() && reminders().length === 0}>
              <div class="border-t border-(--color-outl-border)/30 px-4 py-6 text-center text-[14px] text-(--color-outl-fg-dim)">
                No reminders yet. Long-press a TODO and pick{" "}
                <span class="font-medium">Remind me…</span>
              </div>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
}
