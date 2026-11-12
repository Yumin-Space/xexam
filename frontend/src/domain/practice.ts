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

/** 1-based numbers of the questions a "Finish" would leave open (same rule as openCount) */
export function openNumbers<T extends Q & { id: string }>(
  kind: AttemptKind,
  qs: readonly T[],
  answers: Readonly<Record<string, Answer>>,
  checked: ReadonlySet<string>,
): number[] {
  const out: number[] = [];
  qs.forEach((q, i) => {
    const open = kind === 'practice' ? !checked.has(q.id) : stateOf(q, answers[q.id]) < 2;
    if (open) out.push(i + 1);
  });
  return out;
}

/** "4, 9, 12" for up to `max` numbers; `more` is how many were left out (0 = none) */
export function listNumbers(nums: readonly number[], max = 10): { shown: number[]; more: number } {
  return { shown: nums.slice(0, max), more: Math.max(0, nums.length - max) };
}

/** The answer sheet's filter chips (QZ-5): all, open (not answered / not checked), wrong (practice only), flagged */
export type SheetFilter = 'all' | 'open' | 'wrong' | 'flag';

export function sheetMatches(f: SheetFilter, tile: { open: boolean; wrong: boolean; flagged: boolean }): boolean {
  if (f === 'open') return tile.open;
  if (f === 'wrong') return tile.wrong;
  if (f === 'flag') return tile.flagged;
  return true;
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

// ---------------------------------------------------------------- the action bar (v1.0, QZ-3)

/** The slots of the practice / exam action bar, in DOM order (the phone layout is one row of these) */
export type BarSlotName = 'prev' | 'main' | 'hint' | 'explain' | 'flag';

/** What the main area can hold: Check answer, Next, Finish practice, Submit (Check answer and Next are always two separate buttons) */
export type BarAct = 'check' | 'next' | 'finish' | 'submit';

export interface BarSlot {
  slot: BarSlotName;
  enabled: boolean;
}

export interface BarItem {
  act: BarAct;
  enabled: boolean;
}

export interface BarSlots {
  slots: BarSlot[];
  /** The items of the main slot, left to right: practice = [Check answer, Next | Finish practice], exam = [Next | Submit] */
  main: BarItem[];
}

export interface BarInput {
  kind: AttemptKind;
  qType: Question['kind'];
  /** Something is chosen (any hotspot item is enough): "Check answer" can be pressed */
  answered: boolean;
  /** Practice: this question is checked (locked) */
  checked: boolean;
  first: boolean;
  last: boolean;
  /** AI+ controls of a practice: none = no buttons, usable, greyed (shown, aria-disabled) */
  ai: 'none' | 'usable' | 'greyed';
}

/**
 * The slot logic of the action bar as a pure function, so its invariants run in CI: the slot list depends only on the page
 * (kind and AI state), never on the question; a practice always shows Check answer (usable once something is chosen and the
 * question is not checked yet, greyed otherwise, a single-choice question is checked by the pick) and, as a separate button
 * in the next place, Next (Finish practice on the last question); an exam has no Check answer, only Next (Submit on the last).
 */
export function barSlots(i: BarInput): BarSlots {
  const practice = i.kind === 'practice';
  const slots: BarSlot[] = [{ slot: 'prev', enabled: !i.first }, { slot: 'main', enabled: true }];
  if (practice && i.ai !== 'none') {
    const on = i.ai === 'usable';
    slots.push({ slot: 'hint', enabled: on }, { slot: 'explain', enabled: on });
  }
  slots.push({ slot: 'flag', enabled: true });

  const main: BarItem[] = [];
  if (practice) main.push({ act: 'check', enabled: i.answered && !i.checked && !checksOnPick({ kind: i.qType }) });
  main.push({ act: i.last ? (practice ? 'finish' : 'submit') : 'next', enabled: true });
  return { slots, main };
}
