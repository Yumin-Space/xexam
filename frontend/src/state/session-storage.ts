// The saved (in-progress) exam in localStorage. Nothing read back is trusted: bad data is only dropped or corrected, never throws.
import type { Bank, Mode, Question } from '../types/api.ts';
import { stateOf, type Answer } from '../domain/grading.ts';
import { paths } from './storage-keys.ts';

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
}

export interface SessionToSave {
  qs: Question[];
  answers: Record<string, Answer>;
  flags: Set<string>;
  idx: number;
  elapsedSeconds: number;
  mode: Mode;
  sheets: string[];
}

const MODES: readonly string[] = ['all', 'weak', 'retry'];

export function saveSession(se: SessionToSave): void {
  try {
    localStorage.setItem(paths.session, JSON.stringify({
      v: 1, ids: se.qs.map((q) => q.id), answers: se.answers, flags: [...se.flags],
      idx: se.idx, elapsed: Math.floor(se.elapsedSeconds), mode: se.mode, sheets: se.sheets,
    }));
  } catch {
    /* If it cannot be saved, so be it */
  }
}

export function clearSaved(): void {
  try {
    localStorage.removeItem(paths.session);
  } catch {
    /* ignore */
  }
}

const isObj = (x: unknown): x is Record<string, unknown> => typeof x === 'object' && x !== null && !Array.isArray(x);

/** Match the saved content against the current question bank. Parts that do not match are dropped; if nothing matches at all, return null. */
export function loadSaved(bank: Bank | null): SavedSession | null {
  if (!bank) return null;
  let d: unknown;
  try {
    d = JSON.parse(localStorage.getItem(paths.session) ?? 'null');
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

  return {
    qs, answers, answered,
    flags: new Set(flagsRaw.filter((id): id is string => typeof id === 'string' && keep.has(id))),
    idx: Math.min(Math.max(0, num(d.idx)), qs.length - 1),
    elapsed: Math.max(0, num(d.elapsed)),
    mode: typeof d.mode === 'string' && MODES.includes(d.mode) ? (d.mode as Mode) : 'all',
    sheets: Array.isArray(d.sheets) ? d.sheets.filter((s): s is string => typeof s === 'string') : [],
    dropped: (d.ids as unknown[]).length - qs.length,
  };
}
