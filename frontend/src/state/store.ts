// Application state: one typed module-level object. Page-local UI state (filters, open panels, a pending upload preview)
// lives in module-level variables of the page file itself; only state shared between pages lives here.
import type {
  AttemptKind, AuthUser, Bank, BankSummary, HistoryEntry, Info, Mode, Progress, Question, RecordDetail, RecordSummary, Settings,
} from '../types/api.ts';
import { poolOf, type Answer } from '../domain/grading.ts';
import { type Route } from '../router.ts';
import { firstPaintSettings } from './settings-cache.ts';
import { mayManageBanks } from './bank-access.ts';

export type Phase = 'loading' | 'fatal' | 'ready';

/**
 * A page shown instead of the app: the sign-in page, or "choose a new password" (the default or initial password must be
 * replaced before anything else works). null = the app itself.
 */
export type AuthView = 'signin' | 'password' | null;

/** Why the sign-in page is shown, when there is something to say about it */
export type AuthNote = 'signedOut' | 'expired' | null;

/** An exam or a practice in progress */
export interface Session {
  qs: Question[];
  answers: Record<string, Answer>;
  flags: Set<string>;
  idx: number;
  /** Seconds already used before the last save */
  base: number;
  /** Start time (milliseconds) of this page load */
  t0: number;
  mode: Mode;
  sheets: string[];
  /** v0.6.5: an exam (feedback after submitting) or a practice (each question checked right away) */
  kind: AttemptKind;
  /** Practice: ids of the questions already checked (locked, feedback shown). Always empty in an exam. */
  checked: Set<string>;
}

export interface ResultRow {
  q: Question;
  ans: Answer;
  correct: boolean;
}

export interface ResultState {
  results: ResultRow[];
  seconds: number;
  mode: Mode;
  prev: HistoryEntry | null;
  /** Id of the saved exam record (null if saving failed or it could not be identified) */
  recordId: number | null;
  /** Questions left out of the score because they no longer exist (their bank was deleted during the exam) */
  notCounted: number;
  /** Exam or practice: the result page names it, and "take it again" starts the same kind */
  kind: AttemptKind;
}

export interface SetupConfig {
  mode: Mode;
  /** Selected sheets, as sheetKey(bankId, sheetName); null = not initialised yet */
  selected: Set<string> | null;
  /** Sheet keys seen last time (to auto-select sheets that appear later) */
  known: string[];
  count: number | null;
  shuffle: boolean;
}

export interface AppState {
  phase: Phase;
  /** The signed-in user (null before the sign-in is known and after signing out) */
  user: AuthUser | null;
  authView: AuthView;
  authNote: AuthNote;
  /**
   * The sign-in page: the username of an enabled administrator that still has the default password (GET /api/auth/state,
   * nobody signed in), else null
   */
  defaultAdmin: string | null;
  /** Why loading failed (an ApiError or anything else; shown with errorText) */
  fatal: unknown;
  route: Route;
  /** Whether the navigation menu (dropdown under the menu button) is open */
  navOpen: boolean;
  settings: Settings;
  info: Info;
  /** All active questions of all banks */
  bank: Bank | null;
  banks: BankSummary[];
  progress: Progress | null;
  /** Exam records, newest first (null until loaded) */
  records: RecordSummary[] | null;
  recordsRetention: number;
  /** Exam detail already fetched, by record id */
  recordCache: Map<number, RecordDetail>;
  cfg: SetupConfig;
  session: Session | null;
  result: ResultState | null;
  rvFilter: 'wrong' | 'all';
  timer: number | null;
}

export const S: AppState = {
  phase: 'loading',
  user: null,
  authView: null,
  authNote: null,
  defaultAdmin: null,
  fatal: null,
  route: { name: 'home' },
  navOpen: false,
  settings: firstPaintSettings(),
  info: { version: '', arch: '', canShutdown: false, bankCount: 0 },
  bank: null,
  banks: [],
  progress: null,
  records: null,
  recordsRetention: 500,
  recordCache: new Map(),
  cfg: { mode: 'all', selected: null, known: [], count: null, shuffle: true },
  session: null,
  result: null,
  rvFilter: 'wrong',
  timer: null,
};

/** Selection key of one sheet of one bank (sheet names repeat across banks) */
export function sheetKey(bankId: number, sheet: string): string {
  return `${bankId}\u0001${sheet}`;
}

export const questionSheetKey = (q: Pick<Question, 'bank' | 'sheet'>): string => sheetKey(q.bank, q.sheet);

/** The current scope the questions are drawn from */
export function pool(): Question[] {
  if (!S.bank || !S.cfg.selected) return [];
  return poolOf(S.bank.questions, S.cfg.selected, S.cfg.mode, S.progress, questionSheetKey);
}

/** Keys of every sheet that has usable questions, in bank order */
export function usableSheetKeys(): string[] {
  return S.banks.flatMap((b) => b.sheets.filter((s) => s.usable > 0).map((s) => sheetKey(b.id, s.name)));
}

/** The banks changed (after load, import or delete): keep the selection in sync - new sheets are selected by default, vanished ones are removed. */
export function syncSelection(): void {
  const keys = usableSheetKeys();
  const c = S.cfg;
  if (!c.selected) {
    c.selected = new Set(keys);
  } else {
    for (const k of keys) if (!c.known.includes(k)) c.selected.add(k);
    for (const k of [...c.selected]) if (!keys.includes(k)) c.selected.delete(k);
  }
  c.known = keys;
}

export function setBankData(bank: Bank, banks: BankSummary[]): void {
  S.bank = bank;
  S.banks = banks;
  syncSelection();
}

export function bankById(id: number): BankSummary | undefined {
  return S.banks.find((b) => b.id === id);
}

/** Bank ids of a list of questions, in order of first appearance */
export function bankIdsOf(qs: readonly Pick<Question, 'bank'>[]): number[] {
  return [...new Set(qs.map((q) => q.bank))];
}

/** Whether the signed-in user is an administrator (manages every question bank and the users) */
export function isAdmin(): boolean {
  return S.user?.role === 'admin';
}

/**
 * Whether the signed-in user may open Question banks (v0.6.5): administrators, and plain users with the "bank manager"
 * permission (they import new banks and update or delete only their own). The server decides; this only hides links.
 */
export function canManageBanks(): boolean {
  return mayManageBanks(S.user);
}

/** Forgets everything loaded for the signed-in user (signing out). Shared data (version) is kept. */
export function clearUserData(): void {
  S.user = null;
  S.bank = null;
  S.banks = [];
  S.progress = null;
  S.records = null;
  S.recordCache.clear();
  S.cfg = { mode: 'all', selected: null, known: [], count: null, shuffle: true };
  S.session = null;
  S.result = null;
  S.rvFilter = 'wrong';
  S.navOpen = false;
}
