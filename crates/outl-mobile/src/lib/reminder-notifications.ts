/**
 * Register the banner category on boot, then route what the user
 * pressed into `reminder-actions.ts`.
 *
 * **Never move this to `@outl/shared`:** the command, the registration
 * and the `actionPerformed` event are all mobile-only, so a shared
 * wrapper would hand the desktop an IPC call that fails.
 * `crates/outl-mobile/CLAUDE.md` → Reminders → Actionable banners.
 */

import { invoke } from "@tauri-apps/api/core";
import { onAction, registerActionTypes } from "@tauri-apps/plugin-notification";
import { markBlockDone, openPageBySlug, snoozeReminder } from "@outl/shared/api/commands";

import {
  handleReminderAction,
  type ReminderActionDeps,
  type ReminderActionPayload,
} from "./reminder-actions";

/** Mirrors `ReminderActionCatalog` in `src-tauri/src/commands/reminders.rs`. */
interface ReminderActionCatalog {
  category: string;
  actions: { id: string; title: string; kind: "snooze" | "done" }[];
}

/**
 * The two things only the component can supply. The three commands the
 * dispatcher calls are plain module imports — they stay injectable in
 * `reminder-actions.ts`, which is the half that has tests.
 */
export type ReminderNotificationDeps = Pick<
  ReminderActionDeps,
  "navigateToBlock" | "onError"
>;

/**
 * Register the banner category and start listening.
 *
 * Returns a cleanup for the listener. Registration itself is not
 * undone: categories live on `UNUserNotificationCenter` for the
 * process, and unregistering on unmount would strip the buttons off
 * banners that are still queued.
 *
 * Every failure here is swallowed after being reported once. A device
 * that cannot register categories still delivers reminders, still
 * lists them in the sheet, and is strictly better off than one that
 * refused to boot the journal over a missing button.
 */
export async function setupReminderNotifications(
  deps: ReminderNotificationDeps,
): Promise<() => void> {
  let catalog: ReminderActionCatalog;
  try {
    catalog = await invoke<ReminderActionCatalog>("reminder_action_catalog");
    await registerActionTypes([
      {
        id: catalog.category,
        // `foreground: false` on every button: resolving a reminder
        // without being pulled into the app is the whole point of one.
        // The plain tap is the path that foregrounds and navigates.
        actions: catalog.actions.map((a) => ({ id: a.id, title: a.title, foreground: false })),
      },
    ]);
  } catch {
    // Desktop dev build, or a platform without the command. Banners
    // still fire, they just arrive without buttons — which is exactly
    // what the capability catalog already tells the user.
    return () => {};
  }

  try {
    // The catalog goes through as-is: the same ids the OS was told
    // about, carrying the kind Rust declared, so the handler cannot
    // drift from what the banner is stamped with.
    const sub = await onAction((n) => {
      void handleReminderAction(n as ReminderActionPayload, {
        ...deps,
        snoozeReminder,
        openPageBySlug,
        markBlockDone,
        buttons: catalog.actions,
      });
    });
    return () => sub.unregister();
  } catch (e) {
    deps.onError(e instanceof Error ? e.message : String(e));
    return () => {};
  }
}
