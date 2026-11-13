import { describe, expect, it } from "vitest";

import {
  NATIVE_ACTION_PLUGIN_ID,
  actionSlashCommands,
  actionSlug,
  rankSlashCommands,
} from "./slash-commands";
import type { PluginCommand } from "@outl/shared/api/types";
import type { Chord } from "./api";
import type { PaletteActionRow } from "./palette-results";

const cmd = (command_id: string, title: string): PluginCommand => ({
  plugin_id: `app.outl.examples.${command_id}`,
  command_id,
  title,
});

const ALL: PluginCommand[] = [
  cmd("stats", "Workspace statistics"),
  cmd("greet", "Greet me"),
  cmd("pick", "Pick a random task"),
  cmd("todo-archive-done", "Archive DONE blocks"),
];

describe("rankSlashCommands", () => {
  it("ranks the id-prefix match on top — the `/sta` report", () => {
    const out = rankSlashCommands(ALL, "sta");
    expect(out[0].command_id).toBe("stats");
  });

  it("matches by full id and keeps it first", () => {
    const out = rankSlashCommands(ALL, "stats");
    expect(out[0].command_id).toBe("stats");
  });

  it("returns every command for an empty query (bare `/`)", () => {
    expect(rankSlashCommands(ALL, "")).toHaveLength(ALL.length);
  });

  it("falls back to a title match when the id does not match", () => {
    // No id contains "archive"; only the title "Archive DONE blocks".
    const out = rankSlashCommands(ALL, "archive");
    expect(out.map((c) => c.command_id)).toContain("todo-archive-done");
  });

  it("drops commands that match neither id nor title", () => {
    expect(rankSlashCommands(ALL, "zzzz")).toEqual([]);
  });

  it("ranks id-substring above title-substring", () => {
    // Query "e": id "greet" (substring) must outrank a title-only hit.
    const out = rankSlashCommands(ALL, "gree");
    expect(out[0].command_id).toBe("greet");
  });
});

/**
 * The slash menu's universe used to be `upload` + templates + plugins,
 * so on a workspace with neither it listed exactly one row while the
 * TUI's `/` listed 31. These pin the native half: the actions this
 * client already dispatches from the keyboard and the palette.
 */
describe("actionSlashCommands", () => {
  const row = (
    action: string,
    label: string,
  ): PaletteActionRow<Chord> => ({
    kind: "action",
    action: action as PaletteActionRow<Chord>["action"],
    label,
    chord: null,
  });

  it("turns an action kind into the name the user types", () => {
    expect(actionSlug("OpenToday")).toBe("open-today");
    expect(actionSlug("ToggleSidebar")).toBe("toggle-sidebar");
    expect(actionSlug("Quit")).toBe("quit");
  });

  it("lists one row per action, under the native sentinel", () => {
    const out = actionSlashCommands([
      row("OpenToday", "Open today's journal"),
      row("ToggleSidebar", "Toggle the sidebar"),
    ]);
    expect(out).toEqual([
      {
        plugin_id: NATIVE_ACTION_PLUGIN_ID,
        command_id: "open-today",
        title: "Open today's journal",
      },
      {
        plugin_id: NATIVE_ACTION_PLUGIN_ID,
        command_id: "toggle-sidebar",
        title: "Toggle the sidebar",
      },
    ]);
  });

  it("is reachable by the word a user actually types", () => {
    // `/today` has to find `OpenToday`, the way `/today` does in the
    // TUI. `rankSlashCommands` scores an id *substring*, which is what
    // makes the kebab slug typeable without memorising the prefix.
    const out = rankSlashCommands(
      actionSlashCommands([row("OpenToday", "Open today's journal")]),
      "today",
    );
    expect(out.map((c) => c.command_id)).toEqual(["open-today"]);
  });
});
