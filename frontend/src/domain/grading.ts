// Grading and wrong-answer notebook rules (pure functions, no DOM, no network).
//
// Note: the spec requires "the frontend contains no authoritative business rules", but the old version's grading and wrong-answer notebook rules lived in the browser, and the server only stores {id, correct}.
// To keep the API backward compatible, the old behavior is kept here, isolated as pure functions + golden tests (grading.test.ts).
// This is a deliberate deviation from the spec; see Q3 in docs/migration-plan.md.
import type { Mode, Progress, Question, QuestionStat } from '../types/api.ts';

/** An answer. Single / multiple choice: the selected letters; hotspot question: the index of the choice picked for each item (null if none). */
export type Answer = Array<string | number | null> | undefined | null;

/** 0 unanswered; 1 partial (hotspot questions only); 2 fully answered */
export type AnswerState = 0 | 1 | 2;

export function stateOf(q: Pick<Question, 'kind' | 'items'>, a: Answer): AnswerState {
  if (!Array.isArray(a)) return 0;
  if (q.kind === 'hotspot') {
    const c = a.filter((x) => x != null).length;
    return c === 0 ? 0 : c === q.items.length ? 2 : 1;
  }
  return a.length ? 2 : 0;
}

export function grade(q: Pick<Question, 'kind' | 'answer'>, a: Answer): boolean {
  if (!Array.isArray(a)) return false;
  const right = q.answer as Array<string | number>;
  if (q.kind === 'hotspot') {
    return a.length === right.length && right.every((x, i) => a[i] === x);
  }
  const mine = [...a].map(String).sort();
  const want = [...right].map(String).sort();
  return mine.length === want.length && mine.every((x, i) => x === want[i]);
}

/** Wrong-answer notebook: answered wrong before, and the consecutive-correct streak is still below masterStreak. */
export function inNotebook(stat: QuestionStat | undefined | null, masterStreak: number): boolean {
  return !!stat && stat.wrong > 0 && stat.streak < masterStreak;
}

export function masterStreakOf(p: Progress | null | undefined): number {
  return (p && p.masterStreak) || 2;
}

/** Question scope: the questions in the selected sheets; "wrong-answer notebook" mode keeps only those in the notebook. */
export function poolOf(
  questions: Question[],
  selected: ReadonlySet<string>,
  mode: Mode,
  progress: Progress | null | undefined,
): Question[] {
  const ms = masterStreakOf(progress);
  return questions.filter(
    (q) => selected.has(q.sheet) && (mode !== 'weak' || inNotebook(progress?.questions?.[q.id], ms)),
  );
}

/**
 * Percentage: round half to even (banker's rounding), matching the server (Python's round),
 * so the numbers on the results page and in the study history never differ by 1. Uses integer math to avoid floating-point error.
 */
export function percentOf(good: number, total: number): number {
  if (total <= 0) return 0;
  const num = 100 * good;
  const q = Math.floor(num / total);
  const r2 = 2 * (num - q * total);
  if (r2 < total) return q;
  if (r2 > total) return q + 1;
  return q % 2 === 0 ? q : q + 1;
}

/** Fisher–Yates */
export function shuffle<T>(arr: readonly T[], random: () => number = Math.random): T[] {
  const a = arr.slice();
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(random() * (i + 1));
    const tmp = a[i] as T;
    a[i] = a[j] as T;
    a[j] = tmp;
  }
  return a;
}
