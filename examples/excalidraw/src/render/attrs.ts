/**
 * The attributes every element shares: stroke, dash pattern, rotation,
 * opacity, corner radius.
 *
 * Split out of the shape renderers so a stroke looks the same whether it is
 * around a rectangle or along an arrow, and so "how does Excalidraw compute a
 * corner radius" has exactly one answer in this plugin.
 */

import type { ExcalidrawElement } from "../types";
import { asString, escape } from "../escape";
import { r2 } from "./rough";

/** Excalidraw's default stroke when an element names none. */
const DEFAULT_STROKE = "#1e1e1e";

/** Stroke colour, falling back to Excalidraw's default. */
export function stroke(el: ExcalidrawElement): string {
  const c = asString(el.strokeColor);
  return c === "" || c === "transparent" ? DEFAULT_STROKE : c;
}

/** Stroke width in scene units. */
export function strokeWidth(el: ExcalidrawElement): number {
  const w = typeof el.strokeWidth === "number" ? el.strokeWidth : 1;
  return w > 0 ? w : 1;
}

/**
 * `stroke-dasharray` for Excalidraw's three stroke styles, scaled by width so
 * a thick dashed line keeps dashes that read as dashes.
 */
export function dash(el: ExcalidrawElement): string {
  const w = strokeWidth(el);
  if (el.strokeStyle === "dashed") return ` stroke-dasharray="${r2(w * 4)} ${r2(w * 4)}"`;
  if (el.strokeStyle === "dotted") return ` stroke-dasharray="${r2(w)} ${r2(w * 3)}"`;
  return "";
}

/** The shared stroke attributes for a path that is only stroked. */
export function strokeAttrs(el: ExcalidrawElement): string {
  return (
    ` fill="none" stroke="${escape(stroke(el))}" stroke-width="${r2(strokeWidth(el))}"` +
    ` stroke-linecap="round" stroke-linejoin="round"${dash(el)}`
  );
}

/**
 * `opacity`, remembering that Excalidraw's scale is 0–100.
 *
 * Returns `""` at full opacity so the common case adds no attribute.
 */
export function opacity(el: ExcalidrawElement): string {
  const o = typeof el.opacity === "number" ? el.opacity : 100;
  if (o >= 100) return "";
  return ` opacity="${r2(Math.max(0, o) / 100)}"`;
}

/**
 * `transform` for a rotated element.
 *
 * `angle` is radians clockwise about the element's centre; SVG wants degrees
 * and an explicit centre.
 */
export function rotation(el: ExcalidrawElement): string {
  const a = typeof el.angle === "number" ? el.angle : 0;
  if (a === 0) return "";
  const cx = (el.x || 0) + (el.width || 0) / 2;
  const cy = (el.y || 0) + (el.height || 0) / 2;
  return ` transform="rotate(${r2((a * 180) / Math.PI)} ${r2(cx)} ${r2(cy)})"`;
}

/**
 * Corner radius, following Excalidraw's `getCornerRadius`.
 *
 * Three roundness types, two formulas: proportional (and the legacy type)
 * take a quarter of the shorter side; adaptive holds a fixed 32 once the shape
 * is big enough for it, and falls back to proportional below that — which is
 * why a small rounded box does not look like a stadium.
 */
export function cornerRadius(el: ExcalidrawElement): number {
  const roundness = el.roundness;
  if (!roundness) return 0;

  const shorter = Math.min(Math.abs(el.width || 0), Math.abs(el.height || 0));
  const proportional = 0.25;

  if (roundness.type === 3 || roundness.type === 1) return shorter * proportional;

  if (roundness.type === 2) {
    const fixed = typeof roundness.value === "number" ? roundness.value : 32;
    const cutoff = fixed / proportional;
    return shorter <= cutoff ? shorter * proportional : fixed;
  }
  return 0;
}

/** The four corners of an element's box, clockwise from the top left. */
export function box(el: ExcalidrawElement): [number, number][] {
  const x = el.x || 0;
  const y = el.y || 0;
  const w = el.width || 0;
  const h = el.height || 0;
  return [
    [x, y],
    [x + w, y],
    [x + w, y + h],
    [x, y + h],
  ];
}

/** The four points of a diamond inscribed in the element's box. */
export function diamondPoints(el: ExcalidrawElement): [number, number][] {
  const x = el.x || 0;
  const y = el.y || 0;
  const w = el.width || 0;
  const h = el.height || 0;
  return [
    [x + w / 2, y],
    [x + w, y + h / 2],
    [x + w / 2, y + h],
    [x, y + h / 2],
  ];
}

/** Absolute points of a linear element (`points` are relative to `x`/`y`). */
export function absolutePoints(el: ExcalidrawElement): [number, number][] {
  // Bounded for the reason `renderText` bounds its lines: the element cap is
  // on the count of elements, and this walk runs twice per element (once for
  // the bounding box, once to draw). A freehand stroke with a million points
  // is one element.
  const MAX_POINTS = 5000;

  const x = el.x || 0;
  const y = el.y || 0;
  const pts: [number, number][] = [];
  const raw = Array.isArray(el.points) ? el.points : [];
  const count = Math.min(raw.length, MAX_POINTS);
  for (let i = 0; i < count; i++) {
    const p = raw[i];
    if (!Array.isArray(p) || p.length < 2) continue;
    const px = Number(p[0]);
    const py = Number(p[1]);
    if (!Number.isFinite(px) || !Number.isFinite(py)) continue;
    pts.push([x + px, y + py]);
  }
  return pts;
}
