// The sign-in page and the "choose a new password" page: one calm card in the middle of the sheet (logo, title, a short line,
// the fields, the primary button). No shell: nothing else of the app is reachable before signing in.
// Their language is the cached settings of the user who signed in last on this browser, else the browser's language; the
// small language switch under the card changes it for this page only (after signing in, the user's own settings apply).
import '../styles/auth.css';
import { ApiError } from '../api/client.ts';
import { brand } from '../components/common.ts';
import { field, passwordLengthOk } from '../components/fields.ts';
import { UI_LANGS } from '../i18n/define.ts';
import { errorText } from '../i18n/errors.ts';
import { localeTag, t, type MsgKey } from '../i18n/index.ts';
import { authEpoch, render, saveNewPassword, signIn, signOut } from '../state/actions.ts';
import { applySettings } from '../state/preferences.ts';
import { S } from '../state/store.ts';
import type { UiLang } from '../types/api.ts';
import { h, type Child } from '../utils/dom.ts';

const LANG_NATIVE: Record<UiLang, MsgKey> = {
  en: 'settings.lang.native.en',
  zh: 'settings.lang.native.zh',
  ja: 'settings.lang.native.ja',
};

// ---------------------------------------------------------------- page-local state (survives re-renders)

const signin = { username: '', password: '', shown: false, busy: false, error: null as Child | null };
/** "Choose a new password": each field remembers its own show / hide state */
const choose = { password: '', repeat: '', shownNew: false, shownRepeat: false, busy: false, error: null as Child | null };
/** The field that gets the focus after the next render (an error points at the field to fix) */
let focusNext: string | null = null;
/** authEpoch() when the forms were last drawn: a change means another person may be at this page now */
let formsEpoch = authEpoch();

/** Forgets everything typed into the "choose a new password" form whenever the person at the page may have changed. */
function forgetForOtherPerson(): void {
  if (formsEpoch === authEpoch()) return;
  formsEpoch = authEpoch();
  Object.assign(choose, { password: '', repeat: '', shownNew: false, shownRepeat: false, busy: false, error: null });
  signin.password = '';
  signin.shown = false;
}

function focusLater(root: HTMLElement, fallback: string): void {
  const wanted = focusNext;
  focusNext = null;
  queueMicrotask(() => {
    if (!root.isConnected) return;
    // An error points at its field; otherwise keep the focus where the user is, or start on the first field to fill
    const free = document.activeElement === null || document.activeElement === document.body || !root.contains(document.activeElement);
    if (!wanted && !free) return;
    const el = document.getElementById(wanted ?? fallback);
    if (!el || el === document.activeElement) return;
    el.focus({ preventScroll: true });
    if (wanted && el instanceof HTMLInputElement && el.value) el.select();
  });
}

function errorBox(error: Child | null): HTMLElement | null {
  return error ? h('div', { class: 'err auth-msg', role: 'alert' }, h('p', null, error)) : null;
}

/** The language of these pages only (not saved: nobody is signed in yet) */
function langSwitch(): HTMLElement {
  return h('div', { class: 'seg auth-langs', role: 'group', 'aria-label': t('auth.lang.label') }, UI_LANGS.map((code) =>
    h('button', {
      type: 'button', lang: localeTag(code), 'aria-pressed': String(S.settings.uiLang === code),
      onclick: () => {
        if (S.settings.uiLang === code) return;
        S.settings = { ...S.settings, uiLang: code };
        applySettings(S.settings);
        render();
      },
    }, t(LANG_NATIVE[code]))));
}

function card(title: string, lead: Child, ...body: Child[]): HTMLElement {
  return h('div', { class: 'auth' },
    h('main', { class: 'auth-card', id: 'main', 'aria-labelledby': 'auth-title' },
      h('div', { class: 'auth-brand' }, brand()),
      h('h1', { id: 'auth-title' }, title),
      h('p', { class: 'auth-lead' }, lead),
      body),
    h('div', { class: 'auth-foot' },
      langSwitch(),
      S.info.version ? h('span', { class: 'auth-ver' }, t('footer.version', { version: S.info.version })) : null));
}

// ---------------------------------------------------------------- sign in

async function submitSignIn(): Promise<void> {
  if (signin.busy) return;
  const name = signin.username.trim();
  if (!name || !signin.password) {
    signin.error = t('auth.err.missing');
    focusNext = name ? 'auth-pass' : 'auth-user';
    render();
    return;
  }
  signin.busy = true;
  signin.error = null;
  render();
  try {
    await signIn(name, signin.password);
    // Signed in: forget what was typed
    signin.password = '';
    signin.shown = false;
  } catch (e) {
    signin.error = errorText(e);
    if (e instanceof ApiError && e.status === 401) {
      signin.password = '';
      focusNext = 'auth-pass';
    }
  } finally {
    signin.busy = false;
  }
  render();
}

/** The note of the previous render: a new reason to sign in starts the form afresh */
let lastNote: typeof S.authNote | undefined;

