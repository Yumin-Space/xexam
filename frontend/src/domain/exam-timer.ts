// Exam time limit and not-answered questions (v1.1, TL-1 ... TL-8). Pure: no DOM, no clock (callers pass the elapsed
// seconds), no imports. The same rule as the backend for what counts as an empty answer (TL-9, `is_empty_answer`).

export const LIMIT_MIN_MINUTES = 1;
export const LIMIT_MAX_MINUTES = 600;
/** Seconds left at which the 30-minute notice is shown. */
export const NOTICE_AT = 1800;
/** Bounds of a saved limit in seconds (TL-6, TL-9). */
const LIMIT_MIN_SECONDS = LIMIT_MIN_MINUTES * 60;
const LIMIT_MAX_SECONDS = LIMIT_MAX_MINUTES * 60;

export type LimitError = 'empty' | 'notInt' | 'range';
export type EndKind = 'submit' | 'timeUp';

/** Full-width digits (U+FF10 ... U+FF19) converted to ASCII; everything else untouched. */
function asciiDigits(s: string): string {
  return s.replace(/[０-９]/g, (c) => String(c.charCodeAt(0) - 0xff10));
}

/** Whole seconds, never negative or NaN. */
function wholeSeconds(n: number): number {
  return Number.isFinite(n) && n > 0 ? Math.floor(n) : 0;
}

/** The minutes typed into the time-limit field: a whole number 1 ... 600 (trimmed, full-width digits accepted). */
export function parseLimitMinutes(
  raw: string,
): { ok: true; minutes: number; seconds: number } | { ok: false; reason: LimitError } {
  const t = asciiDigits(raw.trim());
  if (t === '') return { ok: false, reason: 'empty' };
  if (!/^[0-9]+$/.test(t)) return { ok: false, reason: 'notInt' };
  const minutes = Number(t);
  if (minutes < LIMIT_MIN_MINUTES || minutes > LIMIT_MAX_MINUTES) return { ok: false, reason: 'range' };
  return { ok: true, minutes, seconds: minutes * 60 };
}

/** The Home controls at "开始考试": unticked means no limit (the text is ignored); ticked must parse. */
export function readLimitControls(
  checked: boolean,
  raw: string,
): { ok: true; limit: number | null } | { ok: false; reason: LimitError } {
  if (!checked) return { ok: true, limit: null };
  const p = parseLimitMinutes(raw);
  return p.ok ? { ok: true, limit: p.seconds } : { ok: false, reason: p.reason };
}

/** True exactly when typing into the empty, unticked field should tick the checkbox (once per edit, TL-1). */
export function autoTick(prevRaw: string, nextRaw: string, checked: boolean): boolean {
  return !checked && prevRaw.trim() === '' && /[0-9０-９]/.test(nextRaw);
}

/** Seconds left: `limit - floor(elapsed)`, never below 0. */
export function remaining(limit: number, elapsed: number): number {
  return Math.max(0, limit - wholeSeconds(elapsed));
}

/** `mm:ss` below one hour, `h:mm:ss` from one hour; a negative or fractional value is clamped / floored. */
export function countdownText(left: number): string {
  const s = wholeSeconds(left);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const two = (n: number): string => String(n).padStart(2, '0');
  return h > 0 ? `${h}:${two(m)}:${two(sec)}` : `${two(m)}:${two(sec)}`;
}

export type TimerTone = 'normal' | 'warn' | 'danger';

/** Colour of the countdown (TL-3): danger at 5 min or less (only when the limit is above 5 min) or 1 min or less; warn at 30 min or less of a longer limit. */
export function timerTone(limit: number, left: number): TimerTone {
  if ((limit > 300 && left <= 300) || left <= 60) return 'danger';
  if (limit > NOTICE_AT && left <= NOTICE_AT) return 'warn';
  return 'normal';
}

/** The 30-minute notice (TL-4): a limit above 30 minutes, not shown yet, 0 < left <= 1800. */
export function noticeDue(limit: number, left: number, warned: boolean): boolean {
  return limit > NOTICE_AT && !warned && left > 0 && left <= NOTICE_AT;
}

/**
 * Screen-reader lines at the crossing of 300 s / 60 s (TL-4): the tick that goes from above a threshold to at or below
 * it. A jump over both (a hidden tab) gives 'm1' only. Nothing at 0 left: the time-up screen takes over.
 */
export function srDue(limit: number, prevLeft: number, left: number): 'm5' | 'm1' | null {
  if (left <= 0) return null;
  if (limit > 60 && prevLeft > 60 && left <= 60) return 'm1';
  if (limit > 300 && prevLeft > 300 && left <= 300) return 'm5';
  return null;
}

