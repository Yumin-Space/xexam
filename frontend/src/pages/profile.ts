// Profile page (v2.6): the account (login e-mail, role, AI+ badge), the username (display name, edited in place) and
// "Change password" (the collapsed row that moved here from Settings, unchanged). Centred narrow page like Settings.
// What was typed survives a redraw and is forgotten when the page is left or the person signs in again.
import '../styles/settings.css';
import '../styles/profile.css';
import { ApiError } from '../api/client.ts';
import { pageHead } from '../components/common.ts';
import { field, passwordLengthOk } from '../components/fields.ts';
import { aiBadge, badgeReasonText } from '../components/ai-badge.ts';
import { miniIcon } from '../components/mini-icons.ts';
import { isBankManager, roleTags } from '../components/shell.ts';
import { offList, readFeatures } from '../domain/ai-features.ts';
import { ownBadgeState } from '../domain/badge-state.ts';
import { cleanDisplayName, nameSave } from '../domain/user-name.ts';
import { t, type MsgKey } from '../i18n/index.ts';
import { errorText } from '../i18n/errors.ts';
import { changeOwnName, changeOwnPassword, onRouteChange, onSignedIn, render } from '../state/actions.ts';
import { aiStatus } from '../state/ai.ts';
import { S } from '../state/store.ts';
import { h } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';

// ---------------------------------------------------------------- page-local state

/** Profile > Change password (what was typed survives a redraw; it is forgotten when the page is left) */
const pw = { open: false, current: '', next: '', repeat: '', shown: new Set<string>(), busy: false, error: null as string | null, done: false };
/** Profile > Username: the typed draft (null = untouched, the field shows the current name) */
const nm = { draft: null as string | null, busy: false, error: null as string | null };
/** The control to focus after the next render */
let focusNext: string | null = null;

function forgetPasswords(): void {
  Object.assign(pw, { open: false, current: '', next: '', repeat: '', shown: new Set<string>(), busy: false, error: null, done: false });
}

function forgetAll(): void {
  forgetPasswords();
  Object.assign(nm, { draft: null, busy: false, error: null });
}

onRouteChange((from, to) => {
  if (from.name === 'profile' && to.name !== 'profile') forgetAll();
});
// Signed in again after the session ended: nothing typed before (nor its error) is kept
onSignedIn(forgetAll);

// ---------------------------------------------------------------- actions

async function submitName(): Promise<void> {
  if (nm.busy) return;
  const draft = nm.draft ?? S.user?.displayName ?? '';
  const verdict = nameSave(S.user?.displayName, draft);
  if (verdict !== 'ok') {
    nm.error = t(verdict === 'empty' ? 'auth.check.nameEmpty' : verdict === 'bad' ? 'auth.check.nameBad' : 'profile.name.same');
    focusNext = 'pf-name';
    render();
    return;
  }
  nm.busy = true;
  nm.error = null;
  render();
  try {
    await changeOwnName(cleanDisplayName(draft));
    nm.draft = null;
    toast(t('profile.name.saved'));
    // The save button, not the input: focusing an input right after the tap would reopen the phone keyboard
    focusNext = 'pf-name-save';
  } catch (e) {
    nm.error = errorText(e);
    focusNext = 'pf-name';
  } finally {
    nm.busy = false;
  }
  if (S.route.name === 'profile') render();
}

async function submitPassword(): Promise<void> {
  if (pw.busy) return;
  const fail = (key: 'profile.password.missing' | 'auth.check.length' | 'auth.check.mismatch' | 'auth.check.same', focus: string): void => {
    pw.error = t(key);
    pw.done = false;
    focusNext = focus;
    render();
  };
  if (!pw.current) return fail('profile.password.missing', 'pf-pw-current');
  if (!passwordLengthOk(pw.next)) return fail('auth.check.length', 'pf-pw-new');
  if (pw.next !== pw.repeat) return fail('auth.check.mismatch', 'pf-pw-repeat');
  if (pw.next === pw.current) return fail('auth.check.same', 'pf-pw-new');
  pw.busy = true;
  pw.error = null;
  pw.done = false;
  render();
  try {
    await changeOwnPassword(pw.current, pw.next);
    forgetPasswords();
    pw.done = true;
    focusNext = 'pf-pw-done';
  } catch (e) {
    pw.busy = false;
    pw.error = errorText(e);
    // A wrong current password: type it again
    if (e instanceof ApiError && e.status === 403) {
      pw.current = '';
      focusNext = 'pf-pw-current';
    } else {
      // Anything else (too many attempts, a network error): the submit button was redrawn, so keep keyboard and screen
      // reader users in the form, on the message that says why
      focusNext = 'pf-pw-err';
    }
  }
  if (S.route.name === 'profile') render();
}

// ---------------------------------------------------------------- sections

