import { describe, expect, it } from "vitest";

import {
  NATIVE_ACTION_PLUGIN_ID,
  NATIVE_BLOCK_PLUGIN_ID,
  actionSlashCommands,
  blockSlashCommands,
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
  // `name` is what `outl_shortcuts::command_name` answered — the
  // same string the TUI's own built-ins read their names from. The
  // caller resolves it; nothing here derives one.
  const row = (
    action: string,
    name: string,
    label: string,
  ): PaletteActionRow<Chord> & { name: string } => ({
    kind: "action",
    action: action as PaletteActionRow<Chord>["action"],
    label,
    chord: null,
    name,
  });

  it("lists one row per action, named by the catalog", () => {
    const out = actionSlashCommands([
      row("OpenToday", "today", "Open today's journal"),
      row("ToggleSidebar", "toggle-sidebar", "Toggle the sidebar"),
    ]);
    expect(out).toEqual([
      {
        plugin_id: NATIVE_ACTION_PLUGIN_ID,
        command_id: "today",
        title: "Open today's journal",
      },
      {
        plugin_id: NATIVE_ACTION_PLUGIN_ID,
        command_id: "toggle-sidebar",
        title: "Toggle the sidebar",
      },
    ]);
  });

  it("calls it what the TUI calls it", () => {
    // The reason `command_name` exists. This client used to derive
    // the name from the variant and list `/open-today`, so a user who
    // learned `/today` in the terminal found a row with another name
    // on it. Nothing here may re-derive it.
    const out = rankSlashCommands(
      actionSlashCommands([row("OpenToday", "today", "Open today's journal")]),
      "today",
    );
    expect(out.map((c) => c.command_id)).toEqual(["today"]);
  });
});

describe("blockSlashCommands", () => {
  it("offers /quote under the block sentinel", () => {
    // The desktop's only route to `toggle_quote`. Before this row the
    // command was registered on both GUI clients, documented in two
    // places as in use, and called by nothing — so the row *existing*
    // is the assertion, not its label.
    expect(blockSlashCommands()).toEqual([
      {
        plugin_id: NATIVE_BLOCK_PLUGIN_ID,
        command_id: "quote",
        title: "quote: make this block a blockquote (or undo it)",
      },
    ]);
  });

  it("is found by the name a user types", () => {
    const out = rankSlashCommands([...ALL, ...blockSlashCommands()], "quo");
    expect(out[0].command_id).toBe("quote");
  });

  it("does not shadow a plugin's own id", () => {
    // The sentinel is not a loadable plugin directory, so a real
    // plugin can never claim it — but two rows answering `/quote`
    // would still be two things one key means.
    const ids = [...ALL, ...blockSlashCommands()].map((c) => c.command_id);
    expect(new Set(ids).size).toBe(ids.length);
  });
});
