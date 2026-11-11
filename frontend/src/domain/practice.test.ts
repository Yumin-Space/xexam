// Practice mode (v0.6.5): checking single / multiple-choice / hotspot questions, locking, the answer-sheet marks and the
// results saved when a practice (or an exam) is finished. Made-up questions only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { kindOf } from './attempt.ts';
import { canCheck, checksOnPick, finalResults, isLocked, openCount, practiceCounts, practiceMark } from './practice.ts';
import type { Answer } from './grading.ts';
import type { Question } from '../types/api.ts';

type Q = Pick<Question, 'id' | 'kind' | 'items' | 'answer'>;
const S1: Q = { id: 's', kind: 'single', items: [], answer: ['B'] };
const M1: Q = { id: 'm', kind: 'multi', items: [], answer: ['A', 'C'] };
const H1: Q = {
  id: 'h', kind: 'hotspot', answer: [0, 1],
  items: [{ text: 'Water is wet.', choices: ['Yes', 'No'] }, { text: 'Fire is cold.', choices: ['Yes', 'No'] }],
};
const qs: Q[] = [S1, M1, H1];

test('single choice is checked by the pick; multiple choice and hotspot need "Check answer"', () => {
  assert.equal(checksOnPick(S1), true);
  assert.equal(checksOnPick(M1), false);
  assert.equal(checksOnPick(H1), false);
});

test('"Check answer" is usable once something is chosen and not after checking', () => {
  assert.equal(canCheck(M1, undefined, false), false);
  assert.equal(canCheck(M1, [], false), false);
  assert.equal(canCheck(M1, ['A'], false), true);
  assert.equal(canCheck(M1, ['A'], true), false);
  // a hotspot question: one item answered is enough to check (the other items then count as wrong)
  assert.equal(canCheck(H1, [null, null], false), false);
  assert.equal(canCheck(H1, [0, null], false), true);
  assert.equal(canCheck(H1, [0, 1], true), false);
});

test('a checked question is locked in a practice only; an exam never locks', () => {
  const checked = new Set(['m']);
  assert.equal(isLocked('practice', checked, 'm'), true);
  assert.equal(isLocked('practice', checked, 's'), false);
  assert.equal(isLocked('exam', checked, 'm'), false);
});

test('answer-sheet marks: open until checked, then right or wrong by the usual grading', () => {
  assert.equal(practiceMark(S1, ['B'], false), 'open');
  assert.equal(practiceMark(S1, ['B'], true), 'right');
  assert.equal(practiceMark(S1, ['A'], true), 'wrong');
  assert.equal(practiceMark(M1, ['C', 'A'], true), 'right');
  assert.equal(practiceMark(M1, ['A'], true), 'wrong');
  assert.equal(practiceMark(H1, [0, 1], true), 'right');
  assert.equal(practiceMark(H1, [0, null], true), 'wrong');
});

test('counts: how many are checked and how many of those are right', () => {
  const answers: Record<string, Answer> = { s: ['B'], m: ['A'], h: [0, 1] };
  assert.deepEqual(practiceCounts(qs, answers, new Set(['s', 'm'])), { checked: 2, right: 1 });
  assert.deepEqual(practiceCounts(qs, answers, new Set()), { checked: 0, right: 0 });
});

test('open questions at the end: unanswered ones of an exam, unchecked ones of a practice', () => {
  const answers: Record<string, Answer> = { s: ['B'], m: ['A'], h: [0, null] };
  assert.equal(openCount('exam', qs, answers, new Set()), 1); // the half-answered hotspot
  assert.equal(openCount('practice', qs, answers, new Set(['s'])), 2); // m and h are chosen but not checked
  assert.equal(openCount('practice', qs, answers, new Set(['s', 'm', 'h'])), 0);
});

test('final results: a practice counts unchecked questions as not answered, an exam keeps every answer', () => {
  const answers: Record<string, Answer> = { s: ['B'], m: ['A', 'C'], h: [0, 1] };
  const practice = finalResults('practice', qs, answers, new Set(['s']));
  assert.deepEqual(practice.map((r) => [r.q.id, r.ans, r.correct]), [['s', ['B'], true], ['m', null, false], ['h', null, false]]);
  const exam = finalResults('exam', qs, answers, new Set());
  assert.deepEqual(exam.map((r) => [r.q.id, r.correct]), [['s', true], ['m', true], ['h', true]]);
  // nothing chosen at all
  assert.deepEqual(finalResults('exam', qs, {}, new Set()).map((r) => [r.ans, r.correct]), [[null, false], [null, false], [null, false]]);
});

test('kindOf: only "practice" is a practice; missing or unknown values are exams (records from before v0.6.5)', () => {
  assert.equal(kindOf({ kind: 'practice' }), 'practice');
  assert.equal(kindOf({ kind: 'exam' }), 'exam');
  for (const v of [undefined, null, 'PRACTICE', 1, {}]) assert.equal(kindOf({ kind: v }), 'exam');
  assert.equal(kindOf(null), 'exam');
});
