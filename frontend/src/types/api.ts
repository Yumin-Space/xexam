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

/** Response of POST /api/import/confirm */
export interface ImportConfirmed {
  result: ImportPlan;
  bank: Bank;
}

export interface AttemptRequest {
  results: { id: string; correct: boolean }[];
  sheets: string[];
  mode: Mode;
  seconds: number;
}

export interface ApiErrorBody {
  error: string;
}
