// Applying and saving the settings (theme, UI language, auto-advance, question language).
// The server (data directory) is the source of truth; see settings-cache.ts for the browser-side first-paint cache.
import { fetchSettings, resetSettings as apiResetSettings, saveSettings } from '../api/settings.ts';
import { localeTag, setUiLang } from '../i18n/index.ts';
import type { Settings } from '../types/api.ts';
import { cacheSettings, normalizeSettings } from './settings-cache.ts';
import { paths } from './storage-keys.ts';
import { S } from './store.ts';

const LIGHT_BAR = '#E3E8ED';
const DARK_BAR = '#10151D';

function systemDark(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia('(prefers-color-scheme: dark)').matches;
}

/** Whether the page is currently drawn dark */
export function isDark(s: Settings = S.settings): boolean {
  return s.theme === 'dark' || (s.theme === 'system' && systemDark());
}

/** Applies theme and UI language to the document (does not re-render). */
export function applySettings(s: Settings): void {
  setUiLang(s.uiLang);
  const root = document.documentElement;
  root.lang = localeTag(s.uiLang);
  if (s.theme === 'system') delete root.dataset.theme;
  else root.dataset.theme = s.theme;
  const bar = document.querySelector('meta[name="theme-color"]');
  if (bar) bar.setAttribute('content', isDark(s) ? DARK_BAR : LIGHT_BAR);
}

/** Keeps the browser bar colour in step when the system theme changes while "follow system" is selected. */
export function watchSystemTheme(): void {
  if (typeof window.matchMedia !== 'function') return;
  const mq = window.matchMedia('(prefers-color-scheme: dark)');
  const onChange = (): void => {
    if (S.settings.theme === 'system') applySettings(S.settings);
  };
  if (typeof mq.addEventListener === 'function') mq.addEventListener('change', onChange);
}

function adopt(s: Settings): void {
  S.settings = normalizeSettings(s);
  cacheSettings(S.settings);
  applySettings(S.settings);
}

export async function loadSettings(): Promise<void> {
  adopt(await fetchSettings());
}

/**
 * Saves part of the settings. Applied immediately (optimistic); on failure the previous settings come back and the error is thrown.
 * `rerender` is called after the optimistic change and again after the server answered.
 */
export async function updateSettings(patch: Partial<Settings>, rerender: () => void): Promise<void> {
  const before = S.settings;
  adopt({ ...before, ...patch });
  rerender();
  try {
    adopt(await saveSettings(patch));
  } catch (e) {
    adopt(before);
    rerender();
    throw e;
  }
  rerender();
}

/** Restores theme, UI language and auto-advance to their defaults (the question language is kept). */
export async function resetSettings(rerender: () => void): Promise<void> {
  adopt(await apiResetSettings());
  rerender();
}

/**
 * One-time migration: before v0.5 the question language lived only in this browser. If the server still has the default
 * and this browser remembers another language, move it to the server once, then forget the browser copy.
 */
export async function migrateBrowserQuestionLang(): Promise<void> {
  let local: string | null = null;
  try {
    local = localStorage.getItem(paths.lang);
  } catch {
    return;
  }
  if (local !== 'zh' && local !== 'en' && local !== 'ja') return;
  try {
    if (local !== S.settings.questionLang && S.settings.questionLang === 'ja') adopt(await saveSettings({ questionLang: local }));
    localStorage.removeItem(paths.lang);
  } catch {
    /* try again next time */
  }
}
