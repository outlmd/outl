import type { PluginCommand, TemplateDto } from "@outl/shared/api/types";
import type { InsertCommandDto } from "@outl/shared/api/commands";

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
 *
 * The **name** on each row comes from `outl_shortcuts::command_name`,
 * never from a derivation here: see {@link actionSlashCommands}.
 */
export const NATIVE_ACTION_PLUGIN_ID = "@outl/action";

/**
 * Reserved `plugin_id` for a command that writes **text at the caret**
 * rather than acting — `/date-today`, `/time-now`, `/week-num`.
 *
 * They are not `Action`s and never will be: they take no chord, mutate
 * nothing and answer one question, *what string goes where the cursor
 * is?* `outl_actions::insert` is the catalog, shared with the TUI, so
 * `/date-today` produces the same `[[YYYY-MM-DD]]` in a terminal and
 * in a webview — and this client holds no date format of its own.
 *
 * Same sentinel trick as {@link NATIVE_TEMPLATE_PLUGIN_ID}, and
 * `block-suggest.ts` runs this one through `resolveInsertCommand`,
 * splicing the reply over the `/token` instead of deleting it.
 */
export const NATIVE_INSERT_PLUGIN_ID = "@outl/insert";

/**
 * Project the text-inserting commands onto slash-menu rows.
 *
 * A command needing an argument is **left out**: the `/` trigger
 * closes on the first space (`detectSlashContext`), so `/date +3d` is
 * unreachable here and a row for it could only ever fail. The TUI
 * collects the argument through its `:` palette and keeps the command.
 */
export function insertSlashCommands(
  commands: InsertCommandDto[],
): PluginCommand[] {
  return commands
    .filter((c) => !c.needs_args)
    .map((c) => ({
      plugin_id: NATIVE_INSERT_PLUGIN_ID,
      command_id: c.name,
      title: c.description,
    }));
}

/**
 * Project the client's own actions onto slash-menu rows.
 *
 * Two facts, neither decided here:
 *
 * - **Which actions** is `paletteActions`' answer — it already knows
 *   which ones this client has a handler for, and the label is the
 *   catalog's own description. A second filter here would be a second
 *   opinion about what the desktop can do (invariant 12).
 * - **What the user types** is `name`, resolved by the caller from
 *   `outl_shortcuts::command_name`.
 *
 * This used to kebab-case the variant locally (`OpenToday` →
 * `open-today`) while the TUI carried its own literal (`today`). Of 31
 * TUI built-ins and 66 action rows, exactly two names coincided, so a
 * user who learned `/today` in the terminal typed it here and got a
 * row with another name on it.
 */
export function actionSlashCommands(
  rows: (PaletteActionRow<Chord> & { name: string })[],
): PluginCommand[] {
  return rows.map((r) => ({
    plugin_id: NATIVE_ACTION_PLUGIN_ID,
    command_id: r.name,
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
