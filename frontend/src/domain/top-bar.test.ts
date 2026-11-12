import assert from 'node:assert/strict';
import { test } from 'node:test';
import { BAR_START, barStep, initialBar, type BarState } from './top-bar.ts';

const run = (s: BarState, ys: number[], max = 5000): BarState => ys.reduce((acc, y) => barStep(acc, y, max), s);

test('the bar stays while the page is within the first 56 px', () => {
  assert.equal(run(initialBar(), [10, 30, BAR_START]).hidden, false);
});

test('scrolling down by more than 8 px after the first 56 px hides the bar', () => {
  assert.equal(run(initialBar(100), [105, 110]).hidden, true);
  assert.equal(run(initialBar(100), [105]).hidden, false, 'one small step is not enough');
  assert.equal(run(initialBar(), [30, 50]).hidden, false, 'inside the first 56 px nothing hides');
});

test('an upward scroll of 8 px shows it again, a smaller one does not', () => {
  const hidden = run(initialBar(100), [200, 300]);
  assert.equal(hidden.hidden, true);
  assert.equal(barStep(hidden, 295, 5000).hidden, true);
  assert.equal(barStep(hidden, 292, 5000).hidden, true);
  assert.equal(barStep(hidden, 291, 5000).hidden, false);
});

test('slow small moves add up in one direction', () => {
  assert.equal(run(initialBar(100), [103, 106, 109]).hidden, true);
});

test('a change of direction restarts the count', () => {
  const s = run(initialBar(200), [204, 202, 206]);
  assert.equal(s.hidden, false);
});

test('the very top shows the bar', () => {
  const hidden = run(initialBar(100), [300]);
  assert.equal(barStep(hidden, 0, 5000).hidden, false);
});

test('rubber-band positions change nothing', () => {
  const hidden = run(initialBar(100), [300]);
  assert.equal(barStep(hidden, -20, 5000), hidden);
  assert.equal(barStep(hidden, 5030, 5000), hidden);
  const shown = initialBar(1000);
  assert.equal(barStep(shown, 1500, 1400), shown);
});
