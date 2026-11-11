/**
 * The hand-drawn look, in about two hundred lines.
 *
 * Excalidraw draws through [roughjs](https://roughjs.com): every stroke is
 * jittered and drawn twice, so a rectangle looks sketched rather than printed.
 * Bundling roughjs is not an option here (the plugin ships as one bundle into a
 * JS interpreter with a loop-iteration cap), so this is the part of it that
 * matters for a static render: a seeded PRNG, a jittered edge, a double stroke.
 *
 * **Fill and stroke are separate paths, and that is load-bearing.** A sketchy
 * outline is a handful of disconnected `M…Q` sub-paths — pretty, and useless to
 * fill, because a filled path has to close. So every shape produces two: a
 * clean closed path that gets the fill, and a jittered one that gets the
 * stroke. roughjs does the same thing for the same reason.
 *
 * **Seeded, not random.** Every element carries a `seed`, so a scene draws
 * identically on every client and on every repaint. `Math.random()` here would
 * make a drawing flicker each time it is rendered.
 */

/** A deterministic 0..1 generator. */
export type Rand = () => number;

/**
 * Park–Miller minimal standard PRNG.
 *
 * Chosen over the usual bit-twiddling generators because it stays in exact
 * float arithmetic (`s * 16807` peaks below 2^53), so it cannot depend on how
 * an engine treats `|0` or whether it has `Math.imul`.
 */
export function seeded(seed: number | undefined): Rand {
  let s = Math.abs(Math.floor(seed || 1)) % 2147483647;
  if (s <= 0) s += 2147483646;
  return () => {
    s = (s * 16807) % 2147483647;
    return (s - 1) / 2147483646;
  };
}

/**
 * How far a stroke may wander, from the element's `roughness`.
 *
 * Excalidraw's three presets are architect (0), artist (1) and cartoonist (2);
 * roughjs scales its offsets by the same number. Capped so a cartoonist-grade
 * scene stays legible at small sizes.
 */
export function jitter(roughness: number | undefined): number {
  const r = typeof roughness === "number" ? roughness : 1;
  if (r <= 0) return 0;
  return Math.min(2.4, 0.9 * r);
}

/** A random offset in `[-amount, amount]`. */
function off(rand: Rand, amount: number): number {
  return amount === 0 ? 0 : (rand() * 2 - 1) * amount;
}

/** Round to 2dp: shorter path data, no visible difference. */
export function r2(n: number): number {
  return Math.round(n * 100) / 100;
}

/** A point `d` units from `a` along the straight line towards `b`. */
export function along(
  ax: number,
  ay: number,
  bx: number,
  by: number,
  d: number,
): [number, number] {
  const dx = bx - ax;
  const dy = by - ay;
  const len = Math.hypot(dx, dy);
  if (len === 0 || d <= 0) return [ax, ay];
  const t = Math.min(1, d / len);
  return [ax + dx * t, ay + dy * t];
}

/**
 * One jittered edge, as a quadratic curve, starting with `M`.
 *
 * The bow in the middle is what makes a straight line read as hand-drawn: the
 * endpoints move a little, the control point moves more.
 */
export function sketchEdge(
  rand: Rand,
  x1: number,
  y1: number,
  x2: number,
  y2: number,
  amount: number,
): string {
  const mx = (x1 + x2) / 2 + off(rand, amount * 1.8);
  const my = (y1 + y2) / 2 + off(rand, amount * 1.8);
  const sx = x1 + off(rand, amount);
  const sy = y1 + off(rand, amount);
  const ex = x2 + off(rand, amount);
  const ey = y2 + off(rand, amount);
  return `M ${r2(sx)} ${r2(sy)} Q ${r2(mx)} ${r2(my)} ${r2(ex)} ${r2(ey)}`;
}

/**
 * Draw `build` twice so the outline doubles back over itself the way a pen
 * does. The second pass draws from the same PRNG, so it lands near the first
 * without tracing it. A roughness of 0 means one clean pass.
 */
export function doubled(build: () => string, amount: number): string {
  const first = build();
  if (amount === 0) return first;
  return `${first} ${build()}`;
}

/**
 * Corner points of a closed polygon, expanded so each corner is a pair of
 * points with the vertex between them.
 *
 * Both path builders below walk this, so "where does a rounded corner start"
 * is answered once. `radius <= 0` collapses each pair onto the vertex, which
 * is how a sharp polygon falls out of the same code.
 */
function corners(
  pts: [number, number][],
  radius: number,
): { start: [number, number]; vertex: [number, number]; end: [number, number] }[] {
  const out: { start: [number, number]; vertex: [number, number]; end: [number, number] }[] = [];
  for (let i = 0; i < pts.length; i++) {
    const prev = pts[(i + pts.length - 1) % pts.length];
    const vertex = pts[i];
    const next = pts[(i + 1) % pts.length];

    const edgeIn = Math.hypot(vertex[0] - prev[0], vertex[1] - prev[1]);
    const edgeOut = Math.hypot(next[0] - vertex[0], next[1] - vertex[1]);
    const rIn = Math.min(radius, edgeIn / 2);
    const rOut = Math.min(radius, edgeOut / 2);

    out.push({
      start: along(vertex[0], vertex[1], prev[0], prev[1], rIn),
      vertex,
      end: along(vertex[0], vertex[1], next[0], next[1], rOut),
    });
  }
  return out;
}

/**
 * Clean closed polygon path — the one that gets filled.
 *
 * Rectangles and diamonds are both closed polygons whose corners Excalidraw
 * rounds the same way, so one function covers both and there is a single place
 * where "a rounded corner looks like this" is decided.
 */