export function renderSignIn(): HTMLElement {
  forgetForOtherPerson();
  if (S.authNote !== lastNote) {
    lastNote = S.authNote;
    // After signing out the next person starts with an empty form; after a lost session the name is filled in again
    signin.username = S.authNote === 'expired' ? S.user?.username ?? signin.username : S.authNote === 'signedOut' ? '' : signin.username;
    signin.password = '';
    signin.shown = false;
    signin.error = null;
  }
  const user = field({
    id: 'auth-user', label: t('auth.username'), value: signin.username, autocomplete: 'username', maxlength: 200,
    onInput: (v) => {
      signin.username = v;
    },
  });
  const pass = field({
    id: 'auth-pass', label: t('auth.password'), type: 'password', value: signin.password, autocomplete: 'current-password',
    shown: signin.shown,
    onInput: (v) => {
      signin.password = v;
    },
    onShow: (v) => {
      signin.shown = v;
    },
  });
  const note = S.authNote === 'signedOut' ? 'auth.note.signedOut' : S.authNote === 'expired' ? 'auth.note.expired' : null;
  // First start (an administrator still has the default password): say how to sign in instead of "ask an administrator"
  const firstRun = S.defaultAdmin && !signin.error && !note
    ? h('div', { class: 'callout info auth-msg', role: 'note' }, h('p', null, t('auth.firstRun')))
    : null;
  const form = h('form', {
    class: 'auth-form', method: 'post', action: '', novalidate: true, 'aria-busy': signin.busy ? 'true' : null,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      void submitSignIn();
    },
  },
  user.el, pass.el,
  h('button', { class: 'btn primary big auth-submit', type: 'submit', disabled: signin.busy }, t(signin.busy ? 'auth.signin.busy' : 'auth.signin.submit')));

  const root = card(t('auth.signin.title'), t('auth.signin.lead'),
    note && !signin.error ? h('div', { class: 'callout ' + (S.authNote === 'expired' ? 'warn' : 'info') + ' auth-msg', role: 'status' }, h('p', null, t(note))) : null,
    firstRun,
    errorBox(signin.error),
    form,
    S.defaultAdmin ? null : h('p', { class: 'auth-hint' }, t('auth.signin.hint')));
  document.title = `${t('auth.signin.title')} - ${t('app.name')}`;
  focusLater(root, signin.username ? 'auth-pass' : 'auth-user');
  return root;
}

// ---------------------------------------------------------------- choose a new password

async function submitNewPassword(): Promise<void> {
  if (choose.busy) return;
  if (!passwordLengthOk(choose.password)) {
    choose.error = t('auth.check.length');
    focusNext = 'auth-new';
    render();
    return;
  }
  if (choose.password !== choose.repeat) {
    choose.error = t('auth.check.mismatch');
    focusNext = 'auth-repeat';
    render();
    return;
  }
  choose.busy = true;
  choose.error = null;
  render();
  try {
    await saveNewPassword(choose.password);
    Object.assign(choose, { password: '', repeat: '', shownNew: false, shownRepeat: false });
  } catch (e) {
    choose.error = errorText(e);
    focusNext = 'auth-new';
  } finally {
    choose.busy = false;
  }
  render();
}

export function renderNewPassword(): HTMLElement {
  forgetForOtherPerson();
  const name = S.user?.username ?? '';
  const pw = field({
    id: 'auth-new', label: t('auth.password.new'), type: 'password', value: choose.password, autocomplete: 'new-password',
    hint: t('auth.password.rule'), shown: choose.shownNew,
    onInput: (v) => {
      choose.password = v;
    },
    onShow: (v) => {
      choose.shownNew = v;
    },
  });
  const again = field({
    id: 'auth-repeat', label: t('auth.password.repeat'), type: 'password', value: choose.repeat, autocomplete: 'new-password',
    shown: choose.shownRepeat,
    onInput: (v) => {
      choose.repeat = v;
    },
    onShow: (v) => {
      choose.shownRepeat = v;
    },
  });
  const form = h('form', {
    class: 'auth-form', method: 'post', action: '', novalidate: true, 'aria-busy': choose.busy ? 'true' : null,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      void submitNewPassword();
    },
  },
  // For password managers: the account the new password belongs to
  h('input', { class: 'sr-only', type: 'text', name: 'username', autocomplete: 'username', value: name, readonly: true, tabindex: '-1', 'aria-hidden': 'true' }),
  pw.el, again.el,
  h('div', { class: 'auth-acts' },
    h('button', { class: 'btn primary big auth-submit', type: 'submit', disabled: choose.busy }, t(choose.busy ? 'auth.password.busy' : 'auth.password.submit')),
    h('button', { class: 'btn quiet', type: 'button', disabled: choose.busy, onclick: () => void signOut() }, t('auth.password.signOut'))));

  const lead = t('auth.password.lead', { name: '\u0000' }).split('\u0000');
  const root = card(t('auth.password.title'),
    [lead[0] ?? '', h('b', { class: 'auth-name' }, name), lead[1] ?? ''],
    errorBox(choose.error),
    form);
  document.title = `${t('auth.password.title')} - ${t('app.name')}`;
  focusLater(root, 'auth-new');
  return root;
}
