// Settings page: theme, interface language, auto-advance, reset to defaults.
// Every change goes through updateSettings(): applied at once (optimistic), saved on the server (Xexam's data directory),
// and rolled back with a toast when saving fails. The page is redrawn by render(); the focused control keeps the focus.
import '../styles/settings.css';
import { pageHead } from '../components/common.ts';
import { localeTag, t, type MsgKey } from '../i18n/index.ts';
import { UI_LANGS } from '../i18n/define.ts';
import { errorText } from '../i18n/errors.ts';
import { render } from '../state/actions.ts';
import { resetSettings, updateSettings } from '../state/preferences.ts';
import { DEFAULT_SETTINGS } from '../state/settings-cache.ts';
import { S } from '../state/store.ts';
import type { Settings, Theme, UiLang } from '../types/api.ts';
import { ask } from '../utils/dialog.ts';
import { h, type Child } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';

const THEMES: readonly Theme[] = ['system', 'light', 'dark'];

const THEME_NAME: Record<Theme, MsgKey> = {
  system: 'settings.theme.system',
  light: 'settings.theme.light',
  dark: 'settings.theme.dark',
};
const THEME_DESC: Record<Theme, MsgKey> = {
  system: 'settings.theme.system.desc',
  light: 'settings.theme.light.desc',
  dark: 'settings.theme.dark.desc',
};
/** Each language's own name (the same in every UI language; the cards show nothing else, see settings.css) */
const LANG_NATIVE: Record<UiLang, MsgKey> = {
  en: 'settings.lang.native.en',
  zh: 'settings.lang.native.zh',
  ja: 'settings.lang.native.ja',
};

// ---------------------------------------------------------------- page-local state

/** Saves still waiting for the server */
let pending = 0;
/** "Saved" is shown for a moment after the last save succeeded */
let savedShown = false;
let savedTimer: number | undefined;
let resetting = false;
/** The status text of the current render (updated in place, without a redraw) */
let statusEl: HTMLElement | null = null;
/** A control to focus after the next render instead of the one that had the focus */
let forceFocus: string | null = null;

const statusText = (): string => (pending > 0 ? t('common.saving') : savedShown ? t('common.saved') : '');

function showStatus(): void {
  if (statusEl) statusEl.textContent = statusText();
}

/** The data-focus key of the focused control, so it gets the focus back after the page is redrawn */
function focusedKey(): string | null {
  const a = document.activeElement;
  return a instanceof HTMLElement ? a.dataset.focus ?? null : null;
}

function restoreFocus(root: HTMLElement, key: string | null): void {
  if (!key) return;
  queueMicrotask(() => {
    if (!root.isConnected) return;
    const el = root.querySelector<HTMLElement>(`[data-focus="${key}"]`);
    if (el && !(el instanceof HTMLButtonElement && el.disabled)) el.focus({ preventScroll: true });
  });
}

const atDefaults = (s: Settings): boolean =>
  s.theme === DEFAULT_SETTINGS.theme && s.uiLang === DEFAULT_SETTINGS.uiLang && s.autoAdvance === DEFAULT_SETTINGS.autoAdvance;

// ---------------------------------------------------------------- actions

async function save(patch: Partial<Settings>): Promise<void> {
  pending += 1;
  savedShown = false;
  window.clearTimeout(savedTimer);
  let ok = false;
  try {
    await updateSettings(patch, render); // redraws at once (optimistic) and again when the server answered
    ok = true;
  } catch (e) {
    toast(t('act.settings.saveFailed', { reason: errorText(e) }), 6000);
  } finally {
    pending -= 1;
  }
  if (ok && pending === 0) {
    savedShown = true;
    savedTimer = window.setTimeout(() => {
      savedShown = false;
      showStatus();
    }, 2500);
  }
  showStatus();
}

async function reset(): Promise<void> {
  if (resetting) return;
  const yes = await ask({
    title: t('settings.reset.confirmTitle'),
    body: t('settings.reset.confirmBody', {
      theme: t(THEME_NAME[DEFAULT_SETTINGS.theme]),
      lang: t(LANG_NATIVE[DEFAULT_SETTINGS.uiLang]),
      auto: t(DEFAULT_SETTINGS.autoAdvance ? 'settings.on' : 'settings.off'),
    }),
    ok: t('settings.reset.ok'),
    cancel: t('common.cancel'),
  });
  if (!yes || resetting) return;
  resetting = true;
  render();
  let ok = false;
  try {
    await resetSettings(render);
    ok = true;
  } catch (e) {
    toast(t('settings.reset.failed', { reason: errorText(e) }), 6000);
  } finally {
    resetting = false;
  }
  // After a reset the button is disabled (everything is at its defaults): move the focus to the sentence that says so
  if (S.route.name === 'settings') forceFocus = ok ? 'set-reset-note' : 'set-reset';
  render();
  if (ok) toast(t('settings.reset.done'));
}

// ---------------------------------------------------------------- building blocks

function section(id: string, title: string, desc: string, ...body: Child[]): HTMLElement {
  return h('section', { class: 'panel set-sec', 'aria-labelledby': id },
    h('h2', { id }, title),
    h('p', { class: 'set-desc' }, desc),
    body);
}

