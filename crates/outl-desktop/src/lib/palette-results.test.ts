import { describe, expect, it } from "vitest";

import type { Binding } from "./api";
import { CHORDLESS_ACTIONS, paletteActions, paletteResults } from "./palette-results";

const cmdK = [{ mods: 8, key: { kind: "Char", value: "k" } }] as Binding["chord"];
const ctrlK = [{ mods: 1, key: { kind: "Char", value: "k" } }] as Binding["chord"];

const bindings: Binding[] = [
  { chord: cmdK, mode: "global", action: { kind: "OpenToday" }, description: "Open today's journal" },
  { chord: ctrlK, mode: "global", action: { kind: "OpenToday" }, description: "Ctrl twin" },
  { chord: cmdK, mode: "normal", action: { kind: "ToggleSidebar" }, description: "Toggle sidebar" },
  { chord: cmdK, mode: "insert", action: { kind: "WrapBold" }, description: "Bold" },
  { chord: cmdK, mode: "global", action: { kind: "OpenPicker" }, description: "Open quick switcher" },
  { chord: cmdK, mode: "global", action: { kind: "Quit" }, description: "Quit" },
] as Binding[];

const actions = paletteActions(bindings, (k) => k !== "Quit" && k !== "ExportMarkdown", true);

describe("paletteActions", () => {
  it("keeps one row per handled global or normal action, first description wins", () => {
    expect(actions.map((a) => [a.action, a.label])).toEqual([
      ["OpenToday", "Open today's journal"],
      ["ToggleSidebar", "Toggle sidebar"],
    ]);
  });

  it("shows the Cmd chord on macOS and the Ctrl twin elsewhere", () => {
    expect(actions[0].chord).toEqual(cmdK);
    const other = paletteActions(bindings, (k) => k !== "Quit" && k !== "ExportMarkdown", false);
    expect(other[0].chord).toEqual(ctrlK);
    expect(other[0].label).toBe("Open today's journal");
  });
});

describe("chordless actions", () => {
  it("lists Export as Markdown by name once it has a handler", () => {
    const rows = paletteActions([], (k) => k === "ExportMarkdown", true);
    expect(rows).toEqual([{ kind: "action", action: "ExportMarkdown", label: "Export as Markdown", chord: null }]);
    const r = paletteResults({ query: "> export", pages: [], blocks: [], actions: rows });
    expect(r.items).toHaveLength(1);
  });

  it("stays out when the client has no handler", () => {
    expect(paletteActions([], () => false, true)).toEqual([]);
    expect(CHORDLESS_ACTIONS.every((c) => c.label.length > 0)).toBe(true);
  });
});
