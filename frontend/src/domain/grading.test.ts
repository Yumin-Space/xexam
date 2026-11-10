// Golden unit tests: node --experimental-strip-types --test src/domain/*.test.ts
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { completes, grade, inNotebook, percentOf, pickQuestions, poolOf, shuffle, stateOf } from './grading.ts';
import type { Progress, Question } from '../types/api.ts';

const single = { kind: 'single', answer: ['B'], items: [] } as unknown as Pick<Question, 'kind' | 'answer' | 'items'>;
const multi = { kind: 'multi', answer: ['A', 'C'], items: [] } as unknown as Pick<Question, 'kind' | 'answer' | 'items'>;
const hot = { kind: 'hotspot', answer: [0, 1, 0], items: [{}, {}, {}] } as never;

test('stateOf: single choice / multiple choice', () => {
  assert.equal(stateOf(single, undefined), 0);
  assert.equal(stateOf(single, null), 0);
  assert.equal(stateOf(single, []), 0);
  assert.equal(stateOf(single, ['B']), 2);
  assert.equal(stateOf(multi, ['A']), 2);
});

test('stateOf: hotspot question (0 unanswered / 1 partial / 2 complete)', () => {
  assert.equal(stateOf(hot, undefined), 0);
  assert.equal(stateOf(hot, []), 0);
  assert.equal(stateOf(hot, [null, null, null]), 0);
  assert.equal(stateOf(hot, [0, null, null]), 1);
  assert.equal(stateOf(hot, [0, 1, null]), 1);
  assert.equal(stateOf(hot, [0, 1, 0]), 2);
  assert.equal(stateOf(hot, [0, 1]), 1, 'shorter than the number of sub-items: not complete');
  assert.equal(stateOf(hot, [0, 1, 0, 1]), 1, 'longer than the number of sub-items: not complete');
});

test('completes: only the pick that fills the last open hotspot item (auto-advance trigger)', () => {
  assert.equal(completes(hot, [0, 1, null], [0, 1, 0]), true, 'last open item answered');
  assert.equal(completes(hot, [0, null, null], [0, 1, null]), false, 'still an open item');
  assert.equal(completes(hot, [0, 1, 0], [0, 0, 0]), false, 'changing an item of a complete question');
  assert.equal(completes(hot, [0, 1, 0], [0, null, 0]), false, 'clearing an item');
  assert.equal(completes(hot, undefined, [0, null, null]), false, 'first item of three');
  const one = { kind: 'hotspot', answer: [1], items: [{}] } as never;
  assert.equal(completes(one, undefined, [1]), true, 'single-item hotspot question');
  assert.equal(completes(one, [0], [1]), false, 'single-item hotspot question changed');
  assert.equal(completes(single, undefined, ['B']), true);
  assert.equal(completes(single, ['A'], ['B']), false);
});

test('grade: single choice', () => {
  assert.equal(grade(single, ['B']), true);
  assert.equal(grade(single, ['A']), false);
  assert.equal(grade(single, []), false);
  assert.equal(grade(single, undefined), false);
  assert.equal(grade(single, null), false);
  assert.equal(grade(single, ['B', 'C']), false);
});

test('grade: multiple choice must select all correct options, order does not matter', () => {
  assert.equal(grade(multi, ['A', 'C']), true);
  assert.equal(grade(multi, ['C', 'A']), true);
  assert.equal(grade(multi, ['A']), false);
  assert.equal(grade(multi, ['A', 'C', 'D']), false);
  assert.equal(grade(multi, ['A', 'B']), false);
});

test('grade: hotspot question is correct only if every sub-item is correct', () => {
  assert.equal(grade(hot, [0, 1, 0]), true);
  assert.equal(grade(hot, [0, 1, 1]), false);
  assert.equal(grade(hot, [0, 1, null]), false);
  assert.equal(grade(hot, [0, 1]), false);
  assert.equal(grade(hot, [0, 1, 0, 0]), false);
  assert.equal(grade(hot, [null, null, null]), false);
  assert.equal(grade(hot, undefined), false);
});

