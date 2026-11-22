// The sign-in page, the "choose a new password" page (v2.6: with a username field while the user has none) and the small
// "set a username" page (an existing user without a display name): one calm card in the middle of the sheet (logo, title, a short line,
// the fields, the primary button). No shell: nothing else of the app is reachable before signing in.
// Their language is the cached settings of the user who signed in last on this browser, else the browser's language; the
// small language switch under the card changes it for this page only (after signing in, the user's own settings apply).
// v1.0 (AU-1): the brand row is compact (the 32 px logo beside the app name and the title), the lead is short (two lines at most
// on a phone), the focused field and the submit button stay in view while the on-screen keyboard is up (keepInView), the language
// segments are 44 px on touch, and the page ends with the GitHub line under the language switch.
import { signInNameOk } from '../domain/user-name.ts';
import '../styles/auth.css';
import { ApiError, MSG_NAME_BAD, MSG_NAME_EMPTY } from '../api/client.ts';
import { githubLine } from '../components/github-line.ts';
import { field, passwordLengthOk } from '../components/fields.ts';
import { UI_LANGS } from '../i18n/define.ts';
import { cleanDisplayName, displayNameProblem } from '../domain/user-name.ts';
import { errorText } from '../i18n/errors.ts';
import { localeTag, t, type MsgKey } from '../i18n/index.ts';
import { authEpoch, render, saveFirstName, saveNewPassword, signIn, signOut } from '../state/actions.ts';
import { applySettings } from '../state/preferences.ts';
import { paths } from '../state/storage-keys.ts';
import { S } from '../state/store.ts';
import type { UiLang } from '../types/api.ts';
import { h, keepInView, type Child } from '../utils/dom.ts';

const LANG_NATIVE: Record<UiLang, MsgKey> = {
  en: 'settings.lang.native.en',
  zh: 'settings.lang.native.zh',
  ja: 'settings.lang.native.ja',
};

// ---------------------------------------------------------------- page-local state (survives re-renders)

const signin = { username: '', password: '', shown: false, busy: false, error: null as Child | null, remember: rememberChoice() };

/** The "Keep me signed in" tick of this browser (ticked when nothing was saved; private windows may refuse storage) */
function rememberChoice(): boolean {
  try {
    return window.localStorage.getItem(paths.remember) !== '0';
  } catch {
    return true;
  }
}

function saveRemember(on: boolean): void {
  try {
    window.localStorage.setItem(paths.remember, on ? '1' : '0');
  } catch {
    // not remembered: the box is ticked again next time
  }
}
/** "Choose a new password": each field remembers its own show / hide state */
const choose = { name: '', password: '', repeat: '', shownNew: false, shownRepeat: false, busy: false, error: null as Child | null };
/** "Set a username" (v2.6) */
const setName = { name: '', busy: false, error: null as Child | null };
/** The field that gets the focus after the next render (an error points at the field to fix) */
let focusNext: string | null = null;
/** authEpoch() when the forms were last drawn: a change means another person may be at this page now */
let formsEpoch = authEpoch();

