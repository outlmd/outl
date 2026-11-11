/**
 * Text elements.
 *
 * Excalidraw has already wrapped the text by the time it reaches the file, so
 * `\n` is a real line break and there is no measuring to do here — only
 * placing lines, which keeps this renderer free of font metrics it has no way
 * to obtain inside a sandboxed iframe.
 */

import type { ExcalidrawElement } from "../types";
import { opacity, rotation } from "./attrs";
import { asString, escape } from "../escape";
import { r2 } from "./rough";


/**
 * Font stacks per Excalidraw's `fontFamily` ids.
 *
 * The iframe has no network, so Excalifont, Nunito and Comic Shanns are not
 * available unless the host happens to have them installed. Each stack
 * therefore ends in a generic family that carries the *character* of the
 * original — cursive for the hand-drawn fonts, so a sketch still reads as a
 * sketch rather than as a form.
 */
const FONTS: Record<number, string> = {
  1: 'Excalifont, Virgil, "Segoe Print", "Bradley Hand", cursive',
  2: 'Nunito, ui-sans-serif, system-ui, -apple-system, sans-serif',
  3: '"Comic Shanns", "Cascadia Code", ui-monospace, SFMono-Regular, monospace',
  5: 'Excalifont, Virgil, "Segoe Print", "Bradley Hand", cursive',
  6: 'Nunito, ui-sans-serif, system-ui, -apple-system, sans-serif',
  7: '"Lilita One", ui-sans-serif, system-ui, sans-serif',
  8: '"Comic Shanns", ui-monospace, SFMono-Regular, monospace',
  9: '"Liberation Sans", ui-sans-serif, system-ui, sans-serif',
};

const DEFAULT_FONT = FONTS[1];

/**
 * The stack for a `fontFamily`, with no way to reach `Object.prototype`.
 *
 * `FONTS[el.fontFamily || 0]` looks safe and is not: the value comes from
 * JSON, so `"constructor"` resolves to a function — truthy, so the `||`
 * fallback never fires — and escaping it then calls `.replace` on it and
 * throws, costing the whole fence.
 */
function fontStack(id: unknown): string {
  if (typeof id !== "number" || !Object.prototype.hasOwnProperty.call(FONTS, id)) {
    return DEFAULT_FONT;
  }
  return FONTS[id];
}

/**
 * Where a line's baseline sits inside its line box, as a fraction of the font
 * size. Fonts differ; this is the ratio that keeps a single-line label centred
 * in its container across the stacks above.
 */
const BASELINE = 0.78;

/** Render one text element, one `<tspan>` per line. */
export function renderText(el: ExcalidrawElement): string {
  const raw = typeof el.text === "string" ? el.text : "";
  if (raw === "") return "";

  const fontSize = typeof el.fontSize === "number" && el.fontSize > 0 ? el.fontSize : 20;
  const lineHeight =
    typeof el.lineHeight === "number" && el.lineHeight > 0 ? el.lineHeight : 1.25;
  const lineBox = fontSize * lineHeight;

  const family = fontStack(el.fontFamily);
  const color = asString(el.strokeColor) || "#1e1e1e";

  // MAX_ELEMENTS caps how many elements are drawn, not how much work one of
  // them asks for: a single text element holding a million newlines emits a
  // million <tspan>s. Cap the lines the same way the scene caps elements.
  const MAX_LINES = 2000;
  const all = raw.split("\n");
  const lines = all.length > MAX_LINES ? all.slice(0, MAX_LINES) : all;
  const x = anchorX(el);
  const anchor = anchorName(el.textAlign);
  const top = (el.y || 0) + verticalOffset(el, lines.length, lineBox);

  let spans = "";
  for (let i = 0; i < lines.length; i++) {
    const baseline = top + i * lineBox + (lineBox - fontSize) / 2 + fontSize * BASELINE;
    // An empty line still has to advance, and an empty <tspan> is dropped by
    // some renderers — a zero-width space keeps the line in the flow.
    const content = lines[i] === "" ? "​" : escape(lines[i]);
    spans += `<tspan x="${r2(x)}" y="${r2(baseline)}">${content}</tspan>`;
  }

  return (
    `<g${rotation(el)}${opacity(el)}><text font-family="${escape(family)}"` +
    ` font-size="${r2(fontSize)}" fill="${escape(color)}" text-anchor="${anchor}"` +
    ` style="white-space:pre">${spans}</text></g>`
  );
}

/** The x every line is anchored at, which depends on the alignment. */
function anchorX(el: ExcalidrawElement): number {
  const x = el.x || 0;
  const w = el.width || 0;
  if (el.textAlign === "center") return x + w / 2;
  if (el.textAlign === "right") return x + w;
  return x;
}

/** `text-anchor` for an Excalidraw alignment. */
function anchorName(align: string | undefined): string {
  if (align === "center") return "middle";
  if (align === "right") return "end";
  return "start";
}

/**
 * How far down the element's box the first line starts.
 *
 * Text bound to a container carries `verticalAlign: middle`, which is what
 * centres a label inside a shape.
 */
function verticalOffset(el: ExcalidrawElement, lines: number, lineBox: number): number {
  const height = el.height || 0;
  const total = lines * lineBox;
  if (el.verticalAlign === "middle") return Math.max(0, (height - total) / 2);
  if (el.verticalAlign === "bottom") return Math.max(0, height - total);
  return 0;
}
