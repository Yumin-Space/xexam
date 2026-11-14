// The usage chart geometry (UC-1): ticks, gap filling, the monotone path, label thinning.
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  wrapLabel, areaPath, axisMax, bars, controlYs, dayRange, fillPeriods, labelStep, monotonePath, monthRange, niceMax, xs, yOf, yTicks,
  PHONE_CHART, fitText, hbars, horizontalBars, nameColumn, newestIndexes, shortPeriod, textWidth,
} from './usage-chart.ts';

test('niceMax rounds up to 1, 2, 5, 10 times a power of ten', () => {
  assert.deepEqual([0, 1, 1.5, 2, 3, 5, 6, 9, 10, 11, 47, 120, 999].map(niceMax), [1, 1, 2, 2, 5, 5, 10, 10, 10, 20, 50, 200, 1000]);
});

test('yTicks: whole numbers from 0 to a multiple of a nice step', () => {
  assert.deepEqual(yTicks(0), [0, 1]);
  assert.deepEqual(yTicks(1), [0, 1]);
  assert.deepEqual(yTicks(3), [0, 1, 2, 3]);
  assert.deepEqual(yTicks(4), [0, 1, 2, 3, 4]);
  assert.deepEqual(yTicks(8), [0, 2, 4, 6, 8]);
  assert.deepEqual(yTicks(47), [0, 20, 40, 60]);
  assert.deepEqual(yTicks(100), [0, 50, 100]);
  assert.deepEqual(yTicks(1234), [0, 500, 1000, 1500]);
  for (const m of [0, 1, 7, 33, 250, 9999]) {
    const t = yTicks(m);
    assert.equal(t[0], 0);
    assert.ok(t[t.length - 1]! >= m);
    assert.ok(t.length <= 6);
    assert.ok(t.every((v, i) => Number.isInteger(v) && (i === 0 || v > t[i - 1]!)));
  }
  assert.equal(axisMax(47), 60);
});

test('dayRange / monthRange: inclusive, over month and year ends, empty on bad input', () => {
  assert.deepEqual(dayRange('2026-11-29', '2026-12-02'), ['2026-11-29', '2026-11-30', '2026-12-01', '2026-12-02']);
  assert.deepEqual(dayRange('2028-02-28', '2028-03-01'), ['2028-02-28', '2028-02-29', '2028-03-01']);
  assert.deepEqual(dayRange('2026-11-05', '2026-11-05'), ['2026-11-05']);
  assert.deepEqual(dayRange('2026-11-06', '2026-11-05'), []);
  assert.deepEqual(dayRange('x', '2026-11-05'), []);
  assert.deepEqual(monthRange('2026-11-15', '2027-02-03'), ['2026-11', '2026-12', '2027-01', '2027-02']);
  assert.deepEqual(monthRange('2026-11', '2026-11'), ['2026-11']);
  assert.deepEqual(monthRange('2026-12', '2026-11'), []);
});

test('fillPeriods: gaps are 0, full YYYY-MM-DD labels for days, YYYY-MM for months, an out-of-range row is kept', () => {
  const rows = [{ period: '2026-11-02', calls: 4 }, { period: '2026-11-04', calls: 1 }];
  assert.deepEqual(fillPeriods('day', '2026-11-01', '2026-11-05', rows), [
    { period: '2026-11-01', calls: 0 }, { period: '2026-11-02', calls: 4 }, { period: '2026-11-03', calls: 0 },
    { period: '2026-11-04', calls: 1 }, { period: '2026-11-05', calls: 0 },
  ]);
  assert.deepEqual(fillPeriods('month', '2026-09-10', '2026-11-05', [{ period: '2026-10', calls: 7 }]), [
    { period: '2026-09', calls: 0 }, { period: '2026-10', calls: 7 }, { period: '2026-11', calls: 0 },
  ]);
  assert.deepEqual(fillPeriods('day', '2026-11-02', '2026-11-03', [{ period: '2026-11-01', calls: 9 }]).map((p) => p.period), ['2026-11-01', '2026-11-02', '2026-11-03']);
  assert.deepEqual(fillPeriods('day', '2026-11-01', '2026-11-01', []), [{ period: '2026-11-01', calls: 0 }]);
  // the same period twice adds up
  assert.equal(fillPeriods('day', '2026-11-01', '2026-11-01', [{ period: '2026-11-01', calls: 2 }, { period: '2026-11-01', calls: 3 }])[0]!.calls, 5);
});

