import assert from 'node:assert/strict';
import test from 'node:test';
import { roundWait, secondsLeft } from './wait.ts';

test('waits are rounded up to whole minutes below an hour', () => {
  assert.deepEqual(roundWait(1), { n: 1, unit: 'minute' });
  assert.deepEqual(roundWait(0.2), { n: 1, unit: 'minute' });
  assert.deepEqual(roundWait(60), { n: 1, unit: 'minute' });
  assert.deepEqual(roundWait(61), { n: 2, unit: 'minute' });
  assert.deepEqual(roundWait(300), { n: 5, unit: 'minute' });
  assert.deepEqual(roundWait(899), { n: 15, unit: 'minute' });
  assert.deepEqual(roundWait(1800), { n: 30, unit: 'minute' });
  assert.deepEqual(roundWait(59 * 60), { n: 59, unit: 'minute' });
});

test('an hour or more is shown in hours, a day or more in days, never less than the real wait', () => {
  assert.deepEqual(roundWait(59 * 60 + 1), { n: 1, unit: 'hour' });
  assert.deepEqual(roundWait(3600), { n: 1, unit: 'hour' });
  assert.deepEqual(roundWait(3601), { n: 2, unit: 'hour' });
  assert.deepEqual(roundWait(23 * 3600), { n: 23, unit: 'hour' });
  assert.deepEqual(roundWait(23 * 3600 + 1), { n: 1, unit: 'day' });
  assert.deepEqual(roundWait(86_400), { n: 1, unit: 'day' });
  assert.deepEqual(roundWait(86_401), { n: 2, unit: 'day' });
});

test('nonsense input never shows "0 minutes"', () => {
  assert.deepEqual(roundWait(0), { n: 1, unit: 'minute' });
  assert.deepEqual(roundWait(-5), { n: 1, unit: 'minute' });
  assert.deepEqual(roundWait(Number.NaN), { n: 1, unit: 'minute' });
});

test('seconds left until a deadline on the same clock', () => {
  assert.equal(secondsLeft(10_000, 4_000), 6);
  assert.equal(secondsLeft(10_000, 10_000), 0);
  assert.equal(secondsLeft(10_000, 12_000), 0);
});
