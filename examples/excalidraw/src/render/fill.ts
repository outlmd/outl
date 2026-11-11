/**
 * Fill styles, as SVG patterns.
 *
 * Excalidraw fills a shape by drawing hatch lines inside it; the SVG way to say
 * that is a `<pattern>`, which the renderer draws instead of a few hundred
 * `<line>` elements per shape. One pattern is emitted per (style, colour,
 * weight) actually used, so a scene with forty blue hachured boxes carries one
 * pattern definition, not forty.
 */

import { asString, escape } from "../escape";

/** What a shape should put in its `fill` attribute. */
export type Paint = string;

/** No fill at all — Excalidraw's default `backgroundColor`. */
const NONE: Paint = "none";

/** Tile size of a hatch pattern, in scene units. */
const GAP = 8;

/**
 * Collects the `<pattern>` definitions a scene needs, de-duplicated.
 *
 * Built as a small accumulator rather than a pure function because the `<defs>`
 * block has to be written once, before the shapes that reference it, while the
 * decision of *which* patterns exist is made while walking the elements.
 */
export class Fills {
  private seen: Record<string, true> = {};
  private defs: string[] = [];

  /**
   * The `fill` value for one element.
   *
   * Registers a pattern on first use and returns `url(#id)`; returns the colour
   * itself for a solid fill, and `none` when there is nothing to fill.
   */
  paint(color: string | undefined, style: string | undefined, strokeWidth: number): Paint {
    const c = asString(color);
    if (c === "" || c === "transparent" || c === "none") return NONE;

    const kind = style === "solid" ? "solid" : style === "cross-hatch" ? "cross" : "hachure";
    if (kind === "solid") return c;

    const weight = Math.max(0.6, Math.round(strokeWidth * 60) / 100);
    const id = `f-${kind}-${slug(c)}-${String(weight).replace(".", "_")}`;
    if (!this.seen[id]) {
      this.seen[id] = true;
      this.defs.push(pattern(id, kind, c, weight));
    }
    return `url(#${id})`;
  }

  /** Every pattern registered so far, ready to drop inside `<defs>`. */
  toDefs(): string {
    return this.defs.join("");
  }
}

/** One hatch pattern. `cross` adds the second direction. */
function pattern(id: string, kind: string, color: string, weight: number): string {
  const lines =
    `<line x1="0" y1="0" x2="0" y2="${GAP}" stroke="${escape(color)}" stroke-width="${weight}"/>` +
    (kind === "cross"
      ? `<line x1="0" y1="0" x2="${GAP}" y2="0" stroke="${escape(color)}" stroke-width="${weight}"/>`
      : "");

  return (
    `<pattern id="${id}" patternUnits="userSpaceOnUse" width="${GAP}" height="${GAP}"` +
    ` patternTransform="rotate(45)">${lines}</pattern>`
  );
}

/**
 * A colour reduced to something usable as an id fragment.
 *
 * Collisions would only merge two patterns that are already identical in
 * everything this id encodes, so a lossy slug is safe here.
 */
function slug(color: string): string {
  let out = "";
  for (let i = 0; i < color.length; i++) {
    const ch = color.charAt(i);
    out += /[A-Za-z0-9]/.test(ch) ? ch.toLowerCase() : "-";
  }
  return out;
}
