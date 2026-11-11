/**
 * Turning a scene into one SVG document.
 *
 * The element renderers each draw one thing; this decides what the drawing as
 * a whole looks like — which elements are drawn and in what order, how big the
 * canvas is, and which `<defs>` the shapes end up referencing.
 */

import type { ExcalidrawElement } from "../types";
import { absolutePoints } from "./attrs";
import { Fills } from "./fill";
import { asString, escape } from "../escape";
import { renderFreedraw, renderLinear } from "./linear";
import { r2 } from "./rough";
import { renderDiamond, renderEllipse, renderFrame, renderImage, renderRectangle } from "./shapes";
import { renderText } from "./text";

/** Blank space kept around the drawing, in scene units. */
const PADDING = 16;

/**
 * Most elements this renderer will draw in one pass.
 *
 * The plugin runs in an interpreter with a loop-iteration cap, and a scene can
 * be arbitrarily large. Hitting the cap surfaces as a JS error — the whole
 * fence renders as nothing. Stopping early and *saying so* in the caption
 * keeps a 10,000-element scene useful instead of broken.
 */
const MAX_ELEMENTS = 1200;

/** What `renderScene` produced, and what it left out. */
export interface Rendered {
  svg: string;
  /** Elements actually drawn. */
  drawn: number;
  /** Elements past [`MAX_ELEMENTS`]. */
  truncated: number;
  /** Elements of a kind this renderer has no drawing for. */
  unsupported: number;
  /** Intrinsic size of the drawing, in scene units. */
  width: number;
  height: number;
}

/** Element kinds that carry no geometry to draw. */
const SKIP = ["selection", "embeddable", "iframe", "magicframe"];

/**
 * Render every visible element.
 *
 * `background` is drawn only when asked for: a fence inside a note usually
 * looks better transparent, sitting on the page's own background, while a
 * scene whose drawing relies on its background colour needs it.
 */
export function renderScene(
  elements: ExcalidrawElement[],
  files: Record<string, string>,
  background: string,
  showBackground: boolean,
): Rendered {
  const live: ExcalidrawElement[] = [];
  let unsupported = 0;

  for (let i = 0; i < elements.length; i++) {
    const el = elements[i];
    if (!el || el.isDeleted) continue;
    if (SKIP.indexOf(el.type || "") !== -1 || !isDrawable(el)) {
      unsupported++;
      continue;
    }
    live.push(el);
  }

  const truncated = Math.max(0, live.length - MAX_ELEMENTS);
  const drawing = truncated > 0 ? live.slice(0, MAX_ELEMENTS) : live;

  const bounds = boundsOf(drawing);
  const fills = new Fills();

  // Frames first, so a frame outline never covers what it contains. Within
  // each pass the array order is the z-order, as in Excalidraw.
  let body = "";
  for (let i = 0; i < drawing.length; i++) {
    if (drawing[i].type === "frame") body += renderFrame(drawing[i]);
  }
  for (let i = 0; i < drawing.length; i++) {
    const el = drawing[i];
    if (el.type === "frame") continue;
    const drawn = renderElement(el, fills, files);
    if (drawn === "") unsupported++;
    body += drawn;
  }

  const width = Math.max(1, bounds.maxX - bounds.minX);
  const height = Math.max(1, bounds.maxY - bounds.minY);

  const backdrop =
    showBackground && isColor(background)
      ? `<rect x="${r2(bounds.minX)}" y="${r2(bounds.minY)}" width="${r2(width)}"` +
        ` height="${r2(height)}" fill="${escape(background)}"/>`
      : "";

  const defs = fills.toDefs();
  const svg =
    `<svg xmlns="http://www.w3.org/2000/svg" viewBox="${r2(bounds.minX)} ${r2(bounds.minY)}` +
    ` ${r2(width)} ${r2(height)}" width="100%" height="100%"` +
    ` preserveAspectRatio="xMidYMid meet" role="img">` +
    (defs === "" ? "" : `<defs>${defs}</defs>`) +
    backdrop +
    body +
    `</svg>`;

  return { svg, drawn: drawing.length, truncated, unsupported, width, height };
}

