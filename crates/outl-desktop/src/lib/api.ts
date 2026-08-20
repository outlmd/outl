/**
 * Tauri command wrappers for `outl-desktop`.
 *
 * Shared commands (every client uses identical: navigation, mutations,
 * paste) are **not** redeclared here — import them directly from
 * `@outl/shared/api/commands`. This file is reserved for commands the
 * desktop client adds on top: workspace picker, settings, and the
 * code execution wrapper.
 */
import { invoke } from "@tauri-apps/api/core";

import type { PageView, WorkspaceSummary } from "@outl/shared/api/types";
import type { DeepLinkNavigate } from "./events";

/**
 * Take (and clear) an `outl://` deep link that arrived during cold
 * start — i.e. a URL that *launched* the app, before the `AppShell`
 * mounted its `deep-link://navigate` listener (issue #98). Returns
 * `null` on a normal launch. Call once on `AppShell` mount; the warm
 * path (app already running) is handled by the live event listener.
 */
export function takePendingDeepLink(): Promise<DeepLinkNavigate | null> {
  return invoke<DeepLinkNavigate | null>("take_pending_deep_link");
}

// ---------------------------------------------------------------------------
// Workspace lifecycle (desktop-only)
// ---------------------------------------------------------------------------

/**
 * Open the workspace rooted at `path`. The backend creates the
 * `ops/`, `journals/`, `pages/` directories if missing, opens the
 * JsonlStorage, runs the legacy migration + orphan reconcile, and
 * persists the choice in `settings.json`.
 *
 * Emits `workspace-ready` when complete — wire `onWorkspaceReady`
 * before calling this so the UI refreshes when the swap lands.
 */
export function setWorkspace(path: string): Promise<void> {
  return invoke<void>("set_workspace", { path });
}

/**
 * Current workspace path, or `null` when the user hasn't picked
 * one yet (first launch, or `last_workspace` no longer exists on
 * disk).
 */
export function currentWorkspace(): Promise<string | null> {
  return invoke<string | null>("current_workspace");
}

/**
 * Re-export of the shared `workspaceStats()` wrapper — kept here for
 * convenience so feature code can import everything desktop-shaped
 * from one file. The DTO is the shared `WorkspaceSummary` (with
 * `ready: boolean`).
 */
export async function workspaceStats(): Promise<WorkspaceSummary> {
  return invoke<WorkspaceSummary>("workspace_stats");
}

// ---------------------------------------------------------------------------
// Code execution (desktop-only)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Undo / redo (desktop-only)
// ---------------------------------------------------------------------------

/**
 * Revert the last committed block mutation on the page. Rejects with
 * `"nothing to undo"` when the page's history stack is empty — the
 * handler surfaces that as a status message, not a crash.
 */
export function undoPage(pageId: string): Promise<PageView> {
  return invoke<PageView>("undo_page", { pageId });
}

/** Re-apply the mutation the last {@link undoPage} reverted. */
export function redoPage(pageId: string): Promise<PageView> {
  return invoke<PageView>("redo_page", { pageId });
}

// ---------------------------------------------------------------------------
// Properties (`key:: value`)
// ---------------------------------------------------------------------------
//
// `setBlockProperty` is shared (`@outl/shared/api/commands`) — both
// GUI clients have registered it since the `remind::` work. These two
// are registered on the desktop today and belong beside it the moment
// mobile picks them up; see `PropertyEditor.tsx`'s note.

// `PropertyKey` is a wire type both GUI clients share — it lives in
// `@outl/shared/api/types`, re-exported here so existing imports from
// this module keep resolving.
export type { PropertyKey } from "@outl/shared/api/types";

/**
 * Property keys used anywhere in the workspace, most-used first.
 *
 * Deliberately **not** cached: the backend answer is a scan of the
 * property map (no tree walk, no block text), so it is cheaper to ask
 * every time an editor opens than to hold a list that goes stale the
 * first time the user adds a key.
 */
export function knownPropertyKeys(): Promise<PropertyKey[]> {
  return invoke<PropertyKey[]>("known_property_keys");
}