test('inNotebook: answered wrong before and fewer than 2 consecutive correct answers', () => {
  const s = (wrong: number, streak: number) => ({ attempts: wrong + streak, correct: streak, wrong, streak });
  assert.equal(inNotebook(undefined, 2), false);
  assert.equal(inNotebook(s(0, 5), 2), false, 'never answered wrong');
  assert.equal(inNotebook(s(1, 0), 2), true);
  assert.equal(inNotebook(s(1, 1), 2), true);
  assert.equal(inNotebook(s(1, 2), 2), false, 'removed after 2 consecutive correct answers');
  assert.equal(inNotebook(s(3, 7), 2), false);
});

test('percentOf: matches Python round() (round half to even)', () => {
  assert.equal(percentOf(1, 8), 12, '12.5 -> 12');
  assert.equal(percentOf(3, 8), 38, '37.5 -> 38');
  assert.equal(percentOf(5, 8), 62, '62.5 -> 62');
  assert.equal(percentOf(7, 8), 88, '87.5 -> 88');
  assert.equal(percentOf(1, 40), 2, '2.5 -> 2');
  assert.equal(percentOf(0, 5), 0);
  assert.equal(percentOf(5, 5), 100);
  assert.equal(percentOf(1, 3), 33);
  assert.equal(percentOf(2, 3), 67);
  assert.equal(percentOf(0, 0), 0);
});

test('percentOf: matches the "exact half-even" definition across all combinations of 1..200 questions', () => {
  for (let total = 1; total <= 200; total++) {
    for (let good = 0; good <= total; good++) {
      const exact = (100 * good) / total;
      const f = Math.floor(exact);
      const d = exact - f;
      let want: number;
      if (Math.abs(d - 0.5) < 1e-9) want = f % 2 === 0 ? f : f + 1;
      else want = Math.round(exact);
      assert.equal(percentOf(good, total), want, `${good}/${total}`);
    }
  }
});

test('poolOf: filter by sheet and mode', () => {
  const qs = [
    { id: 'a', sheet: 'S1' }, { id: 'b', sheet: 'S1' }, { id: 'c', sheet: 'S2' },
  ] as unknown as Question[];
  const progress: Progress = {
    version: 1, masterStreak: 2, history: [],
    questions: { b: { attempts: 1, correct: 0, wrong: 1, streak: 0 }, c: { attempts: 3, correct: 3, wrong: 1, streak: 3 } },
  };
  assert.deepEqual(poolOf(qs, new Set(['S1', 'S2']), 'all', progress).map((q) => q.id), ['a', 'b', 'c']);
  assert.deepEqual(poolOf(qs, new Set(['S1']), 'all', progress).map((q) => q.id), ['a', 'b']);
  assert.deepEqual(poolOf(qs, new Set(['S1', 'S2']), 'weak', progress).map((q) => q.id), ['b']);
  assert.deepEqual(poolOf(qs, new Set(['S1', 'S2']), 'weak', null).map((q) => q.id), []);
});

test('shuffle: does not mutate the original array; no elements lost or duplicated', () => {
  const src = [1, 2, 3, 4, 5, 6, 7, 8];
  const out = shuffle(src);
  assert.deepEqual([...src], [1, 2, 3, 4, 5, 6, 7, 8]);
  assert.deepEqual([...out].sort(), [...src].sort());
  assert.deepEqual(shuffle(src, () => 0), [2, 3, 4, 5, 6, 7, 8, 1], 'deterministic result when the random source is fixed');
});

test('pickQuestions: a smaller count picks a random subset even without shuffling the order', () => {
  const pool = Array.from({ length: 12 }, (_, i) => i);
  // deterministic pseudo-random sequence
  let x = 7;
  const rnd = (): number => ((x = (x * 9301 + 49297) % 233280) / 233280);
  const seen = new Set<number>();
  for (let k = 0; k < 200; k++) {
    const p = pickQuestions(pool, 11, false, rnd);
    assert.equal(p.length, 11);
    assert.deepEqual(p, [...p].sort((a, b) => a - b), 'bank order is kept when the order is not shuffled');
    for (const q of pool) if (!p.includes(q)) seen.add(q);
  }
  assert.ok(seen.size > 6, 'the left-out question changes from exam to exam, not always the last one');
  assert.deepEqual(pickQuestions(pool, 12, false, rnd), pool);
  assert.deepEqual(pickQuestions(pool, 99, false, rnd), pool);
  assert.equal(pickQuestions(pool, 0, true, rnd).length, 0);
  assert.deepEqual([...pickQuestions(pool, 12, true, rnd)].sort((a, b) => a - b), pool, 'shuffling keeps every question');
});
