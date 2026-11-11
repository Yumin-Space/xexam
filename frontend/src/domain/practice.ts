// Practice mode (v0.6.5): the quiz page with immediate feedback. Pure rules, no DOM, no network, so the checking logic is
// tested on its own (practice.test.ts):
//   · a single-choice question is checked as soon as an option is chosen; multiple-choice and hotspot questions have a
//     "Check answer" button, usable once something is chosen;
//   · a checked question is locked: its answer can no longer change;
//   · when the practice is finished, every checked question counts with its answer; an unchecked question counts as not
//     answered (wrong), like an unanswered question of an exam - whatever was chosen on it without checking.
import type { AttemptKind, Question } from '../types/api.ts';
import { grade, stateOf, type Answer } from './grading.ts';

type Q = Pick<Question, 'kind' | 'items' | 'answer'>;

/** Single-choice questions are checked by the pick itself (no "Check answer" button) */
export function checksOnPick(q: Pick<Question, 'kind'>): boolean {
  return q.kind === 'single';
}

/** Whether "Check answer" can be pressed: not checked yet, and something is chosen (any hotspot item is enough) */
export function canCheck(q: Pick<Question, 'kind' | 'items'>, a: Answer, checked: boolean): boolean {
  return !checked && stateOf(q, a) > 0;
}

/** Whether the answer of a question may still change: always in an exam, until it is checked in a practice */
export function isLocked(kind: AttemptKind, checked: ReadonlySet<string>, id: string): boolean {
  return kind === 'practice' && checked.has(id);
}

/** A question of a practice on the answer sheet: not checked yet, or checked and right / wrong */
export type PracticeMark = 'open' | 'right' | 'wrong';

export function practiceMark(q: Q, a: Answer, checked: boolean): PracticeMark {
  if (!checked) return 'open';
  return grade(q, a) ? 'right' : 'wrong';
}

/** "Checked 4 of 10, 3 right": how far a practice is */
export function practiceCounts<T extends Q & { id: string }>(
  qs: readonly T[],
  answers: Readonly<Record<string, Answer>>,
  checked: ReadonlySet<string>,
): { checked: number; right: number } {
  let c = 0;
  let r = 0;
  for (const q of qs) {
    if (!checked.has(q.id)) continue;
    c++;
    if (grade(q, answers[q.id])) r++;
  }
  return { checked: c, right: r };
}

/** How many questions a "Finish" would leave open: unanswered ones of an exam, unchecked ones of a practice */
export function openCount<T extends Q & { id: string }>(
  kind: AttemptKind,
  qs: readonly T[],
  answers: Readonly<Record<string, Answer>>,
  checked: ReadonlySet<string>,
): number {
  if (kind === 'practice') return qs.filter((q) => !checked.has(q.id)).length;
  return qs.filter((q) => stateOf(q, answers[q.id]) < 2).length;
}

/**
 * The graded results of a finished exam or practice, in question order. Exam: every answer as given. Practice: the answers
 * of checked questions; an unchecked question has no answer (null) and is wrong.
 */
export function finalResults<T extends Q & { id: string }>(
  kind: AttemptKind,
  qs: readonly T[],
  answers: Readonly<Record<string, Answer>>,
  checked: ReadonlySet<string>,
): Array<{ q: T; ans: Answer; correct: boolean }> {
  return qs.map((q) => {
    const given = answers[q.id];
    const ans: Answer = Array.isArray(given) && (kind === 'exam' || checked.has(q.id)) ? given : null;
    return { q, ans, correct: grade(q, ans) };
  });
}
