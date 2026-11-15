// The rows of the usage bar charts: the "other" bar, the 万 / K format, the metric.
import assert from 'node:assert/strict';
import test from 'node:test';
import { cachePair, formatBig, limitHintArgs, modelRows, parsePeriod, purposeRows, sortRows, tokenFigures, tokenLabel, valueOf } from './usage-bars.ts';

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

test('A-OWNER-84 tokenFigures: input/output pair, cache write/read pair, row 2 hidden at 0, K / M', () => {
  assert.deepEqual(tokenFigures({ tokensIn: 85400, tokensOut: 59200, tokensCached: 76300, tokensCacheWrite: 6900 }), { io: '85.4K/59.2K', cache: '6.9K/76.3K' });
  assert.equal(tokenFigures({ tokensIn: 1434, tokensOut: 21907 }).cache, null);
  assert.equal(tokenFigures({ tokensIn: 1, tokensOut: 2, tokensCached: 0, tokensCacheWrite: 0 }).cache, null);
  assert.equal(tokenFigures({ tokensIn: 1, tokensOut: 2, tokensCached: 5 }).cache, '0/5');
  assert.equal(tokenFigures({ tokensIn: 1, tokensOut: 2, tokensCacheWrite: 1_234_567 }).cache, '1.2M/0');
  assert.equal(cachePair(6934, 76345), '6.9K/76.3K');
  assert.equal(cachePair(1, 2, (n) => String(n * 10)), '10/20');
});

test('sortRows: by amount high to low with stable ties, by name with CJK and natural numbers, input untouched', () => {
  const r = (label: string, calls: number, tokens = 0, other = false) => ({ key: label, label, other, ...c(calls, tokens, 0) });
  const rows = [r('b', 5), r('a', 5), r('c', 9), r('其他', 20, 0, true), r('m10', 1), r('m2', 1)];
  assert.deepEqual(sortRows(rows, 'amount', 'calls').map((x) => x.label), ['其他', 'c', 'b', 'a', 'm10', 'm2']);
  assert.deepEqual(sortRows(rows, 'name', 'calls', 'en').map((x) => x.label), ['a', 'b', 'c', 'm2', 'm10', '其他']);
  assert.deepEqual(sortRows([r('模型', 1), r('功能', 1)], 'name', 'calls', 'zh-CN').map((x) => x.label), ['功能', '模型']);
  // owner's name order: digit, Latin A-Z (case-insensitive, natural numbers), then other scripts; "other" last; same in every locale
  const mixed = [r('其他', 1, 0, true), r('モデル', 1), r('对话', 1), r('beta-10', 1), r('beta-2', 1), r('alpha', 1), r('Zeta', 1), r('9x', 1)];
  const want = ['9x', 'alpha', 'beta-2', 'beta-10', 'Zeta'];
  for (const loc of ['en', 'zh-CN', 'ja']) {
    const got = sortRows(mixed, 'name', 'calls', loc).map((x) => x.label);
    assert.deepEqual(got.slice(0, 5), want);
    assert.deepEqual(new Set(got.slice(5, 7)), new Set(['对话', 'モデル']));
    assert.equal(got[7], '其他');
  }
  assert.deepEqual(sortRows([r('x', 1, 50), r('y', 9, 10)], 'amount', 'tokens').map((x) => x.label), ['x', 'y']);
  assert.equal(rows[0]!.label, 'b');
});

test('v1.4 formatBig: B / T units, rounding carries into the next unit', () => {
  assert.deepEqual([0, 999, 1000, 1049, 999_949, 999_950, 1_000_000, 999_950_000, 1_000_000_000, 3_400_000_000, 999_950_000_000, 1_200_000_000_000].map(formatBig),
    ['0', '999', '1K', '1K', '999.9K', '1M', '1M', '1B', '1B', '3.4B', '1T', '1.2T']);
});

test('v1.4 tokenLabel: compact text, exact number for the title', () => {
  assert.deepEqual(tokenLabel(3_400_123_456, (n) => n.toLocaleString('en-US')), { text: '3.4B', title: '3,400,123,456' });
  assert.deepEqual(tokenLabel(85_400, (n) => n.toLocaleString('en-US')), { text: '85.4K', title: '85,400' });
});

test('v1.4 parsePeriod: the remembered period, unknown falls back to 7 days', () => {
  assert.equal(parsePeriod('12h'), '12h');
  assert.equal(parsePeriod('today'), 'today');
  assert.equal(parsePeriod('90'), 90);
  assert.equal(parsePeriod('365'), 365);
  assert.equal(parsePeriod('8'), 7);
  assert.equal(parsePeriod(null), 7);
  assert.equal(parsePeriod(undefined), 7);
});

test('v1.4 tokenFigures / cachePair: big figures render compact (B, T)', () => {
  assert.deepEqual(tokenFigures({ tokensIn: 3_400_000_000, tokensOut: 1_200_000_000_000, tokensCacheWrite: 85_400, tokensCached: 2_000_000 }), { io: '3.4B/1.2T', cache: '85.4K/2M' });
  assert.equal(cachePair(999, 1000), '999/1K');
});

test('v1.4 limitHintArgs: a token limit is compact with the exact numbers, a call limit stays exact', () => {
  const ex = (n: number): string => n.toLocaleString('en-US');
  const a = limitHintArgs(true, 1000, 3_400_000_000, 8000, ex);
  assert.deepEqual(a.compact, { min: '1K', max: '3.4B', def: '8K' });
  assert.equal(a.exact.max, '3,400,000,000');
  assert.equal(limitHintArgs(false, 1, 600, 5, ex).compact, null);
});
