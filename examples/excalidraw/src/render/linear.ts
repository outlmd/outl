/**
 * The open shapes: lines, arrows and freehand strokes.
 *
 * All three are a list of points; what differs is how the points are joined
 * (straight, smoothed) and what is drawn at the ends.
 */

import type { ExcalidrawElement } from "../types";
import { absolutePoints, opacity, rotation, strokeAttrs, strokeWidth } from "./attrs";
import { Fills } from "./fill";
import { asString, escape } from "../escape";
import { doubled, jitter, polylineSketch, r2, seeded, smooth } from "./rough";

/**
 * A line or an arrow.
 *
 * `roundness` on a linear element means "curved", not "rounded corners", so a
 * curved line is drawn smoothed and clean while a straight one is jittered —
 * the same split Excalidraw makes.
 *
 * A closed line (`line` whose last point meets its first) can carry a
 * background, so the fill is honoured here too.
 */
export function renderLinear(el: ExcalidrawElement, fills: Fills): string {
  const pts = absolutePoints(el);
  if (pts.length < 2) return "";

  const rand = seeded(el.seed);
  const amount = jitter(el.roughness);
  const curved = !!el.roundness;

  let body = "";

  const closed = el.type === "line" && isClosed(pts);
  if (closed) {
    const fill = fills.paint(el.backgroundColor, el.fillStyle, strokeWidth(el));
    if (fill !== "none") {
      body += `<path d="${smooth(pts)} Z" fill="${escape(fill)}"/>`;
    }
  }

  const path = curved
    ? smooth(pts)
    : doubled(() => polylineSketch(rand, pts, amount), amount);
  body += `<path d="${path}"${strokeAttrs(el)}/>`;

  if (el.type === "arrow") {
    body += arrowhead(el, pts[1], pts[0], el.startArrowhead);
    body += arrowhead(el, pts[pts.length - 2], pts[pts.length - 1], el.endArrowhead);
  }

  return `<g${rotation(el)}${opacity(el)}>${body}</g>`;
}

/**
 * A freehand stroke.
 *
 * Excalidraw renders these through perfect-freehand, which builds a filled
 * outline whose width follows pen pressure. A stroked smooth path is the
 * honest approximation: the line is the right shape and the right weight, and
 * it does not taper. Drawn slightly heavier, which is what the filled outline
 * reads as at normal sizes.
 */
export function renderFreedraw(el: ExcalidrawElement): string {
  const pts = absolutePoints(el);
  if (pts.length === 0) return "";

  const width = r2(strokeWidth(el) * 1.6);

  // A single-point stroke is a dot, and a zero-length path draws nothing.
  if (pts.length === 1) {
    return (
      `<g${rotation(el)}${opacity(el)}><circle cx="${r2(pts[0][0])}" cy="${r2(pts[0][1])}"` +
      ` r="${r2(Math.max(0.8, strokeWidth(el)))}" fill="${escape(strokeOf(el))}"/></g>`
    );
  }

  return (
    `<g${rotation(el)}${opacity(el)}><path d="${smooth(pts)}" fill="none"` +
    ` stroke="${escape(strokeOf(el))}" stroke-width="${width}" stroke-linecap="round"` +
    ` stroke-linejoin="round"/></g>`
  );
}

/** Stroke colour without pulling in the whole attribute string. */
function strokeOf(el: ExcalidrawElement): string {
  const c = asString(el.strokeColor);
  return c === "" || c === "transparent" ? "#1e1e1e" : c;
}

/** Does this polyline end where it started? */
function isClosed(pts: [number, number][]): boolean {
  const first = pts[0];
  const last = pts[pts.length - 1];
  return Math.hypot(last[0] - first[0], last[1] - first[1]) < 1;
}

/**
 * One arrowhead at `tip`, pointing away from `from`.
 *
 * Supports the heads Excalidraw offers; an unknown one draws nothing rather
 * than guessing, so a newer Excalidraw's head is a missing decoration instead
 * of a wrong one.
 */
function arrowhead(
  el: ExcalidrawElement,
  from: [number, number] | undefined,
  tip: [number, number] | undefined,
  kind: string | null | undefined,
): string {
  if (!kind || kind === "none" || !from || !tip) return "";

  const size = Math.min(24, 9 + strokeWidth(el) * 2.5);
  const angle = Math.atan2(tip[1] - from[1], tip[0] - from[0]);
  const spread = 0.45;

  const color = escape(strokeOf(el));
  const left: [number, number] = [
    tip[0] - size * Math.cos(angle - spread),
    tip[1] - size * Math.sin(angle - spread),
  ];
  const right: [number, number] = [
    tip[0] - size * Math.cos(angle + spread),
    tip[1] - size * Math.sin(angle + spread),
  ];

  switch (kind) {
    case "arrow":
      return (
        `<path d="M ${r2(left[0])} ${r2(left[1])} L ${r2(tip[0])} ${r2(tip[1])}` +
        ` L ${r2(right[0])} ${r2(right[1])}" fill="none" stroke="${color}"` +
        ` stroke-width="${r2(strokeWidth(el))}" stroke-linecap="round" stroke-linejoin="round"/>`
      );
    case "triangle":
    case "triangle_outline":
      return (
        `<path d="M ${r2(tip[0])} ${r2(tip[1])} L ${r2(left[0])} ${r2(left[1])}` +
        ` L ${r2(right[0])} ${r2(right[1])} Z"` +
        (kind === "triangle"
          ? ` fill="${color}"/>`
          : ` fill="none" stroke="${color}" stroke-width="${r2(strokeWidth(el))}"/>`)
      );
    case "dot":
    case "circle":
    case "circle_outline":
      return (
        `<circle cx="${r2(tip[0])}" cy="${r2(tip[1])}" r="${r2(size / 2.6)}"` +
        (kind === "circle_outline"
          ? ` fill="none" stroke="${color}" stroke-width="${r2(strokeWidth(el))}"/>`
          : ` fill="${color}"/>`)
      );
    case "bar": {
      const bx = size * 0.6 * Math.cos(angle + Math.PI / 2);
      const by = size * 0.6 * Math.sin(angle + Math.PI / 2);
      return (
        `<path d="M ${r2(tip[0] - bx)} ${r2(tip[1] - by)} L ${r2(tip[0] + bx)} ${r2(tip[1] + by)}"` +
        ` fill="none" stroke="${color}" stroke-width="${r2(strokeWidth(el))}" stroke-linecap="round"/>`
      );
    }
    default:
      return "";
  }
}
