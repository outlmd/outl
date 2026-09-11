/**
 * Wire shapes of the plugin host commands (`plugin_list` / `plugin_run` /
 * `plugin_sync_hooks` / `plugin_toolbar` / `plugin_transformers` /
 * `plugin_transform` / `plugin_settings_describe` / `plugin_registry_list`).
 *
 * Split out of the sibling `types.ts` on size, not on meaning: every
 * consumer still imports these from `@outl/shared/api/types`, which
 * re-exports the whole set. Both GUI clients register the identical Rust
 * commands (thin shims over `PluginService`), so the DTOs have one owner.
 *
 * The desktop-only `PluginKeybinding` (chord surface) stays in
 * `outl-desktop/src/lib/api.ts` — mobile has no keybindings.
 *
 * Same rule as `types.ts`: every shape here mirrors a `serde`-serialized
 * Rust type, the mirror is hand-written, and
 * `outl-tauri-shared/tests/wire_types.rs` is what makes that safe. This
 * file is listed in that test's `ts_parser::MIRROR_FILES`.
 */

import type { PageView } from "./types";

/** A command a loaded plugin contributes — surfaced in the plugin
 *  palette (desktop) / plugin sheet (mobile). */
export interface PluginCommand {
  plugin_id: string;
  command_id: string;
  title: string;
}

/**
 * A toolbar button a loaded plugin contributes to the client chrome.
 * `icon` is the glyph painted inline; clicking / tapping it runs
 * `command_id` via `pluginRun`. `title` is the accessible label /
 * tooltip.
 */
export interface PluginToolbarButton {
  plugin_id: string;
  command_id: string;
  icon: string;
  title?: string;
}

/**
 * Outcome of running a plugin command. `view` is the refreshed
 * {@link PageView} of the page that was on screen when the command
 * fired (so the caller re-renders in one trip); absent when no page id
 * was supplied or the page no longer resolves.
 *
 * `views` carries HTML documents the plugin emitted via
 * `ctx.ui.render` (gated by the `ui-render` capability). Each is
 * played as an ephemeral sandboxed iframe overlay — untrusted plugin
 * output, never injected into the app DOM.
 */
export interface PluginRunReply {
  applied: number;
  notifications: string[];
  errors: string[];
  view?: PageView;
  views: string[];
}

/**
 * Outcome of the plugins' `onOp` hook sweep: a refreshed
 * {@link PageView} **only** when a hook actually mutated the on-screen
 * page (`view` absent otherwise, so the caller skips a needless
 * render), plus any `ui-render` payloads the hooks emitted (`views` —
 * the confetti path, present even when no page re-render is needed).
 */
export interface PluginSyncHooksReply {
  view?: PageView;
  views: string[];
  /** Projection failures after hook mutations were already saved. */
  errors: string[];
}

/**
 * A content transformer a loaded plugin declared for a code-fence
 * language. Clients load the list once per workspace open and, when a
 * fence's language matches a `lang` here, call `pluginTransform` to
 * render it.
 *
 * `kind` decides how the result renders inline in the block:
 * - `"text"` → the `content` is markdown/plain text, rendered inline.
 * - `"rich"` → the `content` is HTML, run in a sandboxed `<iframe>`.
 */
export interface PluginTransformer {
  plugin_id: string;
  lang: string;
  kind: "text" | "rich";
}

/**
 * The descriptor a content transformer produced for a fence body.
 * `kind` mirrors the matching {@link PluginTransformer.kind};
 * `content` is the rendered text (for `"text"`) or HTML run in a
 * sandboxed iframe (for `"rich"` — untrusted plugin output, never
 * injected into the app DOM).
 */
export interface PluginTransformResult {
  kind: "text" | "rich";
  content: string;
}

/** The value type of a plugin settings field (from its config schema). */
export type PluginFieldKind = "string" | "integer" | "number" | "boolean" | "json";

/**
 * One configurable field of a plugin, from `plugin_settings_describe`. Wire
 * shape of `outl_plugins::settings::SettingsField`. Config fields carry their
 * current `value`; secret fields carry only `isSet` (the value stays in the OS
 * keychain and never crosses the wire).
 */
export interface PluginSettingsField {
  /** Property key (`ctx.config.get()[key]` / `ctx.secrets.get(key)`). */
  key: string;
  /** Human label (schema `title`, falling back to the key). */
  title: string;
  /** Help text (schema `description`), when present. */
  description?: string;
  /** Value type. */
  kind: PluginFieldKind;
  /** Whether the field is keychain-backed (schema `x-outl-secret`). */
  secret: boolean;
  /** Schema default, when declared. */
  default?: unknown;
  /** Current config value (config fields only; absent when unset). */
  value?: unknown;
  /** For secret fields: whether a value is stored in the keychain. */
  isSet: boolean;
}

/**
 * One plugin marketplace row: a registry entry (plugins.outl.app) plus the
 * workspace's local state. Wire shape of `outl_plugins::MarketplaceItem`,
 * returned by `plugin_registry_list` on both clients. `installed` / `enabled`
 * drive the install vs. manage affordances.
 */
export interface RegistryItem {
  id: string;
  name: string;
  description: string;
  author: string | null;
  category: string | null;
  capabilities: string[];
  permissions: string[];
  latest: string | null;
  installed: boolean;
  enabled: boolean;
}
