/**
 * Mobile-only Tauri command wrappers.
 *
 * Shared commands (navigation, mutations, paste, peers) live in
 * `@outl/shared/api/commands` — import those directly. This file is
 * reserved for commands the **mobile** client adds on top: today, that's
 * the workspace-folder choice, which is deliberately client-specific
 * (desktop picks a folder via `tauri-plugin-dialog`; mobile keeps notes in
 * its local folder, synced by iroh — see `src-tauri/src/workspace_picker.rs`).
 *
 * Per `@outl/shared/CLAUDE.md`, workspace picking is exactly the kind of
 * client-coupled command that stays in the client's own `lib/api.ts`.
 */

import { invoke } from "@tauri-apps/api/core";

/**
 * Persist `path` as the workspace folder and ask the app to reopen against
 * it. The reopen is boot-read (the backend emits
 * `workspace-reopen-required`; the next launch picks up the new path), so
 * callers should treat a successful `setWorkspace` as "the choice is saved"
 * rather than "the workspace is live this instant".
 *
 * No caller wires this today — the arbitrary-folder native picker is
 * deferred (`workspace_picker.rs`). It's the entry point that picker will
 * use once it can hand back a security-scoped path.
 */
export function setWorkspace(path: string): Promise<void> {
  return invoke<void>("set_workspace", { path });
}

// The whole plugin surface lives in `@outl/shared`: DTOs
// (`PluginCommand`, `PluginToolbarButton`, `PluginRunReply`,
// `PluginSyncHooksReply`, `PluginTransformer`, `PluginTransformResult`)
// in `@outl/shared/api/types`, wrappers (`pluginList`, `pluginRun`,
// `pluginSyncHooks`, `pluginToolbar`, `pluginTransformers`,
// `pluginTransform`) plus the marketplace in `@outl/shared/api/commands`
// — both clients register identical commands.

// ── Properties (issue #13) ──────────────────────────────────────────
// PROMOTE-TO-SHARED: `knownPropertyKeys` / `setPageProperty` wrap
// commands **both** GUI clients register (`known_property_keys`,
// `set_page_property`), so by the repo's own rule they belong next to
// `setBlockProperty` in `@outl/shared/api/commands`. They sit here only
// because the desktop half of #13 was landing in that file at the same
// time; move them up on the first commit that owns
// both sides.

// `PropertyKey` is a wire type both GUI clients share — it lives in
// `@outl/shared/api/types`, re-exported here so existing imports from
// this module keep resolving.
export type { PropertyKey } from "@outl/shared/api/types";

/**
 * Property keys already used somewhere in the workspace, most-used
 * first. The mobile Properties sheet paints the top ones as tappable
 * chips: keys repeat (a graph has a dozen, not a hundred) and typing
 * `oura-date` on a phone keyboard is the cost this removes.
 *
 * Cheap enough to call when the sheet opens — a scan of the property
 * map, no tree walk — so it is never cached into staleness.
 */
export function knownPropertyKeys(): Promise<PropertyKey[]> {
  return invoke<PropertyKey[]>("known_property_keys");
}

/**
 * Set — or clear, with an empty `value` — a `key:: value` property on
 * the **page** itself (`icon::`, `type::`, …). Returns the refreshed
 * page view.
 *
 * The backend refuses the structural keys (`page-slug`, `page-kind`):
 * they are the page's identity, and renaming is `page_rename`, which
 * moves the projection too.
 */
export function setPageProperty(
  pageId: string,
  key: string,
  value: string,
): Promise<import("@outl/shared/api/types").PageView> {
  return invoke("set_page_property", { pageId, key, value });
}