interface CardOptions {
  focus: string;
  pressed: boolean;
  isDefault: boolean;
  name: HTMLElement;
  /** A line under the name (theme cards); language cards have none */
  desc?: string;
  /** Theme cards: the preview drawn above (or, on narrow screens, beside) the name */
  preview?: HTMLElement;
  onPick: () => void;
}

/** One selectable card (a toggle button; exactly one card of a group is pressed) */
function card(o: CardOptions): HTMLElement {
  return h('button', {
    class: 'set-card' + (o.preview ? ' themed' : ''), type: 'button', 'aria-pressed': String(o.pressed), 'data-focus': o.focus, onclick: o.onPick,
  },
  o.preview ?? null,
  h('span', { class: 'set-card-name' },
    h('span', { class: 'set-radio', 'aria-hidden': 'true' }),
    o.name,
    o.isDefault ? h('span', { class: 'set-def' }, t('settings.default')) : null),
  o.desc ? h('span', { class: 'set-card-desc' }, o.desc) : null);
}

/** A small window mock-up of one theme, drawn with the colour tokens (see settings.css) */
function swatch(kind: 'light' | 'dark'): HTMLElement {
  return h('span', { class: `set-sw ${kind}` },
    h('span', { class: 'set-sw-side' }),
    h('span', { class: 'set-sw-main' }, h('i'), h('i'), h('i', { class: 'acc' })));
}

function preview(theme: Theme): HTMLElement {
  return h('span', { class: 'set-sw-frame' + (theme === 'system' ? ' pair' : ''), 'aria-hidden': 'true' },
    theme === 'system' ? [swatch('light'), swatch('dark')] : swatch(theme));
}

// ---------------------------------------------------------------- page

export function renderSettings(): HTMLElement {
  const want = forceFocus ?? focusedKey();
  forceFocus = null;
  const s = S.settings;

  statusEl = h('span', { class: 'set-status', role: 'status' }, statusText());

  const appearance = section('set-appearance', t('settings.appearance'), t('settings.appearance.desc'),
    h('div', { class: 'set-cards', role: 'group', 'aria-label': t('settings.theme.label') }, THEMES.map((theme) => card({
      focus: `set-theme-${theme}`,
      pressed: s.theme === theme,
      isDefault: theme === DEFAULT_SETTINGS.theme,
      preview: preview(theme),
      name: h('span', null, t(THEME_NAME[theme])),
      desc: t(THEME_DESC[theme]),
      onPick: () => {
        if (S.settings.theme !== theme) void save({ theme });
      },
    }))));

  const language = section('set-language', t('settings.language'), t('settings.language.desc'),
    h('div', { class: 'set-cards set-langs', role: 'group', 'aria-label': t('settings.uiLang.label') }, UI_LANGS.map((code) => card({
      focus: `set-lang-${code}`,
      pressed: s.uiLang === code,
      isDefault: code === DEFAULT_SETTINGS.uiLang,
      name: h('span', { lang: localeTag(code) }, t(LANG_NATIVE[code])),
      onPick: () => {
        if (S.settings.uiLang !== code) void save({ uiLang: code });
      },
    }))));

  const on = s.autoAdvance;
  const answering = h('section', { class: 'panel set-sec', 'aria-labelledby': 'set-answering' },
    h('h2', { id: 'set-answering' }, t('settings.answering')),
    h('button', {
      class: 'set-switch', type: 'button', role: 'switch', 'aria-checked': String(on),
      'aria-labelledby': 'set-auto-l', 'aria-describedby': 'set-auto-d', 'data-focus': 'set-auto',
      onclick: () => void save({ autoAdvance: !S.settings.autoAdvance }),
    },
    h('span', { class: 'set-switch-text' },
      h('span', { class: 'set-switch-l', id: 'set-auto-l' }, t('settings.auto.label')),
      h('span', { class: 'set-switch-d', id: 'set-auto-d' }, t('settings.auto.desc'))),
    h('span', { class: 'set-switch-ctl' },
      h('span', { class: 'set-track', 'aria-hidden': 'true' }, h('span', { class: 'set-knob' })),
      h('span', { class: 'set-switch-state', 'aria-hidden': 'true' }, t(on ? 'settings.on' : 'settings.off')))),
    h('p', { class: 'set-note' }, t('settings.auto.note', { label: t('quiz.auto.on') })));

  const defaults = atDefaults(s);
  const resetSec = section('set-reset-h', t('settings.reset'), t('settings.reset.desc'),
    h('div', { class: 'set-reset' },
      h('button', {
        class: 'btn', type: 'button', 'data-focus': 'set-reset', disabled: defaults || resetting,
        'aria-describedby': defaults ? 'set-reset-note' : null,
        onclick: () => void reset(),
      }, t(resetting ? 'settings.reset.busy' : 'settings.reset.button')),
      defaults
        ? h('p', { class: 'set-note', id: 'set-reset-note', tabindex: '-1', 'data-focus': 'set-reset-note' }, t('settings.reset.already'))
        : null));

  const root = h('div', { class: 'page page-settings' },
    pageHead(t('settings.title'), statusEl),
    h('p', { class: 'set-lead' }, t('settings.lead')),
    appearance,
    language,
    answering,
    resetSec);
  restoreFocus(root, want);
  return root;
}