/**
 * Set — or, with an empty `value`, clear — a property on the **page**
 * itself (`icon::`, `type::`, …). Rejects the structural keys
 * (`page-slug`, `page-kind`): those are the page's identity and
 * renaming is `page_rename`, not a property edit.
 */
export function setPageProperty(
  pageId: string,
  key: string,
  value: string,
): Promise<PageView> {
  return invoke<PageView>("set_page_property", { pageId, key, value });
}

// ---------------------------------------------------------------------------
// Settings (desktop-only)
// ---------------------------------------------------------------------------

export interface Settings {
  last_workspace: string | null;
  vim_mode: boolean;
  /**
   * Name of the active palette preset. Matches one of
   * `outl_theme::PRESETS` (`"outl"`, `"dracula"`, `"nord"`, …) so
   * the desktop renders identical hues to the TUI / mobile.
   */
  theme: string;
  font_size: number;
  /**
   * Sync transport: `"iroh"` (direct P2P over QUIC, the default) or
   * `"file"` (iCloud Drive / shared filesystem). Mirrors the Rust
   * `Settings.sync_transport` and the `[sync] transport` config key.
   */
  sync_transport: string;
  /**
   * Backlinks list direction: `"newest"` (default) or `"oldest"`
   * (issue #142). Read-only in the Settings modal — the backlinks
   * toggle writes it via `set_backlinks_order`.
   */
  backlinks_order: string;
  /**
   * Whether this device turns `remind::` rules into OS notifications.
   * Defaults **on** — writing `remind::` on a block is already the
   * opt-in, and a device with no rules never fires. Device-local, it
   * never travels through the op log.
   */
  reminders_enabled: boolean;
  /**
   * Quiet-hours window as `"22:00-07:00"`, or `""` for none. A fire
   * landing inside it is pushed to the window's end, never dropped.
   */
  reminders_quiet_hours: string;
}

/**
 * Palette returned by `get_theme`. Mirrors `outl_theme::Palette`
 * field-for-field — every value is a `#rrggbb` (or `#rrggbbaa`)
 * string that
 * {@link applyPaletteToRoot | the frontend installer} writes as
 * a CSS custom property.
 */
export interface Palette {
  name: string;
  bg: string;
  bg_elev: string;
  fg: string;
  fg_dim: string;
  fg_dimmer: string;
  border: string;
  hint: string;
  accent: string;
  accent_soft: string;
  accent_alt: string;
  warn: string;
  ref_link_fg: string;
  tag_link_fg: string;
  md_link_fg: string;
  bold_fg: string;
  italic_fg: string;
  strike_fg: string;
  code_fg: string;
  todo_open_fg: string;
  todo_done_fg: string;
  todo_done_body_fg: string;
  property_key_fg: string;
  property_value_fg: string;
  heading_fg: string;
  dim_fg: string;
  selected_bullet_bg: string;
  selected_bullet_fg: string;
  cursor_block_bg: string;
  cursor_block_fg: string;
  cursor_caret_fg: string;
  status_normal_bg: string;
  status_normal_fg: string;
  status_insert_bg: string;
  status_insert_fg: string;
  status_visual_bg: string;
  status_visual_fg: string;
  status_message_fg: string;
  list_selected_bg: string;
  list_selected_fg: string;
  help_title_fg: string;
}

export function listThemes(): Promise<string[]> {
  return invoke<string[]>("list_themes");
}

export function getTheme(name: string | null): Promise<Palette> {
  return invoke<Palette>("get_theme", { name });
}

// ---------------------------------------------------------------------------
// Shortcuts (mirrors outl_shortcuts::{Action, Chord, Binding, Mode})
// ---------------------------------------------------------------------------

/** Modifier bitflags — match `outl_shortcuts::chord::Modifiers`. */
export const MOD_CTRL = 0b0001;
export const MOD_ALT = 0b0010;
export const MOD_SHIFT = 0b0100;
export const MOD_META = 0b1000;

