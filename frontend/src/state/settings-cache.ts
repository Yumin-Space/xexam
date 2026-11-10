// Settings are stored on the server (data directory), per user. This browser copy is only a first-paint cache, so the page
// does not flash the wrong theme or language before /api/settings answers; the server value always wins once loaded.
// The cache is kept per user (storage-keys.ts). Before anyone is signed in (the sign-in page, the "choose a new password"
// page) the page uses the cache of the user who signed in last on this browser, else the browser's language.
import type { QuestionLang, Settings, Theme, UiLang } from '../types/api.ts';
import { isUiLang } from '../i18n/define.ts';
import { paths, settingsKey, storageOwner } from './storage-keys.ts';

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

/** The first of the browser's preferred languages that the interface has (en / zh / ja), else English */
export function browserUiLang(): UiLang {
  const list = typeof navigator === 'undefined' ? [] : navigator.languages?.length ? navigator.languages : [navigator.language];
  for (const tag of list) {
    const base = String(tag ?? '').toLowerCase().split('-')[0];
    if (isUiLang(base)) return base;
  }
  return 'en';
}

function read(key: string): Settings | null {
  try {
    const raw = localStorage.getItem(key);
    return raw == null ? null : normalizeSettings(JSON.parse(raw));
  } catch {
    return null;
  }
}

/** The id of the user who signed in last on this browser (null if none, or not readable) */
export function lastUserId(): number | null {
  try {
    const v = Number(localStorage.getItem(paths.lastUser));
    return Number.isSafeInteger(v) && v > 0 ? v : null;
  } catch {
    return null;
  }
}

/** The cached settings of one user, or null when this browser has none for them */
export function cachedSettingsOf(userId: number): Settings | null {
  return read(settingsKey(userId));
}

/**
 * Settings for the first paint, before the sign-in is known: the cache of the user who signed in last on this browser (or
 * the cache of the version before users existed), else the defaults with the browser's language.
 */
export function cachedSettings(): Settings {
  const last = lastUserId();
  return (last != null ? read(settingsKey(last)) : null) ?? read(paths.legacySettings) ?? { ...DEFAULT_SETTINGS, uiLang: browserUiLang() };
}

/**
 * The settings of the very first paint: [`cachedSettings`], with the language picked on the sign-in page when a sign-in just
 * reloaded the page (see [`carryLanguage`]); that note is read once and removed.
 */
export function firstPaintSettings(): Settings {
  const base = cachedSettings();
  let carried: string | null = null;
  try {
    carried = sessionStorage.getItem(paths.pendingLang);
    sessionStorage.removeItem(paths.pendingLang);
  } catch {
    /* not available: the cache decides */
  }
  return isUiLang(carried) ? { ...base, uiLang: carried } : base;
}

/** Keeps the sign-in page's language for the page load that follows (read by [`firstPaintSettings`]). */
export function carryLanguage(lang: UiLang): void {
  try {
    sessionStorage.setItem(paths.pendingLang, lang);
  } catch {
    /* the next page falls back to the cache */
  }
}

/** Caches the settings of the signed-in user (nothing when nobody is signed in) */
export function cacheSettings(s: Settings): void {
  const id = storageOwner();
  if (id == null) return;
  try {
    localStorage.setItem(settingsKey(id), JSON.stringify(s));
  } catch {
    /* only a cache */
  }
}

/** Remembers who signed in last on this browser (for the first paint of the next visit) */
export function rememberUser(userId: number): void {
  try {
    localStorage.setItem(paths.lastUser, String(userId));
    // The single-user cache of earlier versions is no longer needed once a user's own cache exists
    localStorage.removeItem(paths.legacySettings);
  } catch {
    /* only a cache */
  }
}
