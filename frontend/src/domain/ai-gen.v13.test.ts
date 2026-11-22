// v1.3: the questions written so far (cursor, gone numbers), the polling rule and the placeholder bank row.
import assert from 'node:assert/strict';
import test from 'node:test';
import type { GenJob, GenStatus } from '../types/api.ts';
import { applyItems, emptyItems, placeholderOf, runningLine, wantsItems } from './ai-gen.ts';

const gen = (o: Partial<GenStatus> = {}): GenStatus => ({
  perDay: 3, used: 1, left: 2, unlimited: false, resetAt: 1, minSource: 30, minWrong: 10, bank: { single: 14, multi: 6 },
  wrong: { single: 14, multi: 6 }, job: null, unseen: [], ...o,
});
const job = (o: Partial<GenJob> = {}): GenJob => ({
  id: 1, state: 'running', phase: 'writing', source: 'bank', name: 'n', from: [], fromCount: 0, file: null, mode: null, read: null,
  confirmBy: null, errorArg: null, skipped: null, made: 8, goal: 20, calls: 1, createdAt: 1, startedAt: 1, finishedAt: null, bankId: null,
  error: null, charged: true, ...o,
});
const item = (no: number, stem = `q${no}`) => ({ no, kind: 'single' as const, stem, options: [{ text: 'a', correct: true }], explanation: '' });

test('runningLine: the planning phase has its own line and no percentage', () => {
  assert.deepEqual(runningLine(job({ phase: 'planning', made: 0 })), { key: 'aigen.phase.planning', pct: 0 });
});

test('applyItems: appends in order, keeps the cursor, removes gone numbers, never doubles a number', () => {
  let st = applyItems(emptyItems(), { items: [item(1), item(2)], last: 2, gone: [] });
  assert.deepEqual(st.items.map((i) => i.no), [1, 2]);
  assert.equal(st.last, 2);
  st = applyItems(st, { items: [item(3), item(2, 'again')], last: 4, gone: [1] });
  assert.deepEqual(st.items.map((i) => i.no), [2, 3]);
  assert.equal(st.items[0]!.stem, 'again');
  assert.equal(st.last, 4);
  // the cursor never moves back, and an answer without items changes nothing
  assert.deepEqual(applyItems(st, { last: 1 }), st);
  assert.equal(applyItems(st, {}).last, 4);
});

test('applyItems: the cursor stays valid when the newest item was removed (numbers are never reused)', () => {
  const st = applyItems(applyItems(emptyItems(), { items: [item(7), item(8)], last: 8 }), { gone: [8] });
  assert.deepEqual(st.items.map((i) => i.no), [7]);
  assert.equal(st.last, 8);
});

test('wantsItems: only a running generation on Home / Bank management with a visible page; never on the quiz', () => {
  const j = job();
  assert.equal(wantsItems('study', true, j), true);
  assert.equal(wantsItems('banks', true, j), true);
  assert.equal(wantsItems('quiz', true, j), false);
  assert.equal(wantsItems('study', false, j), false);
  assert.equal(wantsItems('settings', true, j), false);
  assert.equal(wantsItems('study', true, job({ source: 'file' })), false);
  assert.equal(wantsItems('study', true, job({ state: 'done' })), false);
  assert.equal(wantsItems('study', true, null), false);
});

test('placeholderOf: the running generation only; none for an import, a finished job or the quiz route', () => {
  const g = gen({ job: job({ name: 'AI bank', made: 8, goal: 20 }) });
  assert.deepEqual(placeholderOf(g, 'study'), { id: 1, name: 'AI bank', made: 8, goal: 20 });
  assert.equal(placeholderOf(g, 'quiz'), null);
  assert.equal(placeholderOf(gen({ job: job({ source: 'file' }) }), 'study'), null);
  assert.equal(placeholderOf(gen({ job: job({ state: 'done' }) }), 'study'), null);
  assert.equal(placeholderOf(null, 'study'), null);
});
