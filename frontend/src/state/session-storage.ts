// The saved (in-progress) exam or practice in localStorage. Nothing read back is trusted: bad data is only dropped or corrected, never throws.
// Each user has their own saved exam (the key carries the user id, see storage-keys.ts); with nobody signed in nothing is
// read or written.
import type { AttemptKind, Bank, Mode, Question } from '../types/api.ts';
import { kindOf } from '../domain/attempt.ts';
import { readTimedFields, writeTimedFields, UNTIMED, type TimedFields } from '../domain/exam-timer.ts';
import { stateOf, type Answer } from '../domain/grading.ts';
import { paths, sessionKey, storageOwner } from './storage-keys.ts';

/** The key of the signed-in user's saved exam (null when nobody is signed in) */
function key(): string | null {
  const id = storageOwner();
  return id == null ? null : sessionKey(id);
}

export interface SavedSession {
  qs: Question[];
  answers: Record<string, Answer>;
  answered: number;
  flags: Set<string>;
  idx: number;
  elapsed: number;
  mode: Mode;
  sheets: string[];
  /** How many of the saved questions are no longer in the question bank */
  dropped: number;
  /** v0.6.5: exam or practice (a save from before v0.6.5 is an exam) */
  kind: AttemptKind;
  /** Practice: the questions already checked (only those that still have an answer); always empty for an exam */
  checked: Set<string>;
  /** v0.9: the session key (32 lower-case hex) and start time (unix seconds); a save without valid ones gets new values */
  key: string;
  startedAt: number;
  /** v1.1 (TL-6): the time limit of a timed exam and its flags (an untimed exam, a practice and a bad save read back as untimed) */
  timed: TimedFields;
}

export interface SessionToSave {
  qs: Question[];
  answers: Record<string, Answer>;
  flags: Set<string>;
  idx: number;
  elapsedSeconds: number;
  mode: Mode;
  sheets: string[];
  kind: AttemptKind;
  checked: Set<string>;
  key: string;
  startedAt: number;
  /** Absent = untimed */
  timed?: TimedFields;
}

const MODES: readonly string[] = ['all', 'weak', 'retry'];
const KEY_RE = /^[0-9a-f]{32}$/;

/** A new session key: 16 random bytes as 32 lower-case hex characters (v0.9: ties a session to its AI chat and its record) */
export function newSessionKey(): string {
  const bytes = new Uint8Array(16);
  crypto.getRandomValues(bytes);
  return Array.from(bytes, (b) => b.toString(16).padStart(2, '0')).join('');
}

export function saveSession(se: SessionToSave): void {
  const k = key();
  if (!k) return;
  try {
    localStorage.setItem(k, JSON.stringify({
      v: 1, ids: se.qs.map((q) => q.id), answers: se.answers, flags: [...se.flags],
      idx: se.idx, elapsed: Math.floor(se.elapsedSeconds), mode: se.mode, sheets: se.sheets,
      // v0.6.5 (still format v1: older versions ignore the two keys, a missing kind reads back as an exam)
      kind: se.kind, checked: se.kind === 'practice' ? [...se.checked] : [],
      // v0.9 (still format v1: older versions ignore the two keys)
      key: se.key, startedAt: se.startedAt,
      // v1.1 (still format v1: older versions ignore the keys): limit / warned / timeUp of a timed exam, nothing otherwise
      ...(se.kind === 'exam' ? writeTimedFields(se.timed ?? UNTIMED) : {}),
    }));
  } catch {
    /* If it cannot be saved, so be it */
  }
}

export function clearSaved(): void {
  const k = key();
  if (!k) return;
  try {
    localStorage.removeItem(k);
  } catch {
    /* ignore */
  }
}

/**
 * Before v0.6 there was one user and one saved exam (`xexam.session.v1`). The upgrade gives the old data to the default
 * administrator, so the old saved exam goes to the first administrator who signs in on this browser (unless they already
 * have one); it is never shown to a plain user.
 */