export type ShortcutMode =
  | "global"
  | "normal"
  | "insert"
  | "visual"
  | "overlay";

/** Chord key — tagged union mirror of `outl_shortcuts::chord::Key`. */
export type Key =
  | { kind: "Char"; value: string }
  | { kind: "Enter" }
  | { kind: "Esc" }
  | { kind: "Tab" }
  | { kind: "Backspace" }
  | { kind: "Delete" }
  | { kind: "Up" }
  | { kind: "Down" }
  | { kind: "Left" }
  | { kind: "Right" }
  | { kind: "Home" }
  | { kind: "End" }
  | { kind: "PageUp" }
  | { kind: "PageDown" }
  | { kind: "Space" }
  | { kind: "Function"; value: number };

export interface Chord {
  /** Bitflag combination of `MOD_*` constants. */
  mods: number;
  key: Key;
}

/** Action discriminant — string `kind` mirrors Rust `Action` variants. */
export type Action =
  | { kind: "OpenPicker" }
  | { kind: "OpenCommandPalette" }
  | { kind: "ToggleHelp" }
  | { kind: "ToggleSidebar" }
  | { kind: "ToggleBacklinks" }
  // Reminders (`remind::`) — authoring + the panel. Delivery is
  // per-OS and never a chord.
  | { kind: "InsertRemind" }
  | { kind: "InsertRemindNag" }
  | { kind: "OpenReminders" }
  | { kind: "SnoozeReminder" }
  // Properties (`key:: value`) — the generic door `remind::` chords
  // are one special case of. `TogglePin` is TUI-only; it is listed so
  // the mirror of the Rust enum stays complete (the desktop fetches
  // the *whole* catalog and an unlisted `kind` is silent drift).
  | { kind: "AddProperty" }
  | { kind: "OpenProperties" }
  | { kind: "TogglePin" }
  | { kind: "OpenSettings" }
  | { kind: "Quit" }
  | { kind: "OpenToday" }
  | { kind: "PrevDay" }
  | { kind: "NextDay" }
  | { kind: "SelectionDown" }
  | { kind: "SelectionUp" }
  | { kind: "OpenRefUnderCursor" }
  | { kind: "EnterInsert" }
  | { kind: "EnterInsertAtStart" }
  | { kind: "EnterInsertAfter" }
  | { kind: "EnterInsertAtEnd" }
  | { kind: "DeleteCharUnderCursor" }
  | { kind: "DeleteCharBeforeCursor" }
  | { kind: "DeleteToEndOfBlock" }
  | { kind: "ChangeToEndOfBlock" }
  | { kind: "SubstituteBlock" }
  | { kind: "SubstituteChar" }
  | { kind: "ReplaceChar" }
  | { kind: "FindCharForward" }
  | { kind: "FindCharBackward" }
  | { kind: "ToggleCharCase" }
  | { kind: "CursorWordEnd" }
  | { kind: "UnfoldAll" }
  | { kind: "FoldAll" }
  | { kind: "ZoomIn" }
  | { kind: "ZoomOut" }
  | { kind: "CenterViewport" }
  | { kind: "SearchWordForward" }
  | { kind: "SearchWordBackward" }
  | { kind: "ReselectLastVisual" }
  | { kind: "IndentVisualRange" }
  | { kind: "OutdentVisualRange" }
  | { kind: "NewBlockBelow" }
  | { kind: "NewBlockAbove" }
  | { kind: "IndentBlock" }
  | { kind: "OutdentBlock" }
  | { kind: "MoveBlockUp" }
  | { kind: "MoveBlockDown" }
  | { kind: "DeleteBlock" }
  | { kind: "DeletePage" }
  | { kind: "ToggleCollapsed" }
  | { kind: "ToggleTodo" }
  | { kind: "CopyBlockRef" }
  | { kind: "CutBlock" }
  | { kind: "CopyBlock" }
  | { kind: "PasteBlock" }
  | { kind: "ExitInsert" }
  | { kind: "CommitAndContinue" }
  | { kind: "DeleteEmptyBlock" }
  | { kind: "EnterVisual" }
  | { kind: "YankCurrentBlock" }
  | { kind: "YankRange" }
  | { kind: "DeleteRange" }
  | { kind: "SelectRangeDown" }
  | { kind: "SelectRangeUp" }
  | { kind: "MoveVisualRangeUp" }
  | { kind: "MoveVisualRangeDown" }
  | { kind: "RunCodeBlock" }
  | { kind: "Undo" }
  | { kind: "Redo" }
  | { kind: "WrapBold" }
  | { kind: "WrapItalic" }
  | { kind: "WrapCode" }
  | { kind: "WrapStrike" }
  | { kind: "InsertLink" };

