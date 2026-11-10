// 后端 REST API 的类型。形状来自旧版的真实响应（tests/baseline/golden/），键名与旧版一致，不要改。

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
 * 中文 / 英语拆出来的结构（形状相同）：单选 / 多选带 options，热点题带 items；
 * 热点题如果每个小项末尾都用本语言写了选项（是 / 否、Yes / No……），还有 choices（个数和日语一致才有）。
 */
export interface Zh {
  stem: string;
  options?: string[];
  items?: string[];
  choices?: string[][];
}

/** 英语版拆出来的结构，和中文一样。 */
export type En = Zh;

export type Kind = 'single' | 'multi' | 'hotspot';

export interface Question {
  kind: Kind;
  tag: string;
  stem: string;
  options: Opt[];
  items: Item[];
  /** 单选 / 多选：字母；热点题：每个小项选了第几个 */
  answer: string[] | number[];
  translation: string;
  zh: Zh | null;
  /** 4 列格式才有：A 列的英语原文，以及按日语结构拆开的版本（对不上日语时 en 为空） */
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
  /** 预览里才有：被截断的"读不出来"列表还有多少条没列出 */
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

/** POST /api/upload 的返回：预览 + 一次性确认码 */
export interface ImportPreview extends ImportPlan {
  filename: string;
  token: string;
}

/** POST /api/import/confirm 的返回 */
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
