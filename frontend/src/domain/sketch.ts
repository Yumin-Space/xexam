// v2.6: a tiny deterministic "hand drawn" helper for inline SVG (the Home chalk board). The look follows the well-known
// sketchy style: every line is drawn twice, each time through slightly moved points, so the two strokes never sit exactly
// on each other; straight segments bow a little; circles do not close perfectly; fills are hachure strokes. No dependency:
// a seeded PRNG makes the drawing the same on every render, so a repaint never makes the board "wobble".

export interface Pt {
  x: number;
  y: number;
}

/** The drawing area: every coordinate a helper writes is kept inside it */
export interface Box {
  w: number;
  h: number;
}

/** A seeded PRNG (mulberry32): the same seed gives the same sequence, values in [0, 1) */
export function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A number in [-1, 1) */
const sym = (rnd: () => number): number => rnd() * 2 - 1;

const clamp = (v: number, lo: number, hi: number): number => Math.min(hi, Math.max(lo, v));

/** One decimal is plenty for a chalk line and keeps the markup short */
const n1 = (v: number): string => {
  const r = Math.round(v * 10) / 10;
  return Object.is(r, -0) ? '0' : String(r);
};

const keep = (p: Pt, box: Box): Pt => ({ x: clamp(p.x, 0, box.w), y: clamp(p.y, 0, box.h) });

/**
 * A smooth path through the points (Catmull-Rom turned into cubic Béziers). Control points are kept inside the box, so the
 * curve never leaves it (a cubic stays inside the hull of its points).
 */
function smooth(pts: readonly Pt[], box: Box, closed = false): string {
  if (pts.length === 0) return '';
  const p = pts.map((q) => keep(q, box));
  if (p.length === 1) return `M${n1(p[0]!.x)} ${n1(p[0]!.y)}`;
  const at = (i: number): Pt => {
    if (closed) return p[((i % p.length) + p.length) % p.length]!;
    return p[clamp(i, 0, p.length - 1)]!;
  };
  let d = `M${n1(p[0]!.x)} ${n1(p[0]!.y)}`;
  const last = closed ? p.length : p.length - 1;
  for (let i = 0; i < last; i++) {
    const p0 = at(i - 1), p1 = at(i), p2 = at(i + 1), p3 = at(i + 2);
    const c1 = keep({ x: p1.x + (p2.x - p0.x) / 6, y: p1.y + (p2.y - p0.y) / 6 }, box);
    const c2 = keep({ x: p2.x - (p3.x - p1.x) / 6, y: p2.y - (p3.y - p1.y) / 6 }, box);
    d += ` C${n1(c1.x)} ${n1(c1.y)} ${n1(c2.x)} ${n1(c2.y)} ${n1(p2.x)} ${n1(p2.y)}`;
  }
  return d;
}

export interface CurveOptions {
  /** How far (px) each point may move per stroke; default 1.2 */
  jitter?: number;
}

/**
 * A curve through `pts`, drawn twice: two path strings, each through its own slightly moved copy of the points. The first
 * stroke moves less than the second (the second is the "quick retrace").
 */
export function sketchCurve(pts: readonly Pt[], rnd: () => number, box: Box, opts: CurveOptions = {}): [string, string] {
  const j = opts.jitter ?? 1.2;
  const pass = (k: number): string => smooth(pts.map((q) => ({ x: q.x + sym(rnd) * j * k, y: q.y + sym(rnd) * j * k })), box);
  return [pass(0.6), pass(1)];
}

/**
 * A straight segment drawn twice; each stroke bows a little to one side (a cubic whose control points sit off the line by a
 * few percent of its length) and its ends move slightly.
 */
