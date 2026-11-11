import assert from 'node:assert/strict';
import test from 'node:test';
import { countedResults } from './attempt.ts';

const row = (id: string, correct: boolean) => ({ q: { id }, correct });

test('without notCounted every result counts', () => {
  const rs = [row('a', true), row('b', false)];
  assert.deepEqual(countedResults(rs, undefined), { counted: rs, left: 0 });
  assert.deepEqual(countedResults(rs, []), { counted: rs, left: 0 });
});

test('results of deleted questions are left out and counted as left out', () => {
  const rs = [row('a', true), row('gone1', false), row('b', false), row('gone2', true)];
  const { counted, left } = countedResults(rs, ['gone1', 'gone2', 'never-shown']);
  assert.deepEqual(counted.map((r) => r.q.id), ['a', 'b']);
  assert.equal(left, 2);
});
