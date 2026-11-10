// Types of the backend REST API. Shapes come from real responses of the old version (tests/baseline/golden/); key names match the old version, do not change them.

export interface Info {
  version: string;
  arch: string;
  canShutdown: boolean;
  bankCount: number;
}

export interface Opt {
  key: string;
  text: string;
}

export interface Item {
  text: string;
  choices: string[];
}

/**
 * Structure split out for Chinese / English (same shape): single / multiple choice carry options, hotspot questions carry items;
 * if every item of a hotspot question ends with options written in this language (Chinese yes / no, English Yes / No, ...), there are also choices (only when the count matches the Japanese).
 */
export interface Zh {
  stem: string;
  options?: string[];
  items?: string[];
  choices?: string[][];
}

/** Structure split out for the English version, same as for Chinese. */
export type En = Zh;

export type Kind = 'single' | 'multi' | 'hotspot';

export interface Question {
  kind: Kind;
  tag: string;
  stem: string;
  options: Opt[];
  items: Item[];
  /** Single / multiple choice: letters; hotspot question: which choice was picked for each item */
  answer: string[] | number[];
  translation: string;
  zh: Zh | null;
  /** Only present in the 4-column format: the original English text of column A, and a version split along the Japanese structure (en is empty when it does not line up with the Japanese) */
  english?: string;
  en?: En | null;
  explanation: string;
  mnemonic: string;
  note: string;
  id: string;
  sheet: string;
  row: number;
  /** Id of the question bank (imported workbook) the question belongs to (v0.5+) */
  bank: number;
}

export interface SheetInfo {
  name: string;
  usable: number;
  inactive: number;
  total: number;
  skipped: SkippedRow[];
}

export interface Bank {
  file: string;
  mtime: string;
  error: string;
  questions: Question[];
  sheets: SheetInfo[];
}

export interface SkippedRow {
  row: number;
  reason: string;
}

export interface QuestionStat {
  attempts: number;
  correct: number;
  wrong: number;
  streak: number;
  last?: boolean;
  at?: string;
}

export type Mode = 'all' | 'weak' | 'retry';

export interface HistoryEntry {
  at: string;
  sheets: string[];
  mode: Mode | string;
  total: number;
  correct: number;
  percent: number;
  seconds: number;
}

export interface Progress {
  version: number;
  questions: Record<string, QuestionStat>;
  history: HistoryEntry[];
  masterStreak: number;
}

export interface ImportRecord {
  id: number;
  at: string;
  filename: string;
  added: number;
  updated: number;
  removed: number;
  restored: number;
  skipped: number;
  total: number;
}

export interface SheetPlan {
  name: string;
  total: number;
  usable: number;
  skipped: SkippedRow[];
  added: number;
  updated: number;
  removed: number;
  restored: number;
  /** Preview only: how many entries of the truncated "unreadable" list are not listed */
  skippedMore?: number;
  skippedCount?: number;
}

export interface ImportPlan {
  sheets: SheetPlan[];
  added: number;
  updated: number;
  unchanged: number;
  restored: number;
  removed: number;
  skipped: number;
  usable: number;
  removed_samples: string[];
  removed_stats: number;
}

/** Response of POST /api/upload: preview + one-time confirmation code */
export interface ImportPreview extends ImportPlan {
  filename: string;
  token: string;
}

/** Response of POST /api/import/confirm. bankId is present when the upload named a target (always, from the v0.5 UI). */
export interface ImportConfirmed {
  result: ImportPlan;
  bank: Bank;
  bankId?: number;
}

/** Where an upload goes: a new bank, or an existing bank (update / replace). */
export type ImportTarget = 'new' | number;

export interface AttemptRequest {
  results: { id: string; correct: boolean; answer?: Array<string | number | null> | null }[];
  sheets: string[];
  mode: Mode;
  seconds: number;
}

export interface ApiErrorBody {
  error: string;
}

// ---------------------------------------------------------------- v0.5: question banks

export interface BankSheet {
  name: string;
  usable: number;
  inactive: number;
}

/** One element of GET /api/banks */
export interface BankSummary {
  id: number;
  /** Display name (the first imported file name without .xlsx) */
  name: string;
  /** The file name of the latest import */
  filename: string;
  /** Holds the data from before v0.5 (unscoped question ids) */
  legacy: boolean;
  createdAt: string;
  updatedAt: string;
  /** Active questions */
  total: number;
  /** Retired questions */
  inactive: number;
  sheets: BankSheet[];
  /** How many of its questions have per-question study stats */
  studied: number;
  /** How many exam records include this bank */
  records: number;
}

export interface BanksResponse {
  banks: BankSummary[];
}

/** Response of POST /api/banks/delete */
export interface BankDeleted {
  banks: BankSummary[];
  bank: Bank;
  progress: Progress;
}

// ---------------------------------------------------------------- v0.5: study records

export interface RecordBank {
  id: number;
  name: string;
  deleted: boolean;
}

/** One exam session (GET /api/records) */
export interface RecordSummary {
  id: number;
  at: string;
  banks: RecordBank[];
  sheets: string[];
  mode: Mode | string;
  total: number;
  correct: number;
  percent: number;
  seconds: number;
  /** Questions that had an answer; null for records saved before v0.5 (no per-question detail) */
  answered: number | null;
  /** Whether per-question detail exists */
  detail: boolean;
}

export interface RecordsResponse {
  records: RecordSummary[];
  retention: number;
}

/** A question as stored now, plus whether it is still active */
export type StoredQuestion = Question & { active: boolean };

export interface RecordItem {
  seq: number;
  questionId: string;
  /** What was picked: letters for single / multi, choice indexes (or null) per item for hotspot; null = unanswered */
  answer: Array<string | number | null> | null;
  correct: boolean;
  /** Null when the question no longer exists (its bank was deleted) */
  question: StoredQuestion | null;
}

/** GET /api/record?id= */
export interface RecordDetail {
  record: RecordSummary;
  items: RecordItem[];
}

/** POST /api/records/delete */
export interface RecordsDeleted extends RecordsResponse {
  progress: Progress;
}

// ---------------------------------------------------------------- v0.5: settings (stored in the data directory)

export type Theme = 'system' | 'light' | 'dark';
export type UiLang = 'en' | 'zh' | 'ja';
export type QuestionLang = 'ja' | 'zh' | 'en';

export interface Settings {
  theme: Theme;
  uiLang: UiLang;
  /** After answering, go to the next question automatically */
  autoAdvance: boolean;
  /** Language the question text is shown in (Japanese is the original) */
  questionLang: QuestionLang;
}