export function sketchLine(a: Pt, b: Pt, rnd: () => number, box: Box): [string, string] {
  const len = Math.hypot(b.x - a.x, b.y - a.y) || 1;
  const nx = -(b.y - a.y) / len, ny = (b.x - a.x) / len;
  const pass = (): string => {
    const end = Math.min(2, len * 0.03);
    const s = keep({ x: a.x + sym(rnd) * end, y: a.y + sym(rnd) * end }, box);
    const e = keep({ x: b.x + sym(rnd) * end, y: b.y + sym(rnd) * end }, box);
    const bow = Math.min(len * 0.02, 1.6) * sym(rnd);
    const c1 = keep({ x: s.x + (e.x - s.x) * 0.3 + nx * bow * 2, y: s.y + (e.y - s.y) * 0.3 + ny * bow * 2 }, box);
    const c2 = keep({ x: s.x + (e.x - s.x) * 0.7 + nx * bow * 2, y: s.y + (e.y - s.y) * 0.7 + ny * bow * 2 }, box);
    return `M${n1(s.x)} ${n1(s.y)} C${n1(c1.x)} ${n1(c1.y)} ${n1(c2.x)} ${n1(c2.y)} ${n1(e.x)} ${n1(e.y)}`;
  };
  return [pass(), pass()];
}

/**
 * A circle drawn twice by hand: each stroke starts at a random angle, its radius wavers a little and it runs a bit past its
 * start (the circle does not close perfectly).
 */
export function sketchCircle(c: Pt, r: number, rnd: () => number, box: Box): [string, string] {
  const steps = Math.max(8, Math.min(16, Math.round(r * 0.9)));
  const pass = (): string => {
    const start = rnd() * Math.PI * 2;
    const over = 0.25 + rnd() * 0.3;           // radians past the start
    const ry = 1 + sym(rnd) * 0.06;            // a slightly squashed circle
    const pts: Pt[] = [];
    const total = steps + 1;
    for (let i = 0; i <= total; i++) {
      const ang = start + ((Math.PI * 2 + over) * i) / total;
      const rr = r * (1 + sym(rnd) * 0.05) + sym(rnd) * 0.4;
      pts.push({ x: c.x + Math.cos(ang) * rr, y: c.y + Math.sin(ang) * rr * ry });
    }
    return smooth(pts, box);
  };
  return [pass(), pass()];
}

/**
 * Hachure fill of a polygon: parallel strokes `gap` px apart at `angle` degrees, cut to the polygon (even-odd), each stroke
 * with slightly moved ends. Returns one path string holding all strokes (empty for a degenerate polygon).
 */
export function hachure(poly: readonly Pt[], gap: number, angle: number, rnd: () => number, box: Box): string {
  if (poly.length < 3 || gap <= 0) return '';
  const rad = (angle * Math.PI) / 180;
  const cos = Math.cos(rad), sin = Math.sin(rad);
  // Rotate the polygon so the strokes are horizontal, scan, rotate the segments back
  const rot = poly.map((p) => ({ x: p.x * cos + p.y * sin, y: -p.x * sin + p.y * cos }));
  const back = (p: Pt): Pt => ({ x: p.x * cos - p.y * sin, y: p.x * sin + p.y * cos });
  const ys = rot.map((p) => p.y);
  const top = Math.min(...ys), bottom = Math.max(...ys);
  const segs: string[] = [];
  for (let y = top + gap / 2; y < bottom; y += gap) {
    const xs: number[] = [];
    for (let i = 0; i < rot.length; i++) {
      const a = rot[i]!, b = rot[(i + 1) % rot.length]!;
      if ((a.y <= y && b.y > y) || (b.y <= y && a.y > y)) xs.push(a.x + ((y - a.y) / (b.y - a.y)) * (b.x - a.x));
    }
    xs.sort((p, q) => p - q);
    for (let i = 0; i + 1 < xs.length; i += 2) {
      const span = xs[i + 1]! - xs[i]!;
      if (span < 0.5) continue;
      const wob = Math.min(0.6, span * 0.08);
      const s = keep(back({ x: xs[i]! + Math.abs(sym(rnd)) * wob, y: y + sym(rnd) * 0.3 }), box);
      const e = keep(back({ x: xs[i + 1]! - Math.abs(sym(rnd)) * wob, y: y + sym(rnd) * 0.3 }), box);
      segs.push(`M${n1(s.x)} ${n1(s.y)} L${n1(e.x)} ${n1(e.y)}`);
    }
  }
  return segs.join(' ');
}

/** A polygon approximating a circle (for a hachure-filled dot) */
export function circlePoly(c: Pt, r: number, steps = 12): Pt[] {
  return Array.from({ length: steps }, (_, i) => {
    const a = (Math.PI * 2 * i) / steps;
    return { x: c.x + Math.cos(a) * r, y: c.y + Math.sin(a) * r };
  });
}
