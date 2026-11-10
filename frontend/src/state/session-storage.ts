// 暂存的考试（localStorage）。读取时对内容一概不信任：坏数据只会被丢弃或修正，绝不抛错。
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
  /** 暂存的题里有几道已经不在题库了 */
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
    /* 存不了就算了 */
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

/** 把暂存的内容和现在的题库对上。对不上的部分丢掉；整场都对不上就返回 null。 */
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
      // 只留题里确实有的选项；去重；单选最多一个
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