export function polygonFill(pts: [number, number][], radius: number): string {
  if (pts.length < 3) return "";
  const cs = corners(pts, radius);

  let d = `M ${r2(cs[0].start[0])} ${r2(cs[0].start[1])}`;
  for (let i = 0; i < cs.length; i++) {
    const c = cs[i];
    if (radius > 0) {
      d += ` Q ${r2(c.vertex[0])} ${r2(c.vertex[1])} ${r2(c.end[0])} ${r2(c.end[1])}`;
    } else {
      d += ` L ${r2(c.vertex[0])} ${r2(c.vertex[1])}`;
    }
    const next = cs[(i + 1) % cs.length];
    d += ` L ${r2(next.start[0])} ${r2(next.start[1])}`;
  }
  return d + " Z";
}

/** Jittered polygon outline — the one that gets stroked. */
export function polygonSketch(
  rand: Rand,
  pts: [number, number][],
  radius: number,
  amount: number,
): string {
  if (pts.length < 2) return "";
  const cs = corners(pts, radius);

  let d = "";
  for (let i = 0; i < cs.length; i++) {
    const from = cs[i].end;
    const to = cs[(i + 1) % cs.length].start;
    // The straight run between two corners, jittered.
    d += (d === "" ? "" : " ") + sketchEdge(rand, from[0], from[1], to[0], to[1], amount);
    if (radius > 0) {
      // Then the corner itself, as its own stroke so a sharp and a rounded
      // polygon take the same path through this loop.
      const c = cs[(i + 1) % cs.length];
      d +=
        ` M ${r2(c.start[0] + off(rand, amount))} ${r2(c.start[1] + off(rand, amount))}` +
        ` Q ${r2(c.vertex[0])} ${r2(c.vertex[1])} ${r2(c.end[0])} ${r2(c.end[1])}`;
    }
  }
  return d;
}

/** Clean ellipse path — the one that gets filled. */
export function ellipseFill(cx: number, cy: number, rx: number, ry: number): string {
  return arcs(cx, cy, rx, ry, () => 0);
}

/** Jittered ellipse path — the one that gets stroked. */
export function ellipseSketch(
  rand: Rand,
  cx: number,
  cy: number,
  rx: number,
  ry: number,
  amount: number,
): string {
  return arcs(cx, cy, rx, ry, () => off(rand, amount));
}

/**
 * An ellipse as four cubic arcs, each coordinate nudged by `j()`.
 *
 * `kappa` is the control-point distance that makes a cubic match a quarter
 * circle; with `j()` returning 0 this is the exact ellipse.
 */
function arcs(
  cx: number,
  cy: number,
  rx: number,
  ry: number,
  j: () => number,
): string {
  const kappa = 0.5522847498;
  const ox = rx * kappa;
  const oy = ry * kappa;

  return (
    `M ${r2(cx - rx + j())} ${r2(cy + j())}` +
    ` C ${r2(cx - rx + j())} ${r2(cy - oy + j())} ${r2(cx - ox + j())} ${r2(cy - ry + j())} ${r2(cx + j())} ${r2(cy - ry + j())}` +
    ` C ${r2(cx + ox + j())} ${r2(cy - ry + j())} ${r2(cx + rx + j())} ${r2(cy - oy + j())} ${r2(cx + rx + j())} ${r2(cy + j())}` +
    ` C ${r2(cx + rx + j())} ${r2(cy + oy + j())} ${r2(cx + ox + j())} ${r2(cy + ry + j())} ${r2(cx + j())} ${r2(cy + ry + j())}` +
    ` C ${r2(cx - ox + j())} ${r2(cy + ry + j())} ${r2(cx - rx + j())} ${r2(cy + oy + j())} ${r2(cx - rx + j())} ${r2(cy + j())}` +
    " Z"
  );
}

/**
 * Smooth open path through `pts`, each point reached via the midpoint of its
 * neighbours. A curved Excalidraw line and a freehand stroke are both this:
 * the same points, drawn without corners.
 */
export function smooth(pts: [number, number][]): string {
  if (pts.length === 0) return "";
  if (pts.length === 1) return `M ${r2(pts[0][0])} ${r2(pts[0][1])}`;
  if (pts.length === 2) {
    return `M ${r2(pts[0][0])} ${r2(pts[0][1])} L ${r2(pts[1][0])} ${r2(pts[1][1])}`;
  }

  let d = `M ${r2(pts[0][0])} ${r2(pts[0][1])}`;
  for (let i = 1; i < pts.length - 1; i++) {
    const mx = (pts[i][0] + pts[i + 1][0]) / 2;
    const my = (pts[i][1] + pts[i + 1][1]) / 2;
    d += ` Q ${r2(pts[i][0])} ${r2(pts[i][1])} ${r2(mx)} ${r2(my)}`;
  }
  const last = pts[pts.length - 1];
  const beforeLast = pts[pts.length - 2];
  return d + ` Q ${r2(beforeLast[0])} ${r2(beforeLast[1])} ${r2(last[0])} ${r2(last[1])}`;
}

/** Open polyline through `pts`, every segment jittered. */
export function polylineSketch(
  rand: Rand,
  pts: [number, number][],
  amount: number,
): string {
  if (pts.length < 2) return "";
  let d = "";
  for (let i = 0; i < pts.length - 1; i++) {
    d +=
      (d === "" ? "" : " ") +
      sketchEdge(rand, pts[i][0], pts[i][1], pts[i + 1][0], pts[i + 1][1], amount);
  }
  return d;
}
