/**
 * Actions the phone's switcher lists, the mobile half of the command
 * palette (`Action::OpenCommandPalette`). Matching and limits belong to
 * `@outl/shared/palette`; this file only declares the rows.
 *
 * The phone has no chords, so a row is a label and what it opens.
 * Labels match the shortcut catalog's descriptions where one exists, so
 * the same words find the same thing on both clients.
 */
import type { PaletteActionRow } from "@outl/shared/palette";

export const MOBILE_ACTIONS = [
  { kind: "action", action: "OpenToday", label: "Open today's journal", chord: null, keywords: "journal day" },
  { kind: "action", action: "QuickCapture", label: "Quick capture to today", chord: null, keywords: "add note" },
  { kind: "action", action: "TaskList", label: "Open task list", chord: null, keywords: "todo doing" },
  { kind: "action", action: "OpenReminders", label: "Open reminders", chord: null, keywords: "remind notifications" },
  { kind: "action", action: "OpenCalendar", label: "Open calendar", chord: null, keywords: "date month" },
  { kind: "action", action: "SyncNow", label: "Sync now", chord: null, keywords: "refresh pull" },
  { kind: "action", action: "OpenDevices", label: "Devices and sync", chord: null, keywords: "pending merged peers trust history" },
  { kind: "action", action: "OpenPlugins", label: "Plugins", chord: null, keywords: "marketplace extensions" },
  { kind: "action", action: "OpenSettings", label: "Open settings", chord: null, keywords: "preferences theme focus export" },
  { kind: "action", action: "FoldAll", label: "Fold all blocks", chord: null, keywords: "collapse" },
  { kind: "action", action: "UnfoldAll", label: "Unfold all blocks", chord: null, keywords: "expand" },
] as const satisfies readonly PaletteActionRow[];

/** The list is the source of the union: an action with no row cannot exist. */
export type MobileAction = (typeof MOBILE_ACTIONS)[number]["action"];
