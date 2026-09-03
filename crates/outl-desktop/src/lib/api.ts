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

import type { WorkspaceSummary } from "@outl/shared/api/types";
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
// Undo / redo
// ---------------------------------------------------------------------------
//
// Moved to `@outl/shared/api/commands` (RFC 0254 phase 1 — mobile
// registers the same `undo_page` / `redo_page` commands now). Re-exported
// here for backward-compatible imports (same pattern as `runCodeBlock`).
export { redoPage, undoPage } from "@outl/shared/api/commands";

// ---------------------------------------------------------------------------
// Properties (`key:: value`)
// ---------------------------------------------------------------------------
//
// `knownPropertyKeys` / `setPageProperty` live in
// `@outl/shared/api/commands` next to `setBlockProperty` — both GUI
// clients register the same commands, so the wrappers have one owner.
// Re-exported here (with the `PropertyKey` wire type) so existing
// imports from this module keep resolving.
export type { PropertyKey } from "@outl/shared/api/types";
export {
  knownPropertyKeys,
  setPageProperty,
} from "@outl/shared/api/commands";

// ---------------------------------------------------------------------------
// Settings (desktop-only)
// ---------------------------------------------------------------------------

export interface Settings {
  last_workspace: string | null;
  vim_mode: boolean;
  /**
   * Name of the active palette preset. Matches one of
   * `outl_theme::PRESETS` (`"outl"`, `"dracula"`, `"nord"`, …) so
   * the desktop renders identical hues to the TUI / mobile. The
   * light side of the RFC 0022 pair, and the only side used when
   * `theme_mode === "light"`.
   */
  theme: string;
  /**
   * The dark side of the RFC 0022 pair, used when `theme_mode ===
   * "dark"` or `"auto"` resolves dark. Always a concrete preset name
   * (never empty) — the backend resolves it through `ThemeCfg::dark()`,
   * which falls back to `theme` for a config that never set a second
   * preset.
   */
  theme_dark: string;
  /**
   * Which side of the pair to render: `"light"`, `"dark"`, or
   * `"auto"` (default) to follow the OS appearance setting. Mirrors
   * `[theme] mode`.
   */
  theme_mode: string;
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

// `Palette`, `listThemes` and `getTheme` moved to `@outl/shared` (RFC 0022,
// Task 7): both GUI clients register the identical `get_theme` /
// `list_themes` commands, and `applyPaletteToRoot` needs the same `Palette`
// type they return. Import from `@outl/shared/api/commands` /
// `@outl/shared/api/types` instead.

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

/**
 * What a client does with an action — the wire form of
 * `outl_shortcuts::Support`.
 *
 * `why` is the text shown to the *user*, not a log line. It is
 * `null` for `full` and `native`, which are the two states where
 * there is nothing to explain.
 */
export interface SupportDto {
  kind: "full" | "native" | "partial" | "missing" | "n/a";
  why: string | null;
}

/** Per-client support for one action in the catalog. */
export interface ActionSupport {
  action: Action;
  tui: SupportDto;
  desktop: SupportDto;
  mobile: SupportDto;
}

/**
 * Fetch what every client does with every action.
 *
 * The desktop uses its own column to tell the user why a chord did
 * nothing. Before this existed the answer was a `console.warn`, and
 * a chord that is missing was indistinguishable from one that is
 * broken unless you had DevTools open.
 *
 * Single owner: `crates/outl-shortcuts/src/support.rs`. See
 * [`docs/client-parity.md`](../../../../docs/client-parity.md).
 */
export function listActionSupport(): Promise<ActionSupport[]> {
  return invoke<ActionSupport[]>("list_action_support");
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
