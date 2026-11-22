// v2.6 hand-drawn SVG helper (the Home chalk board): the drawing is the same for the same seed, every line is two strokes and
// no coordinate leaves the drawing area. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { circlePoly, hachure, rng, sketchCircle, sketchCurve, sketchLine, type Box } from './sketch.ts';

const BOX: Box = { w: 270, h: 200 };

/** Every x / y pair of a path's commands (M, C, L carry absolute pairs only) */
function pairs(d: string): [number, number][] {
  const nums = (d.match(/-?\d+(\.\d+)?/g) ?? []).map(Number);
  assert.equal(nums.length % 2, 0, `odd number count in ${d}`);
  const out: [number, number][] = [];
  for (let i = 0; i < nums.length; i += 2) out.push([nums[i]!, nums[i + 1]!]);
  return out;
}

const inside = (d: string, box: Box): boolean => pairs(d).every(([x, y]) => x >= 0 && x <= box.w && y >= 0 && y <= box.h);

test('rng: the same seed gives the same sequence in [0, 1), another seed another one', () => {
  const a = rng(42), b = rng(42), c = rng(43);
  const xs = Array.from({ length: 50 }, () => a());
  assert.deepEqual(xs, Array.from({ length: 50 }, () => b()));
  assert.notDeepEqual(xs, Array.from({ length: 50 }, () => c()));
  assert.ok(xs.every((x) => x >= 0 && x < 1));
});

const arc = Array.from({ length: 13 }, (_, i) => ({ x: 20 + i * 19, y: 40 + ((i - 6) ** 2) * 3.5 }));

test('sketchCurve: two strokes, deterministic for a seed, slightly apart', () => {
  const one = sketchCurve(arc, rng(7), BOX);
  assert.equal(one.length, 2);
  assert.deepEqual(sketchCurve(arc, rng(7), BOX), one);
  assert.notDeepEqual(sketchCurve(arc, rng(8), BOX), one);
  assert.notEqual(one[0], one[1], 'the two strokes do not sit on each other');
  for (const d of one) {
    assert.match(d, /^M[\d.]+ [\d.]+( C[\d. ]+)+$/);
    // One cubic per segment between the given points
    assert.equal((d.match(/C/g) ?? []).length, arc.length - 1);
  }
});

test('sketchCurve moves points only a little (the jitter), never far from the true curve', () => {
  const [a, b] = sketchCurve(arc, rng(3), BOX, { jitter: 1.4 });
  for (const d of [a!, b!]) {
    const ends = pairs(d).filter((_, i) => i === 0 || i % 3 === 0);   // M point, then the end of each C
    ends.forEach(([x, y], i) => {
      assert.ok(Math.abs(x - arc[i]!.x) <= 1.41 && Math.abs(y - arc[i]!.y) <= 1.41, `point ${i}`);
    });
  }
});

test('every helper keeps its coordinates inside the box, also for shapes at the very edge', () => {
  const edge = [{ x: 0, y: 0 }, { x: 135, y: 200 }, { x: 270, y: 0 }];
  for (let seed = 1; seed <= 40; seed++) {
    for (const d of sketchCurve(edge, rng(seed), BOX, { jitter: 3 })) assert.ok(inside(d, BOX), `curve ${seed}: ${d}`);
    for (const d of sketchLine({ x: 0, y: 200 }, { x: 270, y: 0 }, rng(seed), BOX)) assert.ok(inside(d, BOX), `line ${seed}`);
    for (const d of sketchCircle({ x: 4, y: 196 }, 12, rng(seed), BOX)) assert.ok(inside(d, BOX), `circle ${seed}`);
    assert.ok(inside(hachure(circlePoly({ x: 2, y: 2 }, 8), 1.8, -41, rng(seed), BOX), BOX), `hachure ${seed}`);
  }
});

test('sketchLine: two bowed strokes near the segment, deterministic', () => {
  const s = sketchLine({ x: 10, y: 100 }, { x: 210, y: 100 }, rng(5), BOX);
  assert.equal(s.length, 2);
  assert.deepEqual(sketchLine({ x: 10, y: 100 }, { x: 210, y: 100 }, rng(5), BOX), s);
  for (const d of s) {
    assert.match(d, /^M[\d.]+ [\d.]+ C[\d.]+ [\d.]+ [\d.]+ [\d.]+ [\d.]+ [\d.]+$/);
    // The bow stays small: every control point within a few px of the line
    for (const [, y] of pairs(d)) assert.ok(Math.abs(y - 100) <= 5, `y ${y}`);
  }
});

test('sketchCircle: two strokes that wander around the radius and overshoot the start a little', () => {
  const c = { x: 100, y: 100 };
  const s = sketchCircle(c, 15, rng(11), BOX);
  assert.equal(s.length, 2);
  assert.deepEqual(sketchCircle(c, 15, rng(11), BOX), s);
  assert.notEqual(s[0], s[1]);
  for (const d of s) {
    const ends = pairs(d).filter((_, i) => i === 0 || i % 3 === 0);
    for (const [x, y] of ends) {
      const r = Math.hypot(x - c.x, y - c.y);
      assert.ok(r > 12 && r < 18.5, `radius ${r}`);
    }
    // Not closed exactly: the stroke does not end on its start point
    const all = pairs(d);
    assert.notDeepEqual(all[0], all[all.length - 1]);
  }
});

test('hachure: parallel strokes cut to the polygon, deterministic, empty for a degenerate shape', () => {
  const sq = [{ x: 20, y: 20 }, { x: 60, y: 20 }, { x: 60, y: 60 }, { x: 20, y: 60 }];
  const d = hachure(sq, 4, -41, rng(2), BOX);
  assert.equal(d, hachure(sq, 4, -41, rng(2), BOX));
  const strokes = d.split('M').filter(Boolean);
  assert.ok(strokes.length >= 10, `strokes ${strokes.length}`);
  for (const st of strokes) {
    for (const [x, y] of pairs(`M${st}`)) assert.ok(x >= 19.5 && x <= 60.5 && y >= 19.5 && y <= 60.5, `${x},${y}`);
  }
  assert.equal(hachure(sq.slice(0, 2), 4, -41, rng(2), BOX), '');
  assert.equal(hachure(sq, 0, -41, rng(2), BOX), '');
});

test('circlePoly: points on the circle', () => {
  const poly = circlePoly({ x: 10, y: 10 }, 5, 8);
  assert.equal(poly.length, 8);
  for (const p of poly) assert.ok(Math.abs(Math.hypot(p.x - 10, p.y - 10) - 5) < 1e-9);
});
