import type { PluginCommand, TemplateDto } from "@outl/shared/api/types";

import type { Chord } from "./api";
import type { PaletteActionRow } from "./palette-results";

/**
 * Reserved `plugin_id` for the native `/template <name>` slash entries.
 * Structural templates ship in the core (reachable via TUI/CLI/MCP), so
 * they must appear in the desktop slash menu WITHOUT a plugin. We reuse
 * the plugin-command popup by injecting synthetic {@link PluginCommand}
 * rows under this sentinel; `OutlineView`'s `onRunPluginCommand`
 * intercepts it and calls `instantiateTemplateAt` instead of `pluginRun`.
 * A real plugin can never claim this id (it isn't a loadable plugin
 * directory), so there's no collision.
 */
export const NATIVE_TEMPLATE_PLUGIN_ID = "@outl/template";

/**
 * Reserved `plugin_id` for the native `/upload` slash entry. File upload
 * is a core feature (also reachable via the 📎 button, TUI `/upload`, and
 * drag-drop), so it appears in the slash menu WITHOUT a plugin. Same
 * sentinel trick as {@link NATIVE_TEMPLATE_PLUGIN_ID}: `OutlineView`'s
 * `onRunPluginCommand` intercepts this id and opens the file picker
 * instead of calling `pluginRun`.
 */
export const NATIVE_ASSET_PLUGIN_ID = "@outl/asset";

/**
 * The single native `/upload` row for the slash menu — opens the OS file
 * picker and attaches the chosen file's link. Kept as a one-element array
 * so it merges into the command list the same way
 * {@link templateSlashCommands} does.
 */
export function assetSlashCommands(): PluginCommand[] {
  return [
    {
      plugin_id: NATIVE_ASSET_PLUGIN_ID,
      command_id: "upload",
      title: "upload: attach a file",
    },
  ];
}

/**
 * Project the workspace's structural templates onto synthetic
 * {@link PluginCommand} rows for the slash menu. `command_id` carries the
 * template's invocation name (what `instantiateTemplateAt` needs);
 * `title` is a friendly label. Empty when the workspace has no template.
 */
export function templateSlashCommands(
  templates: TemplateDto[],
): PluginCommand[] {
  return templates.map((t) => ({
    plugin_id: NATIVE_TEMPLATE_PLUGIN_ID,
    command_id: t.name,
    title: `template: ${t.name}${t.duplicate ? " (duplicate name)" : ""}`,
  }));
}

/**
 * Reserved `plugin_id` for the client's **own** commands in the slash
 * menu — the actions `lib/action-handlers.ts` already dispatches from a
 * chord and the `Cmd+K` palette.
 *
 * Without them the menu's whole universe was `/upload` plus whatever
 * templates and plugins a workspace happened to have, so on one with
 * neither it opened listing a single row while the TUI's `/` listed 31
 * built-ins. Same sentinel trick as {@link NATIVE_TEMPLATE_PLUGIN_ID},
 * and `block-suggest.ts` runs this one through
 * `lib/shortcuts.ts::runNativeSlashCommand` — the same handler map the
 * keyboard and the palette fire, never a second dispatcher.
 */
export const NATIVE_ACTION_PLUGIN_ID = "@outl/action";

/**
 * `OpenToday` → `open-today`: the name a user types after `/`.
 *
 * Kebab because that is the shape every other slash command has (the
 * TUI's `date-today`, `prop-block`, `plugin-settings`), and because
 * `rankSlashCommands` scores an id **substring** — so `/today` finds
 * `open-today` without anyone memorising which verb a kind starts with.
 */
export function actionSlug(kind: string): string {
  return kind
    .replace(/([a-z0-9])([A-Z])/g, "$1-$2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1-$2")
    .toLowerCase();
}

/**
 * Project the client's own actions onto slash-menu rows.
 *
 * The input is {@link PaletteActionRow}, so the universe and the
 * wording are decided exactly once — `paletteActions` already answers
 * "which actions does this client have a handler for", and the label is
 * the catalog's own description. A second filter here would be a second
 * opinion about what the desktop can do (invariant 12).
 */
export function actionSlashCommands(
  rows: PaletteActionRow<Chord>[],
): PluginCommand[] {
  return rows.map((r) => ({
    plugin_id: NATIVE_ACTION_PLUGIN_ID,
    command_id: actionSlug(r.action),
    title: r.label,
  }));
}

/**
 * Rank plugin commands for the inline `/` slash menu against a query.
 *
 * Matching is keyed on the command **id** first (what the user types —
 * `/stats` — mirroring the TUI / CLI), then the human title. Ordering:
 *
 * 1. id starts with the query   (`/sta` → `stats`)
 * 2. id contains the query
 * 3. title contains the query
 *
 * An empty query returns every command (the bare `/` "show all" case).
 * Commands matching nothing are dropped. Ties keep input order (stable
 * sort), so two same-score commands stay in `pluginList()` order.
 */
export function rankSlashCommands(
  all: PluginCommand[],
  query: string,
): PluginCommand[] {
  const q = query.toLowerCase();
  return all
    .map((c, i) => {
      const id = c.command_id.toLowerCase();
      const title = c.title.toLowerCase();
      let score = -1;
      if (q === "") score = 0;
      else if (id.startsWith(q)) score = 3;
      else if (id.includes(q)) score = 2;
      else if (title.includes(q)) score = 1;
      return { c, score, i };
    })
    .filter((r) => r.score >= 0)
    .sort((a, b) => b.score - a.score || a.i - b.i)
    .map((r) => r.c);
}
