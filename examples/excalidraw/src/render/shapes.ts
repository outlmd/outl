/**
 * The closed shapes: rectangle, diamond, ellipse — plus images and frames.
 *
 * Every one follows the same two-path recipe from `rough.ts`: a clean closed
 * path takes the fill, a jittered one takes the stroke. The differences
 * between them are only which points the outline runs through.
 */

import type { ExcalidrawElement } from "../types";
import {
  box,
  cornerRadius,
  diamondPoints,
  opacity,
  rotation,
  strokeAttrs,
  strokeWidth,
} from "./attrs";
import { Fills } from "./fill";
import { asString, escape } from "../escape";
import {
  doubled,
  ellipseFill,
  ellipseSketch,
  jitter,
  polygonFill,
  polygonSketch,
  r2,
  seeded,
} from "./rough";

/** A rectangle, corners rounded when the element says so. */
export function renderRectangle(el: ExcalidrawElement, fills: Fills): string {
  return renderPolygon(el, fills, box(el), cornerRadius(el));
}

/**
 * A diamond.
 *
 * Its corner radius comes from the same `cornerRadius`, halved: the formula is
 * written for a box, and a diamond's edges are the box's diagonals, so the full
 * radius would eat most of a small diamond's edge.
 */
export function renderDiamond(el: ExcalidrawElement, fills: Fills): string {
  return renderPolygon(el, fills, diamondPoints(el), cornerRadius(el) / 2);
}

/** Shared body for every closed polygon. */
function renderPolygon(
  el: ExcalidrawElement,
  fills: Fills,
  pts: [number, number][],
  radius: number,
): string {
  const rand = seeded(el.seed);
  const amount = jitter(el.roughness);
  const fill = fills.paint(el.backgroundColor, el.fillStyle, strokeWidth(el));

  let body = "";
  if (fill !== "none") {
    body += `<path d="${polygonFill(pts, radius)}" fill="${escape(fill)}"/>`;
  }
  body +=
    `<path d="${doubled(() => polygonSketch(rand, pts, radius, amount), amount)}"` +
    `${strokeAttrs(el)}/>`;

  return group(el, body);
}

/** An ellipse inscribed in the element's box. */
export function renderEllipse(el: ExcalidrawElement, fills: Fills): string {
  const rand = seeded(el.seed);
  const amount = jitter(el.roughness);

  const cx = (el.x || 0) + (el.width || 0) / 2;
  const cy = (el.y || 0) + (el.height || 0) / 2;
  const rx = Math.abs(el.width || 0) / 2;
  const ry = Math.abs(el.height || 0) / 2;

  const fill = fills.paint(el.backgroundColor, el.fillStyle, strokeWidth(el));

  let body = "";
  if (fill !== "none") {
    body += `<path d="${ellipseFill(cx, cy, rx, ry)}" fill="${escape(fill)}"/>`;
  }
  body +=
    `<path d="${doubled(() => ellipseSketch(rand, cx, cy, rx, ry, amount), amount)}"` +
    `${strokeAttrs(el)}/>`;

  return group(el, body);
}

/**
 * An embedded image.
 *
 * The API hands back every file as a `dataURL`, so an image needs no network
 * from inside the sandboxed iframe — which is the only reason images can render
 * here at all. A file the cache dropped (over its size budget) draws as a
 * labelled placeholder instead of a blank gap, so the scene does not quietly
 * lose a picture.
 */
export function renderImage(el: ExcalidrawElement, files: Record<string, string>): string {
  const x = el.x || 0;
  const y = el.y || 0;
  const w = Math.abs(el.width || 0);
  const h = Math.abs(el.height || 0);

  const href = el.fileId ? files[el.fileId] : undefined;
  if (typeof href !== "string" || href === "") return group(el, placeholder(x, y, w, h));

  return group(
    el,
    `<image href="${escape(href)}" x="${r2(x)}" y="${r2(y)}" width="${r2(w)}" height="${r2(h)}"` +
      ` preserveAspectRatio="none"/>`,
  );
}

/** The stand-in for an image whose bytes are not in the cache. */
function placeholder(x: number, y: number, w: number, h: number): string {
  const label = w >= 90 && h >= 28 ? "image not cached" : "";
  const text =
    label === ""
      ? ""
      : `<text x="${r2(x + w / 2)}" y="${r2(y + h / 2 + 4)}" text-anchor="middle"` +
        ` font-family="ui-sans-serif, sans-serif" font-size="12" fill="#868e96">${label}</text>`;

  return (
    `<rect x="${r2(x)}" y="${r2(y)}" width="${r2(w)}" height="${r2(h)}" rx="4"` +
    ` fill="rgba(134,142,150,.1)" stroke="#adb5bd" stroke-width="1" stroke-dasharray="4 4"/>` +
    text
  );
}

/**
 * A frame: the outline and its name.
 *
 * Drawn clean rather than sketched, because a frame is chrome in Excalidraw
 * too — it is not part of the drawing, it is the box around part of it.
 */
export function renderFrame(el: ExcalidrawElement): string {
  const x = el.x || 0;
  const y = el.y || 0;
  const w = Math.abs(el.width || 0);
  const h = Math.abs(el.height || 0);
  const name = asString(el.name);

  const label =
    name === ""
      ? ""
      : `<text x="${r2(x)}" y="${r2(y - 6)}" font-family="ui-sans-serif, sans-serif"` +
        ` font-size="12" fill="#868e96">${escape(name)}</text>`;

  return (
    `<g${opacity(el)}>` +
    `<rect x="${r2(x)}" y="${r2(y)}" width="${r2(w)}" height="${r2(h)}" rx="6"` +
    ` fill="none" stroke="#bbb" stroke-width="1.5"/>${label}</g>`
  );
}

/** Wrap an element's body in its rotation and opacity. */
function group(el: ExcalidrawElement, body: string): string {
  return `<g${rotation(el)}${opacity(el)}>${body}</g>`;
}
