// Settings page: the account (username, role, change password), theme, interface language, auto-advance, reset to defaults.
// v1.0: "Change password" is one collapsed 44 px row that opens the three fields in place (ST-1); the theme is a segmented
// control (icon + short label) under PHONE and the preview cards otherwise (ST-2); the page lead is an (i) tip and the
// descriptions that only repeat their heading are gone (ST-3); "Saved" (check icon + text) appears next to the section that
// changed for 2 s, the heading status stays for screen readers (ST-4); the AI chat mode explains itself in its description
// line, with the sentence that free-mode requests count towards the AI request limits (ST-5).
// Every change goes through updateSettings(): applied at once (optimistic), saved on the server (Xexam's data directory),
// and rolled back with a toast when saving fails. The page is redrawn by render(); the focused control keeps the focus.
import '../styles/settings.css';
import { ApiError } from '../api/client.ts';
import { pageHead } from '../components/common.ts';
import { field, passwordLengthOk } from '../components/fields.ts';
import { aiBadge, badgeReasonText } from '../components/ai-badge.ts';
import { miniIcon } from '../components/mini-icons.ts';
import { isBankManager, roleTags } from '../components/shell.ts';
import { infoTip } from '../components/tooltip.ts';
import { ownBadgeState } from '../domain/badge-state.ts';
import { localeTag, t, uiLang, type MsgKey } from '../i18n/index.ts';
import { UI_LANGS } from '../i18n/define.ts';
import { errorText } from '../i18n/errors.ts';
import { changeOwnPassword, onRouteChange, onSignedIn, render } from '../state/actions.ts';
import { setFreeMode } from '../api/ai.ts';
import { aiFreeMode, aiModeShown, aiStatus, setAiFreeMode } from '../state/ai.ts';
import { resetSettings, updateSettings } from '../state/preferences.ts';
import { DEFAULT_SETTINGS } from '../state/settings-cache.ts';
import { S, isAdmin } from '../state/store.ts';
import type { Settings, Theme, UiLang } from '../types/api.ts';
import { ask } from '../utils/dialog.ts';
import { h, type Child } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';

const THEMES: readonly Theme[] = ['system', 'light', 'dark'];
/** The segmented control (PHONE): light, dark, then "follow the system"; the icon and the short name of each */
const THEME_SEG: ReadonlyArray<{ theme: Theme; icon: 'sun' | 'moon' | 'auto'; name: MsgKey }> = [
  { theme: 'light', icon: 'sun', name: 'settings.theme.light' },
  { theme: 'dark', icon: 'moon', name: 'settings.theme.dark' },
  { theme: 'system', icon: 'auto', name: 'settings.theme.systemShort' },
];
/** The sections that can show "Saved" next to their heading */
type SavedSection = 'appearance' | 'language' | 'answering';

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
/** "Saved" is shown for a moment after the last save succeeded, next to the section that changed (ST-4) */
let savedShown = false;
let savedSection: SavedSection | null = null;
let savedTimer: number | undefined;
/** The "Saved" slot of each section of the current render (updated in place, without a redraw) */
let sectionStatus: Partial<Record<SavedSection, HTMLElement>> = {};
/** How long "Saved" stays next to its section */
const SAVED_MS = 2000;
let resetting = false;
/** The request that switches the AI chat mode runs */
let modeBusy = false;
/** The status text of the current render (updated in place, without a redraw) */
let statusEl: HTMLElement | null = null;
/** A control to focus after the next render instead of the one that had the focus */
let forceFocus: string | null = null;

const statusText = (): string => (pending > 0 ? t('common.saving') : savedShown ? t('common.saved') : '');

function showStatus(): void {
  if (statusEl) statusEl.textContent = statusText();
  for (const [key, el] of Object.entries(sectionStatus) as Array<[SavedSection, HTMLElement]>) {
    el.replaceChildren(...(savedShown && savedSection === key ? [miniIcon('check', 'set-saved-ic'), t('common.saved')] : []));
  }
}

