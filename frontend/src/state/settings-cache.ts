// Settings are stored on the server (data directory). This browser copy is only a first-paint cache, so the page does not
// flash the wrong theme or language before /api/settings answers; the server value always wins once loaded.
import type { QuestionLang, Settings, Theme } from '../types/api.ts';
import { isUiLang } from '../i18n/define.ts';
import { paths } from './storage-keys.ts';

export const DEFAULT_SETTINGS: Readonly<Settings> = Object.freeze({
  theme: 'system',
  uiLang: 'en',
  autoAdvance: false,
  questionLang: 'ja',
});

const isTheme = (v: unknown): v is Theme => v === 'system' || v === 'light' || v === 'dark';
const isQuestionLang = (v: unknown): v is QuestionLang => v === 'ja' || v === 'zh' || v === 'en';

/** Accepts anything; returns a complete, valid Settings object (invalid fields fall back to the defaults). */
export function normalizeSettings(raw: unknown): Settings {
  const o = typeof raw === 'object' && raw !== null ? (raw as Record<string, unknown>) : {};
  return {
    theme: isTheme(o.theme) ? o.theme : DEFAULT_SETTINGS.theme,
    uiLang: isUiLang(o.uiLang) ? o.uiLang : DEFAULT_SETTINGS.uiLang,
    autoAdvance: typeof o.autoAdvance === 'boolean' ? o.autoAdvance : DEFAULT_SETTINGS.autoAdvance,
    questionLang: isQuestionLang(o.questionLang) ? o.questionLang : DEFAULT_SETTINGS.questionLang,
  };
}

export function cachedSettings(): Settings {
  try {
    return normalizeSettings(JSON.parse(localStorage.getItem(paths.settings) ?? 'null'));
  } catch {
    return { ...DEFAULT_SETTINGS };
  }
}

export function cacheSettings(s: Settings): void {
  try {
    localStorage.setItem(paths.settings, JSON.stringify(s));
  } catch {
    /* only a cache */
  }
}