test('monotonePath: passes through every point and never leaves the range of two neighbours (no dip below 0)', () => {
  const values = [0, 0, 12, 0, 0, 3, 3, 40, 1, 0];
  const px = xs(values.length, 10, 270);
  const pts = values.map((v, i): [number, number] => [px[i]!, yOf(v, 40, 5, 100)]);
  const path = monotonePath(pts);
  assert.ok(path.startsWith('M10 105'));
  // the baseline is y = 105: a control point is never below it (that would be a dip below 0) nor above the top
  for (const y of controlYs(path)) assert.ok(y <= 105.0001 && y >= 5 - 0.0001, `control point ${y}`);
  // each segment's control points stay between its end points' y (monotone)
  const segs = path.split('C').slice(1).map((s) => s.trim().split(/\s+/).map(Number));
  segs.forEach((c, i) => {
    const lo = Math.min(pts[i]![1], pts[i + 1]![1]) - 1e-6;
    const hi = Math.max(pts[i]![1], pts[i + 1]![1]) + 1e-6;
    assert.ok(c[1]! >= lo && c[1]! <= hi && c[3]! >= lo && c[3]! <= hi, `segment ${i}`);
    assert.equal(c[4], Math.round(pts[i + 1]![0] * 100) / 100);
  });
  assert.equal(segs.length, values.length - 1);
});

test('monotonePath: degenerate inputs and the closed area', () => {
  assert.equal(monotonePath([]), '');
  assert.equal(monotonePath([[5, 7]]), 'M5 7');
  assert.equal(monotonePath([[0, 10], [10, 0]]), 'M0 10L10 0');
  assert.equal(monotonePath([[0, 5], [10, 5], [20, 5]]), 'M0 5C3.33 5 6.67 5 10 5C13.33 5 16.67 5 20 5');
  assert.equal(areaPath('M0 5L10 5', 0, 10, 50), 'M0 5L10 5L10 50L0 50Z');
  assert.equal(areaPath('', 0, 10, 50), '');
  assert.deepEqual(xs(1, 0, 100), [50]);
  assert.deepEqual(xs(3, 10, 100), [10, 60, 110]);
  assert.deepEqual(xs(0, 0, 10), []);
});

test('label thinning and rotation: never overlap', () => {
  // 30 days in 300 px: slot 10 px, a 70 px label needs every 8th one
  assert.equal(labelStep(10, 70), 8);
  assert.equal(labelStep(100, 70), 1);
  assert.equal(labelStep(40, 70), 2);
  assert.equal(labelStep(0, 70), 1);
  for (const slot of [4, 9, 17, 33, 61]) {
    const step = labelStep(slot, 70);
    assert.ok(step * slot >= 70, `slot ${slot} step ${step}`);
  }
});

test('bars: centred in their slots, heights follow the values, a zero is flat', () => {
  const b = bars([0, 5, 10], { left: 0, top: 0, width: 300, height: 100 }, 10);
  assert.equal(b.length, 3);
  assert.deepEqual(b.map((x) => x.height), [0, 50, 100]);
  assert.deepEqual(b.map((x) => x.y), [100, 50, 0]);
  assert.ok(b.every((x) => x.width <= 36 && x.width >= 2));
  assert.ok(b[1]!.x > b[0]!.x && b[1]!.x + b[1]!.width < b[2]!.x);
  assert.deepEqual(bars([], { left: 0, top: 0, width: 10, height: 10 }, 5), []);
  assert.deepEqual(bars([1], { left: 0, top: 0, width: 10, height: 10 }, 0), []);
});