/** The section a changed setting belongs to */
function sectionOf(patch: Partial<Settings>): SavedSection {
  return 'theme' in patch ? 'appearance' : 'uiLang' in patch ? 'language' : 'answering';
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

/** Settings > Account > Change password (what was typed survives a redraw; it is forgotten when the page is left) */
const pw = { open: false, current: '', next: '', repeat: '', shown: new Set<string>(), busy: false, error: null as string | null, done: false };
/** The password field to focus after the next render */
let pwFocus: string | null = null;

function forgetPasswords(): void {
  Object.assign(pw, { open: false, current: '', next: '', repeat: '', shown: new Set<string>(), busy: false, error: null, done: false });
}

onRouteChange((from, to) => {
  if (from.name === 'settings' && to.name !== 'settings') forgetPasswords();
});
// Signed in again after the session ended: nothing typed before (nor its error) is kept
onSignedIn(forgetPasswords);

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
    savedSection = sectionOf(patch);
    savedTimer = window.setTimeout(() => {
      savedShown = false;
      showStatus();
    }, SAVED_MS);
  }
  showStatus();
}

/** Settings > AI chat mode: sets the administrator's free mode on the server; an error shows as a toast */
async function pickMode(free: boolean): Promise<void> {
  if (modeBusy || aiFreeMode() === free) return;
  modeBusy = true;
  forceFocus = free ? 'set-mode-free' : 'set-mode-normal';
  render();
  try {
    const r = await setFreeMode(free);
    setAiFreeMode(r.free === true);
  } catch (e) {
    toast(errorText(e), 6000);
  } finally {
    modeBusy = false;
  }
  if (S.route.name === 'settings') render();
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

async function submitPassword(): Promise<void> {
  if (pw.busy) return;
  const fail = (key: 'settings.password.missing' | 'auth.check.length' | 'auth.check.mismatch' | 'auth.check.same', focus: string): void => {
    pw.error = t(key);
    pw.done = false;
    pwFocus = focus;
    render();
  };
  if (!pw.current) return fail('settings.password.missing', 'set-pw-current');
  if (!passwordLengthOk(pw.next)) return fail('auth.check.length', 'set-pw-new');
  if (pw.next !== pw.repeat) return fail('auth.check.mismatch', 'set-pw-repeat');
  if (pw.next === pw.current) return fail('auth.check.same', 'set-pw-new');
  pw.busy = true;
  pw.error = null;
  pw.done = false;
  render();
  try {
    await changeOwnPassword(pw.current, pw.next);
    forgetPasswords();
    pw.done = true;
    pwFocus = 'set-pw-done';
  } catch (e) {
    pw.busy = false;
    pw.error = errorText(e);
    // A wrong current password: type it again
    if (e instanceof ApiError && e.status === 403) {
      pw.current = '';
      pwFocus = 'set-pw-current';
    } else {
      // Anything else (too many attempts, a network error): the submit button was redrawn, so keep keyboard and screen
      // reader users in the form, on the message that says why
      pwFocus = 'set-pw-err';
    }
  }
  if (S.route.name === 'settings') render();
}

function accountSection(): HTMLElement | null {
  const u = S.user;
  if (!u) return null;
  const input = (id: string, label: MsgKey, value: string, autocomplete: string, set: (v: string) => void, hint?: MsgKey): HTMLElement => field({
    id, label: t(label), type: 'password', value, autocomplete, shown: pw.shown.has(id), disabled: pw.busy, hint: hint ? t(hint) : undefined,
    onInput: (v) => {
      set(v);
      if (pw.done) {
        pw.done = false;
        document.getElementById('set-pw-done')?.remove();
      }
    },
    onShow: (v) => {
      if (v) pw.shown.add(id);
      else pw.shown.delete(id);
    },
  }).el;
  const ownBadge = ownBadgeState(u, aiStatus());
  return h('section', { class: 'panel set-sec set-account', 'aria-labelledby': 'set-account-h' },
    h('h2', { id: 'set-account-h' }, t('settings.account')),
    h('dl', { class: 'props set-props' },
      h('dt', null, t('settings.account.username')), h('dd', { class: 'set-username' }, u.username),
      h('dt', null, t('settings.account.role')), h('dd', { class: 'set-role' }, roleTags(u), aiBadge(ownBadge))),
    // The dark AI+ badge's reason in words too: a tap shows it as a tip on a phone, and this line needs no tap at all
    ownBadge.kind === 'off' ? h('p', { class: 'set-desc set-ai-reason', id: 'set-ai-reason' }, badgeReasonText(ownBadge.reason)) : null,
    // A bank manager: what the extra permission allows (v0.7)
    isBankManager(u) ? h('p', { class: 'set-desc set-manager' }, t('settings.account.managerNote')) : null,
    h('div', { class: 'set-pw' },
      // ST-1: one 44 px row "Change password >" that opens the fields in place
      h('h3', { id: 'set-pw-h', class: 'set-pw-h' }, h('button', {
        class: 'set-pw-toggle', type: 'button', 'aria-expanded': String(pw.open), 'aria-controls': 'set-pw-form', 'data-focus': 'set-pw-toggle',
        onclick: () => {
          pw.open = !pw.open;
          if (pw.open) pwFocus = 'set-pw-current';
          else forceFocus = 'set-pw-toggle';
          render();
        },
      }, h('span', null, t('settings.password')), miniIcon('fwd', 'set-pw-chev'))),
      pw.done ? h('div', { class: 'callout ok set-pw-done', id: 'set-pw-done', role: 'status', tabindex: '-1' }, h('p', null, t('settings.password.done'))) : null,
      h('form', {
        id: 'set-pw-form', class: 'set-pw-form', novalidate: true, hidden: !pw.open, 'aria-labelledby': 'set-pw-h', 'aria-busy': pw.busy ? 'true' : null,
        onsubmit: (e: SubmitEvent) => {
          e.preventDefault();
          void submitPassword();
        },
      },
      h('p', { class: 'set-desc' }, t('settings.password.desc')),
      // For password managers: the account the new password belongs to
      h('input', { class: 'sr-only', type: 'text', name: 'username', autocomplete: 'username', value: u.username, readonly: true, tabindex: '-1', 'aria-hidden': 'true' }),
      h('div', { class: 'set-pw-grid' },
        input('set-pw-current', 'settings.password.current', pw.current, 'current-password', (v) => {
          pw.current = v;
        }),
        input('set-pw-new', 'settings.password.new', pw.next, 'new-password', (v) => {
          pw.next = v;
        }, 'settings.password.rule'),
        input('set-pw-repeat', 'settings.password.repeat', pw.repeat, 'new-password', (v) => {
          pw.repeat = v;
        })),
      pw.error ? h('div', { class: 'err', id: 'set-pw-err', role: 'alert', tabindex: '-1' }, h('p', null, pw.error)) : null,
      h('div', { class: 'acts' },
        h('button', { class: 'btn', type: 'submit', disabled: pw.busy }, t(pw.busy ? 'settings.password.busy' : 'settings.password.submit'))))));
}

// ---------------------------------------------------------------- building blocks

/** A section heading with its "Saved" slot to the right (ST-4; `key` = the sections a setting can change) */
function sectionHead(id: string, title: string, key?: SavedSection): HTMLElement {
  let slot: HTMLElement | null = null;
  if (key) {
    slot = h('span', { class: 'set-saved', role: 'status', 'aria-live': 'polite' });
    sectionStatus[key] = slot;
  }
  return h('div', { class: 'set-sechead' }, h('h2', { id }, title), slot);
}

/** A section; `desc` null = no description line (ST-3: one that only repeats the heading goes) */
function section(id: string, title: string, desc: string | null, key: SavedSection | undefined, ...body: Child[]): HTMLElement {
  return h('section', { class: 'panel set-sec', 'aria-labelledby': id },
    sectionHead(id, title, key),
    desc ? h('p', { class: 'set-desc' }, desc) : null,
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
  const pwWant = pwFocus;
  pwFocus = null;
  const s = S.settings;

  // The heading status stays for screen readers only; the visible "Saved" is next to the section that changed (ST-4)
  statusEl = h('span', { class: 'set-status sr-only', role: 'status' }, statusText());
  sectionStatus = {};

  const pickTheme = (theme: Theme): void => {
    if (S.settings.theme !== theme) void save({ theme });
  };
  // ST-2: the segmented control (icon + short label) under PHONE, the preview cards otherwise (the stylesheet shows one)
  const themeSeg = h('div', { class: 'seg set-themeseg', role: 'group', 'aria-label': t('settings.theme.label') }, THEME_SEG.map((o) => h('button', {
    type: 'button', 'aria-pressed': String(s.theme === o.theme), 'data-focus': `set-themeseg-${o.theme}`, onclick: () => pickTheme(o.theme),
  }, miniIcon(o.icon, 'set-seg-ic'), h('span', null, t(o.name)))));

  const appearance = section('set-appearance', t('settings.appearance'), null, 'appearance',
    themeSeg,
    h('div', { class: 'set-cards set-themecards', role: 'group', 'aria-label': t('settings.theme.label') }, THEMES.map((theme) => card({
      focus: `set-theme-${theme}`,
      pressed: s.theme === theme,
      isDefault: theme === DEFAULT_SETTINGS.theme,
      preview: preview(theme),
      name: h('span', null, t(THEME_NAME[theme])),
      desc: t(THEME_DESC[theme]),
      onPick: () => pickTheme(theme),
    }))));

  const language = section('set-language', t('settings.language'), t('settings.language.desc'), 'language',
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
    sectionHead('set-answering', t('settings.answering'), 'answering'),
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

  // Administrators only: the AI chat mode (the server's state, not a setting saved with the others)
  const free = aiFreeMode();
  const modeSec = aiModeShown(isAdmin())
    ? h('section', { class: 'panel set-sec', 'aria-labelledby': 'set-mode-h' },
      sectionHead('set-mode-h', t('settings.aiMode')),
      // ST-5: its explanation is the description line, with the sentence about the AI request limits
      h('p', { class: 'set-desc' }, [t('settings.aiMode.hint'), t('settings.aiMode.caps')].join(uiLang() === 'en' ? ' ' : '')),
      h('div', { class: 'set-cards set-langs', role: 'group', 'aria-label': t('settings.aiMode'), 'aria-busy': modeBusy ? 'true' : null },
        ([false, true] as const).map((isFree) => h('button', {
          class: 'set-card', type: 'button', 'aria-pressed': String(free === isFree), 'aria-disabled': modeBusy ? 'true' : null,
          'data-focus': isFree ? 'set-mode-free' : 'set-mode-normal', onclick: () => void pickMode(isFree),
        }, h('span', { class: 'set-card-name' }, h('span', { class: 'set-radio', 'aria-hidden': 'true' }),
          h('span', null, t(isFree ? 'settings.aiMode.free' : 'settings.aiMode.normal')))))))
    : null;

  const defaults = atDefaults(s);
  const resetSec = section('set-reset-h', t('settings.reset'), null, undefined,
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
    // ST-3: the page lead is an (i) tip next to the heading status
    pageHead(t('settings.title'), infoTip('set-lead-tip', t('settings.lead.tipBtn'), t('settings.lead')).el, statusEl),
    accountSection(),
    appearance,
    language,
    answering,
    modeSec,
    resetSec);
  restoreFocus(root, want);
  showStatus();
  if (pwWant) {
    queueMicrotask(() => {
      if (root.isConnected) document.getElementById(pwWant)?.focus({ preventScroll: true });
    });
  }
  return root;
}
