/**
 * Plugin host + marketplace wrappers.
 *
 * Split out of `commands.ts` under the file-size ratchet. Both halves
 * talk to the same dedicated plugin thread on the Rust side (the Boa
 * host is `!Send`), so they belong together and apart from everything
 * else. Re-exported from `commands.ts`, which stays the import path
 * every client uses.
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  PluginCommand,
  PluginRunReply,
  PluginSettingsField,
  PluginSyncHooksReply,
  PluginToolbarButton,
  PluginTransformer,
  PluginTransformResult,
  RegistryItem,
} from "./types";

// ── Plugin host ─────────────────────────────────────────────────────
// Both GUI clients register identical `plugin_list` / `plugin_run` /
// `plugin_sync_hooks` / `plugin_toolbar` / `plugin_transformers` /
// `plugin_transform` commands (thin shims over `PluginService` — the
// Boa host is `!Send`, so it runs on a dedicated thread), so the
// wrappers live here once. The desktop-only `plugin_keybindings`
// stays in `outl-desktop/src/lib/api.ts` (mobile has no chord surface).

/**
 * List every command contributed by a loaded plugin. Empty until the
 * workspace opens and plugins load (best-effort — never throws on an
 * empty or failed host).
 */
export function pluginList(): Promise<PluginCommand[]> {
  return invoke<PluginCommand[]>("plugin_list");
}

/**
 * Run a plugin command. Pass the currently-open page id so the reply
 * carries its refreshed `PageView` — the plugin thread re-projects
 * every page's `.md` before returning (a plugin can move blocks across
 * pages).
 */
export function pluginRun(
  pluginId: string,
  commandId: string,
  pageId: string | null,
): Promise<PluginRunReply> {
  return invoke<PluginRunReply>("plugin_run", {
    pluginId,
    commandId,
    pageId,
  });
}

/**
 * Fire the plugins' `onOp` hook sweep after a user mutation. The
 * reply's `view` is the refreshed `PageView` of `pageId` **only** when
 * a hook actually mutated the workspace (absent otherwise, so the
 * caller skips a needless render); `views` carries any `ui-render`
 * HTML the hooks emitted (the confetti path — present even when
 * nothing was re-rendered). Best-effort — a host with no op-hook
 * plugins is a cheap no-op.
 */
export function pluginSyncHooks(
  pageId: string | null,
): Promise<PluginSyncHooksReply> {
  return invoke<PluginSyncHooksReply>("plugin_sync_hooks", { pageId });
}

/**
 * List every toolbar button a loaded plugin contributes to the client
 * chrome — one button per entry (glyph = `icon`, tooltip = `title`,
 * click / tap = {@link pluginRun}). Empty until plugins load
 * (best-effort — never throws).
 */
export function pluginToolbar(): Promise<PluginToolbarButton[]> {
  return invoke<PluginToolbarButton[]>("plugin_toolbar");
}

/**
 * List every content transformer a loaded plugin declared. Load once
 * per workspace open and match each code fence's language against the
 * result. Empty until plugins load (best-effort — never throws).
 */
export function pluginTransformers(): Promise<PluginTransformer[]> {
  return invoke<PluginTransformer[]>("plugin_transformers");
}

/**
 * Run a content transformer for `lang` against a fence `input` (its
 * body). Read-only: never mutates the workspace. Resolves to `null`
 * when the transformer declined or no plugin owns `lang`, otherwise
 * the `{ kind, content }` descriptor. Cache the result by
 * `(blockId, body)` — re-run only when the body changes (see
 * `@outl/shared/plugins/transformer-registry`).
 */
export function pluginTransform(
  pluginId: string,
  lang: string,
  input: string,
): Promise<PluginTransformResult | null> {
  return invoke<PluginTransformResult | null>("plugin_transform", {
    pluginId,
    lang,
    input,
  });
}

// ── Plugin marketplace ──────────────────────────────────────────────
// Both GUI clients register identical `plugin_registry_list` /
// `plugin_install_official` / `plugin_set_enabled` / `plugin_uninstall`
// commands on their src-tauri side, so the wrappers live here once.

/** Fetch the marketplace: the official registry crossed with the lockfile. */
export function pluginRegistryList(): Promise<RegistryItem[]> {
  return invoke<RegistryItem[]>("plugin_registry_list");
}

/** Tap-to-install an official plugin by id; resolves to its display name. */
export function pluginInstallOfficial(id: string): Promise<string> {
  return invoke<string>("plugin_install_official", { id });
}

/** Enable / disable an installed plugin. */
export function pluginSetEnabled(id: string, enabled: boolean): Promise<void> {
  return invoke<void>("plugin_set_enabled", { id, enabled });
}

/** Uninstall a plugin; resolves `true` if anything was removed. */
export function pluginUninstall(id: string): Promise<boolean> {
  return invoke<boolean>("plugin_uninstall", { id });
}

/**
 * Describe a plugin's settings form: every config/secret field with its type,
 * current value, and — for secrets — whether it is set (never the value).
 * Empty when the plugin declares no config schema.
 */
export function pluginSettingsDescribe(
  pluginId: string,
): Promise<PluginSettingsField[]> {
  return invoke<PluginSettingsField[]>("plugin_settings_describe", { pluginId });
}

/**
 * Set a plaintext config field. The host coerces the string to the field's
 * schema type and reloads the plugin so the change is live. Rejects secret
 * fields — use {@link pluginSecretSet}.
 */
export function pluginConfigSet(
  pluginId: string,
  key: string,
  value: string,
): Promise<void> {
  return invoke<void>("plugin_config_set", { pluginId, key, value });
}

/** Store a secret field's value in the OS keychain (never on disk). */
export function pluginSecretSet(
  pluginId: string,
  key: string,
  value: string,
): Promise<void> {
  return invoke<void>("plugin_secret_set", { pluginId, key, value });
}

/** Delete a secret field's value from the keychain (idempotent). */
export function pluginSecretRemove(pluginId: string, key: string): Promise<void> {
  return invoke<void>("plugin_secret_remove", { pluginId, key });
}

/**
 * Filter marketplace rows by a query (case-insensitive substring over id,
 * name, description, and capabilities). Empty query returns every item.
 * Pure — both the desktop modal and the mobile sheet derive their list from
 * it, so the match rule stays in one place.
 */
export function filterRegistryItems(
  items: readonly RegistryItem[],
  query: string,
): RegistryItem[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...items];
  return items.filter(
    (i) =>
      i.id.toLowerCase().includes(q) ||
      i.name.toLowerCase().includes(q) ||
      i.description.toLowerCase().includes(q) ||
      i.capabilities.some((c) => c.toLowerCase().includes(q)),
  );
}
