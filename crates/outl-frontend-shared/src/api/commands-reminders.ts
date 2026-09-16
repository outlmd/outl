/**
 * `remind::` wrappers, plus the two pure display helpers over them.
 *
 * Split out of `commands.ts` under the file-size ratchet. The rule
 * that matters here did not move: **when** a reminder fires is decided
 * once, in `outl_actions::reminders`, for every client — these
 * wrappers only carry the answer across the bridge. Re-exported from
 * `commands.ts`, which stays the import path every client uses.
 */

import { invoke } from "@tauri-apps/api/core";

import type { PageView, Reminder, ReminderSettings, SnoozePreset } from "./types";

// ── Reminders (`remind::`) ──────────────────────────────────────────
//
// The schedule itself is decided in Rust (`outl_actions::reminders`),
// once, for every client. These wrappers only move the answer across
// the bridge — never re-derive "when does this fire" in TS.

/** Every reminder in the workspace, soonest first; finished ones last. */
export function listReminders(): Promise<Reminder[]> {
  return invoke<Reminder[]>("list_reminders");
}

/** This device's reminder delivery preferences. */
export function reminderSettings(): Promise<ReminderSettings> {
  return invoke<ReminderSettings>("reminder_settings");
}

/**
 * Silence a block's reminder for `minutes` from now.
 *
 * Writes `Op::SnoozeRemind`, so the same block goes quiet on every
 * paired device — snoozing on the phone must not leave the laptop
 * buzzing.
 */
export function snoozeReminder(blockId: string, preset: string): Promise<void> {
  return invoke<void>("snooze_reminder", { blockId, preset });
}

/**
 * The snooze options, in render order.
 *
 * Fetched rather than hardcoded because two of the three aren't fixed
 * offsets — "tomorrow 9am" is a wall time — so a client doing its own
 * arithmetic gets them wrong. Render `label`, send back `id`.
 */
export function snoozePresets(): Promise<SnoozePreset[]> {
  return invoke<SnoozePreset[]>("snooze_presets");
}

/** Clear a snooze so the block resumes its normal schedule. */
export function clearReminderSnooze(blockId: string): Promise<void> {
  return invoke<void>("clear_reminder_snooze", { blockId });
}

/**
 * Set — or clear, with an empty `rule` — a block's `remind::` property.
 * Returns the refreshed page. Editing the rule reschedules from
 * scratch, which falls out for free: the schedule is derived on every
 * scan, never cached.
 */
export function setBlockRemind(
  pageId: string,
  blockId: string,
  rule: string,
): Promise<PageView> {
  return invoke<PageView>("set_block_remind", { pageId, blockId, rule });
}

/**
 * Mark a block DONE, cancelling every pending fire of its rule.
 *
 * Not `toggleTodo`. A rule can sit on a block with no TODO marker at
 * all, and toggling that advances it to `TODO` — so the reminders
 * list's "mark done" button used to arm the nag instead of cancelling
 * it. Setting the state outright is also idempotent, which matters
 * for a button that can be double-tapped.
 */
export function markBlockDone(
  pageId: string,
  blockId: string,
): Promise<PageView> {
  return invoke<PageView>("mark_block_done", { pageId, blockId });
}

/**
 * Deliver every reminder that came due as an OS notification, and
 * return what was delivered so an open Reminders panel can refresh
 * without a second round trip.
 *
 * Safe to call on a plain interval: it short-circuits when the device
 * has reminders switched off, and the Rust side keeps the device-local
 * "already fired" log so polling twice never double-buzzes.
 */
export function deliverDueReminders(): Promise<Reminder[]> {
  return invoke<Reminder[]>("deliver_due_reminders");
}

/**
 * How long until `iso` (a local ISO datetime from {@link Reminder}),
 * as a short human label: `"now"`, `"in 20min"`, `"in 3h"`,
 * `"tomorrow 09:00"`, `"Dec 15, 10:00"`.
 *
 * Shared because both the desktop panel and the mobile list show the
 * same column, and two implementations of "in 3h" drift on the edge
 * cases (exactly 60 minutes, midnight rollover) before anyone notices.
 */
export function formatNextFire(iso: string | null, now: Date = new Date()): string {
  if (!iso) return "—";
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "—";
  const minutes = Math.round((at.getTime() - now.getTime()) / 60000);
  if (minutes <= 0) return "now";
  if (minutes < 60) return `in ${minutes}min`;
  const hhmm = `${String(at.getHours()).padStart(2, "0")}:${String(
    at.getMinutes(),
  ).padStart(2, "0")}`;
  // Same calendar day → the relative form reads better than a clock time.
  if (at.toDateString() === now.toDateString()) {
    return `in ${Math.round(minutes / 60)}h`;
  }
  const tomorrow = new Date(now);
  tomorrow.setDate(tomorrow.getDate() + 1);
  if (at.toDateString() === tomorrow.toDateString()) return `tomorrow ${hhmm}`;
  return `${at.toLocaleDateString(undefined, { month: "short", day: "numeric" })}, ${hhmm}`;
}

/**
 * Bucket a reminder list into the groups every client renders:
 * Today / Tomorrow / This week / Later / Done. Order is stable and
 * empty buckets are dropped, so a client can map straight over it.
 */
export function groupReminders(
  reminders: readonly Reminder[],
  now: Date = new Date(),
): { label: string; items: Reminder[] }[] {
  const buckets: Record<string, Reminder[]> = {
    Today: [],
    Tomorrow: [],
    "This week": [],
    Later: [],
    Done: [],
  };
  const startOfDay = (d: Date) =>
    new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const today = startOfDay(now);
  const day = 86_400_000;

  for (const r of reminders) {
    if (!r.next_fire) {
      buckets.Done.push(r);
      continue;
    }
    const at = new Date(r.next_fire);
    const delta = startOfDay(at) - today;
    if (delta <= 0) buckets.Today.push(r);
    else if (delta === day) buckets.Tomorrow.push(r);
    else if (delta < 7 * day) buckets["This week"].push(r);
    else buckets.Later.push(r);
  }
  return Object.entries(buckets)
    .filter(([, items]) => items.length > 0)
    .map(([label, items]) => ({ label, items }));
}

/**
 * Write this device's reminder settings.
 *
 * Mobile has no settings screen, so the Reminders sheet is the only
 * place that can turn delivery on. Without this the sheet could say
 * "notifications are off on this device" and leave the user with no
 * way to change it, since `config.toml` sits inside the iOS sandbox.
 */
export function setReminderSettings(
  enabled: boolean,
  quietHours: string,
): Promise<ReminderSettings> {
  return invoke<ReminderSettings>("set_reminder_settings", {
    enabled,
    quietHours,
  });
}
