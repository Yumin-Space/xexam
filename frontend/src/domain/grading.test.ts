// 金标准单元测试：node --experimental-strip-types --test src/domain/*.test.ts
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { grade, inNotebook, percentOf, poolOf, shuffle, stateOf } from './grading.ts';
import type { Progress, Question } from '../types/api.ts';

const single = { kind: 'single', answer: ['B'], items: [] } as unknown as Pick<Question, 'kind' | 'answer' | 'items'>;
const multi = { kind: 'multi', answer: ['A', 'C'], items: [] } as unknown as Pick<Question, 'kind' | 'answer' | 'items'>;
const hot = { kind: 'hotspot', answer: [0, 1, 0], items: [{}, {}, {}] } as never;

test('stateOf: 单选 / 多选', () => {
  assert.equal(stateOf(single, undefined), 0);
  assert.equal(stateOf(single, null), 0);
  assert.equal(stateOf(single, []), 0);
  assert.equal(stateOf(single, ['B']), 2);
  assert.equal(stateOf(multi, ['A']), 2);
});

test('stateOf: 热点题（0 未答 / 1 部分 / 2 完成）', () => {
  assert.equal(stateOf(hot, undefined), 0);
  assert.equal(stateOf(hot, []), 0);
  assert.equal(stateOf(hot, [null, null, null]), 0);
  assert.equal(stateOf(hot, [0, null, null]), 1);
  assert.equal(stateOf(hot, [0, 1, null]), 1);
  assert.equal(stateOf(hot, [0, 1, 0]), 2);
  assert.equal(stateOf(hot, [0, 1]), 1, '比小项数短：不算完成');
  assert.equal(stateOf(hot, [0, 1, 0, 1]), 1, '比小项数长：不算完成');
});

test('grade: 单选', () => {
  assert.equal(grade(single, ['B']), true);
  assert.equal(grade(single, ['A']), false);
  assert.equal(grade(single, []), false);
  assert.equal(grade(single, undefined), false);
  assert.equal(grade(single, null), false);
  assert.equal(grade(single, ['B', 'C']), false);
});

test('grade: 多选要选对全部，和顺序无关', () => {
  assert.equal(grade(multi, ['A', 'C']), true);
  assert.equal(grade(multi, ['C', 'A']), true);
  assert.equal(grade(multi, ['A']), false);
  assert.equal(grade(multi, ['A', 'C', 'D']), false);
  assert.equal(grade(multi, ['A', 'B']), false);
});

test('grade: 热点题每个小项都对才算对', () => {
  assert.equal(grade(hot, [0, 1, 0]), true);
  assert.equal(grade(hot, [0, 1, 1]), false);
  assert.equal(grade(hot, [0, 1, null]), false);
  assert.equal(grade(hot, [0, 1]), false);
  assert.equal(grade(hot, [0, 1, 0, 0]), false);
  assert.equal(grade(hot, [null, null, null]), false);
  assert.equal(grade(hot, undefined), false);
});

test('inNotebook: 答错过且连续答对不到 2 次', () => {
  const s = (wrong: number, streak: number) => ({ attempts: wrong + streak, correct: streak, wrong, streak });
  assert.equal(inNotebook(undefined, 2), false);
  assert.equal(inNotebook(s(0, 5), 2), false, '从没答错');
  assert.equal(inNotebook(s(1, 0), 2), true);
  assert.equal(inNotebook(s(1, 1), 2), true);
  assert.equal(inNotebook(s(1, 2), 2), false, '连续答对 2 次后移出');
  assert.equal(inNotebook(s(3, 7), 2), false);
});

test('percentOf: 与 Python round() 一致（四舍六入五成双）', () => {
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

test('percentOf: 在 1..200 题的所有组合上与"精确半偶数"定义一致', () => {
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

test('poolOf: 按工作表和模式筛选', () => {
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

test('shuffle: 不改原数组，元素不丢不多', () => {
  const src = [1, 2, 3, 4, 5, 6, 7, 8];
  const out = shuffle(src);
  assert.deepEqual([...src], [1, 2, 3, 4, 5, 6, 7, 8]);
  assert.deepEqual([...out].sort(), [...src].sort());
  assert.deepEqual(shuffle(src, () => 0), [2, 3, 4, 5, 6, 7, 8, 1], '随机数固定时结果固定');
});