test('wrapLabel: Latin labels break at spaces, CJK between characters, never inside a Latin word', () => {
  assert.deepEqual(wrapLabel('Chat', 80), ['Chat']);
  assert.deepEqual(wrapLabel('Practice hints', 60), ['Practice', 'hints']);
  assert.equal(wrapLabel('Explanation', 40), null);
  assert.deepEqual(wrapLabel('练习提示', 80), ['练习提示']);
  assert.deepEqual(wrapLabel('练习提示与解释', 48), ['练习提示', '与解释']);
  assert.deepEqual(wrapLabel('練習のヒント', 58), ['練習のヒ', 'ント']);
  assert.equal(wrapLabel('一二三四五六七八九十一二', 30), null);
});

test('horizontalBars: a phone-wide box, or any label the column chart cannot show flat', () => {
  const flat = [['Chat'], ['Hints']];
  assert.equal(horizontalBars(PHONE_CHART - 1, flat), true);
  assert.equal(horizontalBars(393, flat), true);
  assert.equal(horizontalBars(PHONE_CHART, flat), false);
  assert.equal(horizontalBars(1000, flat), false);
  assert.equal(horizontalBars(1000, [['Chat'], null]), true);
  assert.equal(horizontalBars(1000, []), false);
});

test('fitText: cut with an ellipsis by estimated width, never empty, never wider than the room', () => {
  assert.equal(fitText('Chat', 100), 'Chat');
  const long = 'example-model-long-name-5-5';
  for (const px of [30, 60, 90, 140]) {
    const c = fitText(long, px);
    assert.ok(c.endsWith('…'));
    assert.ok(textWidth(c) <= px + 0.01, `${c} in ${px}`);
    assert.ok(long.startsWith(c.slice(0, -1)));
  }
  assert.equal(fitText('练习提示与解释', 12), '练…');
  assert.equal(fitText('abc', 0), 'a…');
});

test('nameColumn: the widest name, at most 40 percent of the box, at least 40 px', () => {
  assert.equal(nameColumn([50, 80], 360), 80);
  assert.equal(nameColumn([50, 300], 360), 144);
  assert.equal(nameColumn([10], 360), 40);
  assert.equal(nameColumn([], 360), 40);
  assert.equal(nameColumn([500], 200), 80);
});

test('shortPeriod: MM-DD for a day, a month stays', () => {
  assert.equal(shortPeriod('2026-11-13'), '11-13');
  assert.equal(shortPeriod('2026-11'), '2026-11');
  assert.equal(shortPeriod('x'), 'x');
});

test('newestIndexes: counted from the newest, the newest is always named, none touch', () => {
  assert.deepEqual(newestIndexes(7, 3), [0, 3, 6]);
  assert.deepEqual(newestIndexes(7, 1), [0, 1, 2, 3, 4, 5, 6]);
  assert.deepEqual(newestIndexes(1, 5), [0]);
  assert.deepEqual(newestIndexes(0, 2), []);
  // a label the box cannot hold is dropped, the newest never
  assert.deepEqual(newestIndexes(7, 3, (i) => i !== 0), [3, 6]);
  assert.deepEqual(newestIndexes(7, 3, () => false), [6]);
  for (const n of [2, 8, 30, 90]) for (const step of [1, 2, 4, 7]) {
    const idx = newestIndexes(n, step);
    assert.equal(idx[idx.length - 1], n - 1);
    assert.ok(idx.every((v, k) => k === 0 || v - idx[k - 1]! === step));
  }
});

test('hbars: rows from the top, the bar grows right in proportion, centred in its row', () => {
  const b = hbars([10, 5, 0], { left: 100, top: 6, width: 200 }, 10, 30, 16);
  assert.equal(b.length, 3);
  assert.deepEqual(b.map((x) => x.width), [200, 100, 0]);
  assert.deepEqual(b.map((x) => x.rowY), [6, 36, 66]);
  assert.ok(b.every((x) => x.x === 100 && x.height === 16 && x.y === x.rowY + 7));
  assert.deepEqual(hbars([], { left: 0, top: 0, width: 10 }, 1), []);
  assert.deepEqual(hbars([1], { left: 0, top: 0, width: 10 }, 0), []);
  assert.equal(hbars([-3], { left: 0, top: 0, width: 10 }, 5)[0]!.width, 0);
});