function accountSection(): HTMLElement | null {
  const u = S.user;
  if (!u) return null;
  const ownBadge = ownBadgeState(u, aiStatus());
  const ownSt = aiStatus();
  const ownOff = ownSt.state === 'ready' && u.role !== 'admin' ? offList(readFeatures(ownSt.features)) : [];
  return h('section', { class: 'panel set-sec pf-sec', 'aria-labelledby': 'pf-account-h' },
    h('h2', { id: 'pf-account-h' }, t('profile.account')),
    h('dl', { class: 'props set-props' },
      h('dt', null, t('profile.account.login')), h('dd', { class: 'set-username pf-login' }, u.username),
      h('dt', null, t('profile.account.role')), h('dd', { class: 'set-role' }, roleTags(u), aiBadge(ownBadge, { off: ownOff }))),
    // The dark AI+ badge's reason in words too: a tap shows it as a tip on a phone, and this line needs no tap at all
    ownBadge.kind === 'off' ? h('p', { class: 'set-desc set-ai-reason', id: 'pf-ai-reason' }, badgeReasonText(ownBadge.reason)) : null,
    // A bank manager: what the extra permission allows (v0.7)
    isBankManager(u) ? h('p', { class: 'set-desc set-manager' }, t('profile.account.managerNote')) : null);
}

function nameSection(): HTMLElement | null {
  const u = S.user;
  if (!u) return null;
  const draft = nm.draft ?? u.displayName ?? '';
  const f = field({
    id: 'pf-name', label: t('profile.name'), value: draft, autocomplete: 'nickname',
    hint: t('profile.name.hint'), disabled: nm.busy,
    onInput: (v) => {
      nm.draft = v;
      if (nm.error) {
        nm.error = null;
        document.getElementById('pf-name-err')?.remove();
      }
    },
  });
  return h('section', { class: 'panel set-sec pf-sec', 'aria-labelledby': 'pf-name-h' },
    h('h2', { id: 'pf-name-h' }, t('profile.name')),
    h('form', {
      class: 'pf-name-form', novalidate: true, 'aria-busy': nm.busy ? 'true' : null,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void submitName();
      },
    },
    h('div', { class: 'pf-name-row' },
      f.el,
      h('button', { id: 'pf-name-save', class: 'btn primary pf-name-save', type: 'submit', disabled: nm.busy }, t(nm.busy ? 'profile.name.busy' : 'profile.name.save'))),
    nm.error ? h('div', { class: 'err pf-name-err', id: 'pf-name-err', role: 'alert' }, h('p', null, nm.error)) : null));
}

function passwordSection(): HTMLElement | null {
  const u = S.user;
  if (!u) return null;
  const input = (id: string, label: MsgKey, value: string, autocomplete: string, set: (v: string) => void, hint?: MsgKey): HTMLElement => field({
    id, label: t(label), type: 'password', value, autocomplete, shown: pw.shown.has(id), disabled: pw.busy, hint: hint ? t(hint) : undefined,
    onInput: (v) => {
      set(v);
      if (pw.done) {
        pw.done = false;
        document.getElementById('pf-pw-done')?.remove();
      }
    },
    onShow: (v) => {
      if (v) pw.shown.add(id);
      else pw.shown.delete(id);
    },
  }).el;
  return h('section', { class: 'panel set-sec pf-sec set-pw', 'aria-labelledby': 'pf-pw-h' },
    // One 44 px row "Change password >" that opens the fields in place
    h('h2', { id: 'pf-pw-h', class: 'set-pw-h' }, h('button', {
      class: 'set-pw-toggle', type: 'button', 'aria-expanded': String(pw.open), 'aria-controls': 'pf-pw-form', id: 'pf-pw-toggle',
      onclick: () => {
        pw.open = !pw.open;
        focusNext = pw.open ? 'pf-pw-current' : 'pf-pw-toggle';
        render();
      },
    }, h('span', null, t('profile.password')), miniIcon('fwd', 'set-pw-chev'))),
    pw.done ? h('div', { class: 'callout ok set-pw-done', id: 'pf-pw-done', role: 'status', tabindex: '-1' }, h('p', null, t('profile.password.done'))) : null,
    h('form', {
      id: 'pf-pw-form', class: 'set-pw-form', novalidate: true, hidden: !pw.open, 'aria-labelledby': 'pf-pw-h', 'aria-busy': pw.busy ? 'true' : null,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void submitPassword();
      },
    },
    h('p', { class: 'set-desc' }, t('profile.password.desc')),
    // For password managers: the account the new password belongs to
    h('input', { class: 'sr-only', type: 'text', name: 'username', autocomplete: 'username', value: u.username, readonly: true, tabindex: '-1', 'aria-hidden': 'true' }),
    h('div', { class: 'set-pw-grid' },
      input('pf-pw-current', 'profile.password.current', pw.current, 'current-password', (v) => {
        pw.current = v;
      }),
      input('pf-pw-new', 'profile.password.new', pw.next, 'new-password', (v) => {
        pw.next = v;
      }, 'profile.password.rule'),
      input('pf-pw-repeat', 'profile.password.repeat', pw.repeat, 'new-password', (v) => {
        pw.repeat = v;
      })),
    pw.error ? h('div', { class: 'err', id: 'pf-pw-err', role: 'alert', tabindex: '-1' }, h('p', null, pw.error)) : null,
    h('div', { class: 'acts' },
      h('button', { class: 'btn', type: 'submit', disabled: pw.busy }, t(pw.busy ? 'profile.password.busy' : 'profile.password.submit')))));
}

// ---------------------------------------------------------------- page

export function renderProfile(): HTMLElement {
  const want = focusNext;
  focusNext = null;
  const root = h('div', { class: 'page page-profile' },
    pageHead(t('profile.title')),
    accountSection(),
    nameSection(),
    passwordSection());
  if (want) {
    queueMicrotask(() => {
      if (root.isConnected) document.getElementById(want)?.focus({ preventScroll: true });
    });
  }
  return root;
}