/** The limit has run out, however far past it the clock is. */
export function isTimeUp(limit: number, elapsed: number): boolean {
  return remaining(limit, elapsed) === 0;
}

/** Seconds stored in a record: a timed one never exceeds its limit (TL-5). */
export function recordSeconds(limit: number | null, elapsed: number): number {
  const s = wholeSeconds(elapsed);
  return limit === null ? s : Math.min(s, limit);
}

export interface TimedFields {
  limit: number | null;
  warned: boolean;
  timeUp: boolean;
}

export const UNTIMED: TimedFields = Object.freeze({ limit: null, warned: false, timeUp: false });

/** The timed fields of a saved session (TL-6): a bad limit drops all three; warned / timeUp need a valid limit. */
export function readTimedFields(saved: unknown): TimedFields {
  if (typeof saved !== 'object' || saved === null) return { ...UNTIMED };
  const o = saved as Record<string, unknown>;
  const limit = o['limit'];
  if (
    typeof limit !== 'number' ||
    !Number.isInteger(limit) ||
    limit < LIMIT_MIN_SECONDS ||
    limit > LIMIT_MAX_SECONDS
  ) {
    return { ...UNTIMED };
  }
  return { limit, warned: o['warned'] === true, timeUp: o['timeUp'] === true };
}

/** The keys to add to a saved session: nothing for an untimed exam. */
export function writeTimedFields(f: TimedFields): { limit?: number; warned?: boolean; timeUp?: boolean } {
  return f.limit === null ? {} : { limit: f.limit, warned: f.warned, timeUp: f.timeUp };
}

/**
 * The tail of the attempt request (TL-5a, TL-9): practice sends nothing; every exam sends `end`; a timed one also its
 * limit. A 'timeUp' end without a limit becomes 'submit'.
 */
export function attemptTail(
  kind: 'exam' | 'practice',
  f: TimedFields,
  end: EndKind,
): { limit?: number; end?: EndKind } {
  if (kind === 'practice') return {};
  if (f.limit === null) return { end: 'submit' };
  return { limit: f.limit, end };
}

/** Empty answer (same rule as the backend): nothing, or an array with no non-null element. */
export function isEmptyAnswer(ans: unknown): boolean {
  if (ans === null || ans === undefined) return true;
  if (Array.isArray(ans)) return ans.every((x) => x === null || x === undefined);
  return false;
}

/** Marks the not-answered rows of an exam (not correct and an empty answer); practice rows stay as they are. */
export function markUnanswered<T extends { ans: unknown; correct: boolean }>(
  rows: T[],
  kind: 'exam' | 'practice',
): Array<T & { unanswered?: boolean }> {
  if (kind === 'practice') return rows.slice();
  return rows.map((r) => (!r.correct && isEmptyAnswer(r.ans) ? { ...r, unanswered: true } : r));
}

/** The answer an exam's submit sends: null for a not-answered question, the answer as given otherwise. */
export function examAnswer(ans: unknown, correct: boolean): unknown {
  return !correct && isEmptyAnswer(ans) ? null : ans;
}

/**
 * Which confirm dialog an exam's submit shows (TL-5a). `open` counts every unfinished question (also partly answered
 * hotspots), `empty` only the empty ones; `partial` is the number of partly answered ones to mention.
 */
export function submitConfirm(
  open: number,
  empty: number,
): { kind: 'empty' | 'partial' | 'confirm'; partial: number } {
  if (empty > 0) return { kind: 'empty', partial: Math.max(0, open - empty) };
  if (open > 0) return { kind: 'partial', partial: open };
  return { kind: 'confirm', partial: 0 };
}

/** Right / wrong / not-answered counts of a result (the score is good over all rows). */
export function resultCounts(rows: ReadonlyArray<{ correct: boolean; unanswered?: boolean }>): {
  good: number;
  wrong: number;
  open: number;
} {
  let good = 0;
  let wrong = 0;
  let open = 0;
  for (const r of rows) {
    if (r.correct) good += 1;
    else if (r.unanswered === true) open += 1;
    else wrong += 1;
  }
  return { good, wrong, open };
}

/** The review filter shown first: wrong answers, else the not-completed ones, else all. */
export function defaultFilter(c: { wrong: number; open: number }): 'wrong' | 'open' | 'all' {
  if (c.wrong > 0) return 'wrong';
  return c.open > 0 ? 'open' : 'all';
}

/** What Home's resume card shows for a saved session (TL-6): the remaining time (null = untimed) and whether time is up. */
export function resumeView(f: TimedFields, elapsed: number): { left: number | null; timeUp: boolean } {
  if (f.limit === null) return { left: null, timeUp: false };
  const left = remaining(f.limit, elapsed);
  return { left, timeUp: f.timeUp || left === 0 };
}