/** Forgets everything typed into the "choose a new password" form whenever the person at the page may have changed. */
function forgetForOtherPerson(): void {
  if (formsEpoch === authEpoch()) return;
  formsEpoch = authEpoch();
  Object.assign(choose, { name: '', password: '', repeat: '', shownNew: false, shownRepeat: false, busy: false, error: null });
  Object.assign(setName, { name: '', busy: false, error: null });
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
      h('div', { class: 'auth-head' },
        h('img', { class: 'logo auth-logo', src: '/icon.svg', alt: '', width: '32', height: '32', 'aria-hidden': 'true' }),
        h('span', { class: 'auth-app' }, t('app.name')),
        h('h1', { id: 'auth-title' }, title)),
      h('p', { class: 'auth-lead' }, lead),
      body),
    // v1.1 (SI-1): the language switch sits right under the card; only the GitHub line is pushed to the screen bottom
    h('div', { class: 'auth-lang' }, langSwitch()),
    h('div', { class: 'auth-bottom' }, githubLine()));
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
  // v2.6 (owner): sign in with the e-mail address only, never with the display name; the one exception is the default
  // administrator `admin` of a fresh install (it has no e-mail yet)
  if (!signInNameOk(name)) {
    signin.error = t('auth.err.email');
    focusNext = 'auth-user';
    render();
    return;
  }
  signin.busy = true;
  signin.error = null;
  render();
  try {
    await signIn(name, signin.password, signin.remember);
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
    id: 'auth-user', label: t('auth.username'), value: signin.username, autocomplete: 'username', inputmode: 'email', maxlength: 200, placeholder: 'name@example.com',
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
  const note = S.authNote === 'signedOut' ? 'auth.note.signedOut' : S.authNote === 'expired' ? 'auth.expired' : null;
  // First start (an administrator still has the default password): say how to sign in instead of "ask an administrator",
  // with the account's real name (it may have been renamed). It always shows, also under an error (a mistyped password, a
  // lock): whoever got it wrong needs the instruction most. Only the "signed out" / "session ended" note gives way to an error.
  const firstRun = S.defaultAdmin
    ? h('div', { class: 'callout info auth-msg', role: 'note' }, h('p', null, t('auth.firstRun', { name: S.defaultAdmin })))
    : null;
  const rememberBox = h('input', {
    id: 'auth-remember', type: 'checkbox', checked: signin.remember, 'aria-describedby': 'auth-remember-h',
    onchange: () => {
      signin.remember = rememberBox.checked;
      saveRemember(signin.remember);
    },
  });
  const rememberRow = h('div', { class: 'auth-remember' },
    h('label', { class: 'auth-remember-l', for: 'auth-remember' }, rememberBox, h('span', null, t('auth.remember'))),
    h('p', { class: 'auth-remember-h', id: 'auth-remember-h' }, t('auth.remember.hint')));
  const submit = h('button', { class: 'btn primary big auth-submit', type: 'submit', disabled: signin.busy }, t(signin.busy ? 'auth.signin.busy' : 'auth.signin.submit'));
  keepInView(user.input, submit);
  keepInView(pass.input, submit);
  const form = h('form', {
    class: 'auth-form', method: 'post', action: '', novalidate: true, 'aria-busy': signin.busy ? 'true' : null,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      void submitSignIn();
    },
  },
  user.el, pass.el, rememberRow, submit);

  const root = card(t('auth.signin.title'), t('auth.signin.lead'),
    note && !signin.error ? h('div', { class: 'callout ' + (S.authNote === 'expired' ? 'warn' : 'info') + ' auth-msg', role: 'status' }, h('p', null, t(note))) : null,
    errorBox(signin.error),
    firstRun,
    form,
    S.defaultAdmin ? null : h('p', { class: 'auth-hint' }, t('auth.signin.hint')));
  document.title = `${t('auth.signin.title')} - ${t('app.name')}`;
  focusLater(root, signin.username ? 'auth-pass' : 'auth-user');
  return root;
}

// ---------------------------------------------------------------- choose a new password

/** The client-side text for a refused display name (the server applies the same rule) */
function nameProblemText(raw: string): string | null {
  const p = displayNameProblem(raw);
  return p === 'empty' ? t('auth.check.nameEmpty') : p === 'bad' ? t('auth.check.nameBad') : null;
}

async function submitNewPassword(): Promise<void> {
  if (choose.busy) return;
  // v2.6: a user without a username chooses it together with the password
  const needsName = S.user?.needsName === true;
  const nameProblem = needsName ? nameProblemText(choose.name) : null;
  if (nameProblem) {
    choose.error = nameProblem;
    focusNext = 'auth-name';
    render();
    return;
  }
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
    await saveNewPassword(choose.password, needsName ? cleanDisplayName(choose.name) : undefined);
    Object.assign(choose, { name: '', password: '', repeat: '', shownNew: false, shownRepeat: false });
  } catch (e) {
    choose.error = errorText(e);
    // A taken name points back at the name field
    const nameError = e instanceof ApiError && (e.status === 409 || (e.status === 400 && (e.serverMessage === MSG_NAME_BAD || e.serverMessage === MSG_NAME_EMPTY)));
    focusNext = needsName && nameError ? 'auth-name' : 'auth-new';
  } finally {
    choose.busy = false;
  }
  render();
}