export interface Binding {
  /** `ChordSequence` — array of one (single chord) or two (vim-style `g j`). */
  chord: Chord[];
  mode: ShortcutMode;
  action: Action;
  description: string;
}

/**
 * Fetch the full binding catalog from the backend. Cached after the
 * first call (bindings never change at runtime today); a future
 * config-reload path can invalidate.
 */
export function listShortcutBindings(): Promise<Binding[]> {
  return invoke<Binding[]>("list_shortcut_bindings");
}

export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

export function updateSettings(next: Settings): Promise<Settings> {
  return invoke<Settings>("update_settings", { next });
}

// `runCodeBlock` + the `ExecOutputDto` / `RunCodeBlockReply` DTOs
// moved to `@outl/shared/api/commands` once mobile picked up the same
// command (v0.6.x — long-press → "Run code"). Re-exported here so
// every desktop caller keeps importing from one place.
export type { ExecOutputDto, RunCodeBlockReply } from "@outl/shared/api/types";
export { runCodeBlock } from "@outl/shared/api/commands";

// ---------------------------------------------------------------------------
// Plugins (desktop-only surface)
// ---------------------------------------------------------------------------
//
// The plugin host (`outl_plugins::PluginHost`) embeds a Boa `Context` that is
// `!Send`, so it runs on a dedicated thread behind `PluginService` (see
// `src-tauri/src/plugin_service.rs`).
//
// The client-agnostic plugin surface lives in `@outl/shared`: the DTOs
// (`PluginCommand`, `PluginToolbarButton`, `PluginRunReply`,
// `PluginSyncHooksReply`, `PluginTransformer`, `PluginTransformResult`) in
// `@outl/shared/api/types`, the wrappers (`pluginList`, `pluginRun`,
// `pluginSyncHooks`, `pluginToolbar`, `pluginTransformers`,
// `pluginTransform`) in `@outl/shared/api/commands`, and the marketplace
// (`RegistryItem`, `pluginRegistryList`, …, `filterRegistryItems`)
// alongside them — both clients register identical commands.
//
// Only the **keybinding** contribution stays here: mobile has no chord
// surface, so `plugin_keybindings` is a desktop-only command.

/**
 * A keybinding a loaded plugin contributes for the desktop.
 *
 * `chord` and `mode` serialize **identically** to the `outl-shortcuts`
 * catalog ({@link Binding}) — `chord` is a `Chord[]` (`ChordSequence` is
 * `#[serde(transparent)]` over `Vec<Chord>`), `mode` is a lowercase
 * {@link ShortcutMode}. The dispatcher in `lib/shortcuts.ts` reuses the
 * same `seqEq` comparison it already runs against native bindings, so an
 * `eventToChord(e)` matches a plugin chord byte-for-byte the way it
 * matches a native one. Plugin chords are always `"global"`.
 */
export interface PluginKeybinding {
  chord: Chord[];
  mode: ShortcutMode;
  plugin_id: string;
  command_id: string;
  description: string;
}

/**
 * List plugin-contributed desktop keybindings. The dispatcher folds these
 * into the chord pipeline as a Global overlay that only fires when **no**
 * native binding already owns the chord (native wins). Empty until plugins
 * load (best-effort — never throws).
 */
export function pluginKeybindings(): Promise<PluginKeybinding[]> {
  return invoke<PluginKeybinding[]>("plugin_keybindings");
}