/**
 * Can this element's geometry produce real numbers?
 *
 * Bounding the viewBox is not enough: an element at 1e308 keeps every
 * individual field finite while `x + width` overflows, and the renderer then
 * emits `M Infinity …` into the path. The SVG stays well-formed and draws
 * nothing, with no caption to say why — so the sums are what get checked, and
 * a failure counts as unsupported like any other kind it cannot draw.
 */
function isDrawable(el: ExcalidrawElement): boolean {
  const x = el.x || 0;
  const y = el.y || 0;
  const w = el.width || 0;
  const h = el.height || 0;
  const angle = el.angle || 0;
  return (
    Number.isFinite(x + w) && Number.isFinite(y + h) && Number.isFinite(angle)
  );
}

/** Dispatch one element to its renderer. `""` means "nothing drawn". */
function renderElement(
  el: ExcalidrawElement,
  fills: Fills,
  files: Record<string, string>,
): string {
  switch (el.type) {
    case "rectangle":
      return renderRectangle(el, fills);
    case "diamond":
      return renderDiamond(el, fills);
    case "ellipse":
      return renderEllipse(el, fills);
    case "line":
    case "arrow":
      return renderLinear(el, fills);
    case "freedraw":
      return renderFreedraw(el);
    case "text":
      return renderText(el);
    case "image":
      return renderImage(el, files);
    default:
      return "";
  }
}

/** The box every element fits inside, padded. */
function boundsOf(elements: ExcalidrawElement[]): {
  minX: number;
  minY: number;
  maxX: number;
  maxY: number;
} {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;

  for (let i = 0; i < elements.length; i++) {
    const corners = extentOf(elements[i]);
    for (let c = 0; c < corners.length; c++) {
      const [x, y] = corners[c];
      if (x < minX) minX = x;
      if (y < minY) minY = y;
      if (x > maxX) maxX = x;
      if (y > maxY) maxY = y;
    }
  }

  // All four, not just `minX`. That one only answers "were there any
  // elements"; a coordinate like 1e308 leaves it finite while `maxX` overflows
  // to Infinity, and the viewBox then reads `Infinity` and draws nothing,
  // silently.
  if (
    !Number.isFinite(minX) ||
    !Number.isFinite(minY) ||
    !Number.isFinite(maxX) ||
    !Number.isFinite(maxY)
  ) {
    return { minX: 0, minY: 0, maxX: 1, maxY: 1 };
  }
  return {
    minX: minX - PADDING,
    minY: minY - PADDING,
    maxX: maxX + PADDING,
    maxY: maxY + PADDING,
  };
}

/**
 * The points that bound one element.
 *
 * A rotated element's corners are rotated too, so the box is computed from the
 * rotated corners — otherwise a diagonal arrow in a rotated group gets clipped
 * at the edge of the drawing.
 */
function extentOf(el: ExcalidrawElement): [number, number][] {
  const x = el.x || 0;
  const y = el.y || 0;
  const w = el.width || 0;
  const h = el.height || 0;

  const pts: [number, number][] =
    Array.isArray(el.points) && el.points.length > 0
      ? absolutePoints(el)
      : [
          [x, y],
          [x + w, y],
          [x + w, y + h],
          [x, y + h],
        ];

  const angle = typeof el.angle === "number" ? el.angle : 0;
  if (angle === 0) return pts;

  const cx = x + w / 2;
  const cy = y + h / 2;
  const cos = Math.cos(angle);
  const sin = Math.sin(angle);

  const out: [number, number][] = [];
  for (let i = 0; i < pts.length; i++) {
    const dx = pts[i][0] - cx;
    const dy = pts[i][1] - cy;
    out.push([cx + dx * cos - dy * sin, cy + dx * sin + dy * cos]);
  }
  return out;
}

/** Is this a colour worth painting, rather than Excalidraw's "no background"? */
function isColor(value: string): boolean {
  const v = asString(value);
  return v !== "" && v !== "transparent" && v !== "none";
}
