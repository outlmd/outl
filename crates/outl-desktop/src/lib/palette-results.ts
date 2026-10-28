/**
 * What the Cmd+K / Cmd+P palette lists for a query. The ranking (pages,
 * tasks, blocks, actions, and the `>` command prefix) is owned by
 * `@outl/shared/palette`; this file builds the action rows from the
 * shortcut catalog, each with its chord, so the palette doubles as the
 * place you learn the keys. Actions with no chord at all
 * ({@link CHORDLESS_ACTIONS}) are listed by name only.
 */
import {
  BLOCK_QUERY_MIN,
  COMMAND_PREFIX,
  paletteResults,
  sectionOf,
  type PaletteActionRow,
  type PaletteItem,
  type PaletteLimits,
  type PaletteSection,
} from "@outl/shared/palette";

import { MOD_META, type Action, type Binding, type Chord } from "./api";

// The ranking lives in `@outl/shared/palette` so the phone's search sheet
// lists the same things in the same order. What stays here is the part
// that needs chords, which the phone has none of.
export { BLOCK_QUERY_MIN, COMMAND_PREFIX, paletteResults, sectionOf };
export type { PaletteActionRow, PaletteItem, PaletteLimits, PaletteSection };

/**
 * Actions the palette lists although no chord reaches them: things you
 * do rarely enough that a key would be a waste, but should still find
 * by typing their name. Each still owes a row in `outl_shortcuts::support`.
 */
export const CHORDLESS_ACTIONS: ReadonlyArray<{ action: Action["kind"]; label: string }> = [
  { action: "ExportMarkdown", label: "Export as Markdown" },
];

/**
 * Actions that make no sense as a palette row: the palette itself, and
 * anything that only means something mid-chord or mid-edit (cursor
 * motion, text objects). Those have no `global` or `normal` binding,
 * which is the filter `paletteActions` applies; this list is only for
 * the two that do.
 */
const NOT_LISTED = new Set<Action["kind"]>(["OpenPicker", "OpenCommandPalette"]);

/**
 * One row per action that has a handler and a `global` or `normal`
 * binding, labelled with the first binding's description. The catalog
 * lists a Cmd chord before its Ctrl twin, so off macOS a later binding
 * without Cmd replaces the chord: the row teaches a key that works here.
 */
export function paletteActions(
  bindings: Binding[],
  handled: (kind: Action["kind"]) => boolean,
  mac: boolean = isMac(),
): PaletteActionRow<Chord>[] {
  const seen = new Map<Action["kind"], PaletteActionRow<Chord>>();
  const usesMeta = (c: Chord[]) => c.some((k) => (k.mods & MOD_META) !== 0);
  for (const b of bindings) {
    const kind = b.action.kind;
    if (b.mode !== "global" && b.mode !== "normal") continue;
    if (NOT_LISTED.has(kind) || !handled(kind)) continue;
    const prev = seen.get(kind);
    if (!prev) {
      seen.set(kind, { kind: "action", action: kind, label: b.description, chord: b.chord });
    } else if (!mac && prev.chord && usesMeta(prev.chord) && !usesMeta(b.chord)) {
      prev.chord = b.chord;
    }
  }
  for (const c of CHORDLESS_ACTIONS) {
    if (!seen.has(c.action) && handled(c.action)) {
      seen.set(c.action, { kind: "action", action: c.action, label: c.label, chord: null });
    }
  }
  return [...seen.values()];
}

/** Whether the webview runs on macOS, where Cmd is the platform modifier. */
export function isMac(): boolean {
  return typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
}
