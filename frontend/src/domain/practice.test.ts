// Practice mode (v0.6.5): checking single / multiple-choice / hotspot questions, locking, the answer-sheet marks and the
// results saved when a practice (or an exam) is finished. Made-up questions only.
import assert from 'node:assert/strict';
import test from 'node:test';
import { kindOf } from './attempt.ts';
import { barSlots, canCheck, checksOnPick, finalResults, isLocked, listNumbers, openCount, openNumbers, practiceCounts, practiceMark, sheetMatches, type BarInput } from './practice.ts';
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

// ---------------------------------------------------------------- the action bar (v1.0, QZ-3)

const base: BarInput = { kind: 'practice', qType: 'single', answered: false, checked: false, first: false, last: false, ai: 'usable' };
const names = (i: BarInput): string[] => barSlots(i).slots.map((x) => x.slot);

test('barSlots: the slot list is the same for every question of one page (kind and AI state decide it, nothing else)', () => {
  for (const ai of ['none', 'usable', 'greyed'] as const) {
    for (const kind of ['practice', 'exam'] as const) {
      const want = names({ ...base, kind, ai });
      for (const qType of ['single', 'multi', 'hotspot'] as const) {
        for (const answered of [false, true]) {
          for (const checked of [false, true]) {
            for (const first of [false, true]) {
              for (const last of [false, true]) {
                assert.deepEqual(names({ kind, qType, answered, checked, first, last, ai }), want);
              }
            }
          }
        }
      }
    }
  }
  assert.deepEqual(names(base), ['prev', 'main', 'hint', 'explain', 'flag']);
  assert.deepEqual(names({ ...base, ai: 'none' }), ['prev', 'main', 'flag']);
  // an exam never has the AI buttons, whatever the AI state says
  assert.deepEqual(names({ ...base, kind: 'exam', ai: 'usable' }), ['prev', 'main', 'flag']);
});

test('barSlots: Previous is off on the first question, the flag is always on, the AI buttons follow the AI state', () => {
  const on = (i: BarInput, slot: string): boolean | undefined => barSlots(i).slots.find((x) => x.slot === slot)?.enabled;
  assert.equal(on({ ...base, first: true }, 'prev'), false);
  assert.equal(on({ ...base, first: false }, 'prev'), true);
  assert.equal(on({ ...base, first: true }, 'flag'), true);
  assert.equal(on({ ...base, ai: 'usable' }, 'hint'), true);
  assert.equal(on({ ...base, ai: 'greyed' }, 'hint'), false);
  assert.equal(on({ ...base, ai: 'greyed' }, 'explain'), false);
});

test('barSlots: the main area is [Check answer, Next | Finish] in a practice and [Next | Submit] in an exam, never merged', () => {
  for (const kind of ['practice', 'exam'] as const) {
    for (const qType of ['single', 'multi', 'hotspot'] as const) {
      for (const answered of [false, true]) {
        for (const checked of [false, true]) {
          for (const last of [false, true]) {
            const main = barSlots({ kind, qType, answered, checked, first: false, last, ai: 'usable' }).main;
            const end = last ? (kind === 'practice' ? 'finish' : 'submit') : 'next';
            assert.deepEqual(main.map((x) => x.act), kind === 'practice' ? ['check', end] : [end], `${kind} ${qType} ${answered} ${checked} ${last}`);
            // Next / Finish / Submit is never disabled
            assert.equal(main[main.length - 1]?.enabled, true);
          }
        }
      }
    }
  }
});

test('barSlots: Check answer is usable once answered, only until checked, and a single-choice question is checked by the pick', () => {
  const checkOf = (i: BarInput) => barSlots(i).main.find((x) => x.act === 'check');
  assert.equal(checkOf({ ...base, kind: 'exam', qType: 'multi' }), undefined);
  for (const qType of ['multi', 'hotspot'] as const) {
    assert.equal(checkOf({ ...base, qType, answered: false })?.enabled, false);
    assert.equal(checkOf({ ...base, qType, answered: true })?.enabled, true);
    assert.equal(checkOf({ ...base, qType, answered: true, checked: true })?.enabled, false);
  }
  // single choice: the button stays in its place, greyed (the pick has already checked it)
  assert.equal(checkOf({ ...base, qType: 'single', answered: true })?.enabled, false);
  assert.equal(checkOf({ ...base, qType: 'single', answered: true, checked: true })?.enabled, false);
});

test('openNumbers lists the 1-based numbers a finish would leave open; listNumbers caps them at 10', () => {
  const qs = [S1, M1, { ...S1, id: 's2' }];
  assert.deepEqual(openNumbers('practice', qs, {}, new Set(['m'])), [1, 3]);
  assert.deepEqual(openNumbers('exam', qs, { s: ['B'] }, new Set()), [2, 3]);
  const many = Array.from({ length: 13 }, (_, i) => i + 1);
  assert.deepEqual(listNumbers(many), { shown: many.slice(0, 10), more: 3 });
  assert.deepEqual(listNumbers([4, 9]), { shown: [4, 9], more: 0 });
});

test('sheetMatches: each chip keeps only its tiles', () => {
  const t = { open: true, wrong: false, flagged: true };
  assert.equal(sheetMatches('all', t), true);
  assert.equal(sheetMatches('open', t), true);
  assert.equal(sheetMatches('wrong', t), false);
  assert.equal(sheetMatches('flag', { ...t, flagged: false }), false);
});