export function adoptLegacySession(userId: number, isAdmin: boolean): void {
  try {
    const old = localStorage.getItem(paths.legacySession);
    if (old == null || !isAdmin) return;
    if (localStorage.getItem(sessionKey(userId)) == null) localStorage.setItem(sessionKey(userId), old);
    localStorage.removeItem(paths.legacySession);
  } catch {
    /* ignore */
  }
}

const isObj = (x: unknown): x is Record<string, unknown> => typeof x === 'object' && x !== null && !Array.isArray(x);

/** Match the saved content against the current question bank. Parts that do not match are dropped; if nothing matches at all, return null. */
export function loadSaved(bank: Bank | null): SavedSession | null {
  const k = key();
  if (!bank || !k) return null;
  let d: unknown;
  try {
    d = JSON.parse(localStorage.getItem(k) ?? 'null');
  } catch {
    return null;
  }
  if (!isObj(d) || !Array.isArray(d.ids)) return null;

  const byId = new Map(bank.questions.map((q) => [q.id, q] as const));
  const qs = (d.ids as unknown[]).map((id) => (typeof id === 'string' ? byId.get(id) : undefined)).filter((q): q is Question => !!q);
  if (!qs.length) return null;

  const rawAnswers = isObj(d.answers) ? d.answers : {};
  const answers: Record<string, Answer> = {};
  for (const q of qs) {
    const a = rawAnswers[q.id];
    if (!Array.isArray(a)) continue;
    if (q.kind === 'hotspot') {
      const norm = q.items.map((it, i) => {
        const v = a[i];
        return typeof v === 'number' && Number.isInteger(v) && v >= 0 && v < it.choices.length ? v : null;
      });
      if (norm.some((x) => x != null)) answers[q.id] = norm;
    } else {
      // Keep only options that actually exist in the question; dedupe; at most one for single choice
      const have = new Set(q.options.map((o) => o.key));
      let keys = [...new Set(a.filter((k): k is string => typeof k === 'string' && have.has(k)))];
      if (q.kind === 'single') keys = keys.slice(0, 1);
      if (keys.length) answers[q.id] = keys;
    }
  }

  const keep = new Set(qs.map((q) => q.id));
  const answered = qs.filter((q) => stateOf(q, answers[q.id]) === 2).length;
  const flagsRaw = Array.isArray(d.flags) ? d.flags : [];
  const num = (x: unknown): number => (typeof x === 'number' && Number.isFinite(x) ? Math.trunc(x) : 0);
  const kind = kindOf(d);
  // A checked question must still be there and still have an answer (its feedback is drawn from it); otherwise it is open again
  const checkedRaw = kind === 'practice' && Array.isArray(d.checked) ? d.checked : [];
  const checked = new Set(checkedRaw.filter((id): id is string => {
    const cq = typeof id === 'string' && keep.has(id) ? byId.get(id) : undefined;
    return !!cq && stateOf(cq, answers[cq.id]) > 0;
  }));

  // v0.9: a saved key and start time are kept when valid; a save from v0.8 (or a damaged one) gets a new key and the time now
  const nowS = Math.floor(Date.now() / 1000);
  const keyOk = typeof d.key === 'string' && KEY_RE.test(d.key) && typeof d.startedAt === 'number' && Number.isInteger(d.startedAt)
    && d.startedAt >= 1 && d.startedAt <= nowS + 300;

  return {
    key: keyOk ? (d.key as string) : newSessionKey(),
    startedAt: keyOk ? (d.startedAt as number) : nowS,
    qs, answers, answered,
    flags: new Set(flagsRaw.filter((id): id is string => typeof id === 'string' && keep.has(id))),
    idx: Math.min(Math.max(0, num(d.idx)), qs.length - 1),
    elapsed: Math.max(0, num(d.elapsed)),
    mode: typeof d.mode === 'string' && MODES.includes(d.mode) ? (d.mode as Mode) : 'all',
    sheets: Array.isArray(d.sheets) ? d.sheets.filter((s): s is string => typeof s === 'string') : [],
    dropped: (d.ids as unknown[]).length - qs.length,
    kind,
    checked,
    timed: kind === 'exam' ? readTimedFields(d) : { ...UNTIMED },
  };
}