export function renderNewPassword(): HTMLElement {
  forgetForOtherPerson();
  const name = S.user?.username ?? '';
  const needsName = S.user?.needsName === true;
  const nameField = needsName ? field({
    id: 'auth-name', label: t('auth.name.label'), value: choose.name, autocomplete: 'nickname', hint: t('auth.name.hint'),
    onInput: (v) => {
      choose.name = v;
    },
  }) : null;
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
  const submit = h('button', { class: 'btn primary big auth-submit', type: 'submit', disabled: choose.busy }, t(choose.busy ? 'auth.password.busy' : 'auth.password.submit'));
  if (nameField) keepInView(nameField.input, submit);
  keepInView(pw.input, submit);
  keepInView(again.input, submit);
  const form = h('form', {
    class: 'auth-form', method: 'post', action: '', novalidate: true, 'aria-busy': choose.busy ? 'true' : null,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      void submitNewPassword();
    },
  },
  // For password managers: the account the new password belongs to
  h('input', { class: 'sr-only', type: 'text', name: 'username', autocomplete: 'username', value: name, readonly: true, tabindex: '-1', 'aria-hidden': 'true' }),
  nameField?.el ?? null, pw.el, again.el,
  h('div', { class: 'auth-acts' },
    submit,
    h('button', { class: 'btn quiet', type: 'button', disabled: choose.busy, onclick: () => void signOut() }, t('auth.password.signOut'))));

  const titleKey = needsName ? 'auth.setup.title' : 'auth.password.title';
  const lead = t(needsName ? 'auth.setup.lead' : 'auth.password.lead', { name: '\u0000' }).split('\u0000');
  const root = card(t(titleKey),
    [lead[0] ?? '', h('b', { class: 'auth-name' }, name), lead[1] ?? ''],
    errorBox(choose.error),
    form);
  document.title = `${t(titleKey)} - ${t('app.name')}`;
  focusLater(root, needsName ? 'auth-name' : 'auth-new');
  return root;
}

// ---------------------------------------------------------------- set a username (v2.6)

async function submitName(): Promise<void> {
  if (setName.busy) return;
  const problem = nameProblemText(setName.name);
  if (problem) {
    setName.error = problem;
    focusNext = 'auth-name';
    render();
    return;
  }
  setName.busy = true;
  setName.error = null;
  render();
  try {
    await saveFirstName(cleanDisplayName(setName.name));
    setName.name = '';
  } catch (e) {
    setName.error = errorText(e);
    focusNext = 'auth-name';
  } finally {
    setName.busy = false;
  }
  render();
}

/** An existing user without a username (their password is fine): one field, Save, and a Sign out link */
export function renderSetName(): HTMLElement {
  forgetForOtherPerson();
  const nameField = field({
    id: 'auth-name', label: t('auth.name.label'), value: setName.name, autocomplete: 'nickname', hint: t('auth.name.hint'),
    onInput: (v) => {
      setName.name = v;
    },
  });
  const submit = h('button', { class: 'btn primary big auth-submit', type: 'submit', disabled: setName.busy }, t(setName.busy ? 'auth.name.busy' : 'auth.name.submit'));
  keepInView(nameField.input, submit);
  const form = h('form', {
    class: 'auth-form', method: 'post', action: '', novalidate: true, 'aria-busy': setName.busy ? 'true' : null,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      void submitName();
    },
  },
  nameField.el,
  h('div', { class: 'auth-acts' },
    submit,
    h('button', { class: 'btn quiet', type: 'button', disabled: setName.busy, onclick: () => void signOut() }, t('auth.password.signOut'))));
  const root = card(t('auth.name.title'), t('auth.name.lead'), errorBox(setName.error), form);
  document.title = `${t('auth.name.title')} - ${t('app.name')}`;
  focusLater(root, 'auth-name');
  return root;
}
