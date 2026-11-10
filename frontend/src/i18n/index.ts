// UI translation: t('area.key', { n: 3 }) returns the text in the current UI language.
// Every area has its own catalog file under ./catalog (so pages can be edited independently); this file only merges them.
// Missing keys fall back to English, then to the key itself (never throws).
import type { UiLang } from '../types/api.ts';
import type { Dict } from './define.ts';
import common from './catalog/common.ts';
import home from './catalog/home.ts';
import banks from './catalog/banks.ts';
import records from './catalog/records.ts';
import settings from './catalog/settings.ts';
import quiz from './catalog/quiz.ts';
import result from './catalog/result.ts';
import about from './catalog/about.ts';

export type MsgKey =
  | keyof typeof common.en
  | keyof typeof home.en
  | keyof typeof banks.en
  | keyof typeof records.en
  | keyof typeof settings.en
  | keyof typeof quiz.en
  | keyof typeof result.en
  | keyof typeof about.en;

export type Params = Record<string, string | number>;

const ALL = [common, home, banks, records, settings, quiz, result, about];

const tables: Record<UiLang, Dict> = {
  en: Object.assign({}, ...ALL.map((c) => c.en)) as Dict,
  zh: Object.assign({}, ...ALL.map((c) => c.zh)) as Dict,
  ja: Object.assign({}, ...ALL.map((c) => c.ja)) as Dict,
};

let current: UiLang = 'en';

export function uiLang(): UiLang {
  return current;
}

/** Switches the language used by t(). The caller re-renders. */
export function setUiLang(lang: UiLang): void {
  current = lang;
}

function fill(s: string, params?: Params): string {
  if (!params) return s;
  return s.replace(/\{(\w+)\}/g, (m, k: string) => (Object.prototype.hasOwnProperty.call(params, k) ? String(params[k]) : m));
}

function lookup(key: string): string {
  return tables[current][key] ?? tables.en[key] ?? key;
}

export function t(key: MsgKey, params?: Params): string {
  return fill(lookup(key), params);
}

/** Like t(), with `{n}` set to n; English uses `<key>_one` when n === 1 and that key exists. */
export function tn(key: MsgKey, n: number, params?: Params): string {
  const one = `${key}_one`;
  const s = n === 1 && tables[current][one] != null ? (tables[current][one] as string) : lookup(key);
  return fill(s, { n, ...params });
}

/** For tests: whether a key exists in a language's table. */
export function hasKey(lang: UiLang, key: string): boolean {
  return tables[lang][key] != null;
}

/** Locale tag for Intl / the html lang attribute */
export function localeTag(lang: UiLang = current): string {
  return lang === 'zh' ? 'zh-CN' : lang;
}
