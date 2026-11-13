// The rows of the usage bar charts: the "other" bar, the 万 / K format, the metric.
import assert from 'node:assert/strict';
import test from 'node:test';
import { formatBig, modelRows, purposeRows, valueOf } from './usage-bars.ts';

const c = (calls: number, tokensIn = 0, tokensOut = 0, errors = 0) => ({ calls, tokensIn, tokensOut, errors });

test('formatBig: K / M in every language, never 万, exact below 1 000', () => {
  assert.equal(formatBig(999), '999');
  assert.equal(formatBig(1000), '1K');
  assert.equal(formatBig(1234), '1.2K');
  assert.equal(formatBig(48123), '48.1K');
  assert.equal(formatBig(226000), '226K');
  assert.equal(formatBig(999_960), '1M');
  assert.equal(formatBig(1_250_000), '1.3M');
  assert.equal(formatBig(3_400_000), '3.4M');
  assert.equal(formatBig(0), '0');
  assert.ok(!formatBig(123_456_789).includes('万'));
});

test('modelRows: a call without a model goes to the other bar, so the bars add up', () => {
  const rows = modelRows([{ model: 'm-a', ...c(200, 10, 5, 1) }, { model: '', ...c(42) }, { model: 'm-b', ...c(3) }], 'Other');
  assert.deepEqual(rows.map((r) => r.label), ['m-a', 'm-b', 'Other']);
  assert.equal(rows.reduce((s, r) => s + r.calls, 0), 245);
  assert.equal(rows[2]!.other, true);
  assert.equal(modelRows([{ model: 'm', ...c(1) }], 'Other').length, 1);
});

test('purposeRows: the other bar holds what the features do not, both charts reach the total', () => {
  const total = c(245, 1000, 500, 4);
  const rows = purposeRows(['chat', 'hint'] as const, [{ purpose: 'chat', ...c(200, 800, 400, 3) }, { purpose: 'hint', ...c(3, 10, 5) }], total, (k) => k, 'Other');
  assert.deepEqual(rows.map((r) => r.label), ['chat', 'hint', 'Other']);
  assert.deepEqual([rows[2]!.calls, rows[2]!.tokensIn, rows[2]!.tokensOut, rows[2]!.errors], [42, 190, 95, 1]);
  assert.equal(rows.reduce((s, r) => s + r.calls, 0), 245);
  // nothing left over: no other bar; a total below the sum never makes a negative bar
  assert.equal(purposeRows(['chat'] as const, [{ purpose: 'chat', ...c(5) }], c(5), (k) => k, 'Other').length, 1);
  assert.equal(purposeRows(['chat'] as const, [{ purpose: 'chat', ...c(5) }], c(3), (k) => k, 'Other').length, 1);
});

test('valueOf: calls, or input + output tokens', () => {
  assert.equal(valueOf(c(7, 30, 12), 'calls'), 7);
  assert.equal(valueOf(c(7, 30, 12), 'tokens'), 42);
});

test('modelRows: the cache figures are summed when a row has them and left out when none did (A-OWNER-74)', () => {
  const rows = modelRows([{ model: 'm-a', ...c(2), tokensCached: 5, tokensCacheWrite: 7 }, { model: 'm-a', ...c(1), tokensCacheWrite: 3 }, { model: 'm-b', ...c(1) }], 'Other');
  assert.equal(rows[0]?.tokensCached, 5);
  assert.equal(rows[0]?.tokensCacheWrite, 10);
  assert.equal('tokensCached' in (rows[1] ?? {}), false);
});
