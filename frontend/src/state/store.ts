// Application state. The old version used one global object S; this is still one module-level object, but it is typed and can only be accessed through this module.
import type { Bank, ImportPreview, ImportRecord, ImportPlan, Info, Mode, Progress } from '../types/api.ts';
import { poolOf, type Answer } from '../domain/grading.ts';
import { isLang, type Lang } from '../domain/language.ts';
import { paths } from './storage-keys.ts';

export type View = 'loading' | 'fatal' | 'home' | 'admin' | 'quiz' | 'result';
export type { Lang };

/** An exam in progress */
export interface Session {
  qs: Bank['questions'];
  answers: Record<string, Answer>;
  flags: Set<string>;
  idx: number;
  /** Seconds already used before the last save */
  base: number;
  /** Start time (milliseconds) of this page load */
  t0: number;
  mode: Mode;
  sheets: string[];
}

export interface ResultRow {
  q: Bank['questions'][number];
  ans: Answer;
  correct: boolean;
}

export interface ResultState {
  results: ResultRow[];
  seconds: number;
  mode: Mode;
  prev: Progress['history'][number] | null;
}

export interface AdminState {
  preview: ImportPreview | null;
  busy: boolean;
  imports: ImportRecord[];
  done: (ImportPlan & { filename: string; total: number }) | null;
  error: string;
}

export interface SetupConfig {
  mode: Mode;
  selected: Set<string> | null;
  known: string[];
  count: number | null;
  shuffle: boolean;
}

export interface AppState {
  lang: Lang;
  view: View;
  info: Info;
  admin: AdminState;
  bank: Bank | null;
  progress: Progress | null;
  cfg: SetupConfig;
  session: Session | null;
  result: ResultState | null;
  rvFilter: 'wrong' | 'all';
  timer: number | null;
}

function readLang(): Lang {
  try {
    const v = localStorage.getItem(paths.lang);
    return isLang(v) ? v : 'ja';
  } catch {
    return 'ja';
  }
}

export const emptyAdmin = (): AdminState => ({ preview: null, busy: false, imports: [], done: null, error: '' });

export const S: AppState = {
  lang: readLang(),
  view: 'loading',
  info: { version: '', arch: '', canShutdown: false, bankCount: 0 },
  admin: emptyAdmin(),
  bank: null,
  progress: null,
  cfg: { mode: 'all', selected: null, known: [], count: null, shuffle: true },
  session: null,
  result: null,
  rvFilter: 'wrong',
  timer: null,
};

/** The current scope the questions are drawn from */
export function pool(): Bank['questions'] {
  if (!S.bank || !S.cfg.selected) return [];
  return poolOf(S.bank.questions, S.cfg.selected, S.cfg.mode, S.progress);
}

/** The question bank changed (after refresh or import): keep the "checked sheets" in sync - newly appeared sheets are checked by default, vanished ones are removed. */
export function syncSelection(): void {
  if (!S.bank) return;
  const names = S.bank.sheets.filter((s) => s.usable > 0).map((s) => s.name);
  const c = S.cfg;
  if (!c.selected) {
    c.selected = new Set(names);
  } else {
    for (const n of names) if (!c.known.includes(n)) c.selected.add(n);
    for (const n of [...c.selected]) if (!names.includes(n)) c.selected.delete(n);
  }
  c.known = names;
}

export function setBank(b: Bank): void {
  S.bank = b;
  syncSelection();
}
