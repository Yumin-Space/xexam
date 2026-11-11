// User management (#/users, administrators only; the router sends everyone else home). One compact row per user with small
// actions, an inline form to add a user, small dialogs to edit a user and to reset a password, confirmations before disabling
// and deleting. The backend enforces every rule; this page shows the blocked actions disabled with the reason (your own
// account, the last enabled administrator) and always redraws the list from the server's answer.
// Every row shows the account status as tags: Normal (enabled, not locked), Disabled, and "Locked · N min left" while the
// login throttle locks the account (both when both apply), "N tries left" when it is past round 1 without a lock, plus "Must
// choose a new password". Both throttle states offer Unlock (back to round 1 with five tries; owner, v0.6.5), and a small
// visible note under the row says which browsers the state counts (those where the user signed in before are counted apart
// from all others), until when, and what gives the tries back (domain/lock.ts lockNote; visible text rather than a tooltip, so
// it can be read on a phone and with the keyboard).
// The time left is counted down here: each lock's end is fixed on the browser's monotonic clock when the list arrives
// (performance.now() + retryAfter, so the computer clocks of server and browser need not agree), a one-second timer rewrites the
// tags while the page is open and a lock runs, and when a lock ends the list is read again (the next round, tries left).
// The list is also read again quietly when the window comes back into view or gets the focus, and every 30 seconds while the
// page is visible, so a lock that starts while the page is open shows up (never while a form or dialog is open, or a change
// runs, so nothing typed or focused is redrawn away).
import '../styles/users.css';
import { fetchBanks } from '../api/bank.ts';
import { ApiError } from '../api/client.ts';
import { createUser, deleteUser, fetchUsers, resetUserPassword, unlockUser, updateUser } from '../api/users.ts';
import { pageHead } from '../components/common.ts';
import { field, passwordLengthOk, usernameOk } from '../components/fields.ts';
import { icon, ICONS, roleName } from '../components/shell.ts';
import { lockNote, lockView } from '../domain/lock.ts';
import { secondsLeft } from '../domain/wait.ts';
import { errorText } from '../i18n/errors.ts';
import { t, tn, uiLang } from '../i18n/index.ts';
import { waitShort } from '../i18n/wait.ts';
import { onRouteChange, onSignedIn, refreshLibrary, render } from '../state/actions.ts';
import {
  accessOf, accessView, draftOf, editChange, isTicked, ownedBankNames, ownerView, sameAccess, selectAllState, toggleAll, toggleBank,
  type AccessDraft,
} from '../state/bank-access.ts';
import { S } from '../state/store.ts';
import type { BankSummary, Role, UserRow } from '../types/api.ts';
import { ask, formDialog } from '../utils/dialog.ts';
import { h, type Child } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';

const ROLES: readonly Role[] = ['user', 'admin'];
const ROLE_DESC: Record<Role, 'users.role.userDesc' | 'users.role.adminDesc'> = { user: 'users.role.userDesc', admin: 'users.role.adminDesc' };

// ---------------------------------------------------------------- page-local state

let users: UserRow[] | null = null;
let loading = false;
let loadError: unknown = null;
/** Read the list again on the next render (set whenever the page is opened) */
let wanted = true;
let seq = 0;
/** Why the last row action failed (shown above the list) */
let actionError: unknown = null;
/** The user whose row action is running */
let busyId: number | null = null;
/** A dialog or confirmation is open */
let asking = false;
/** The inline "Add user" form */
const add = {
  open: false, username: '', password: '', role: 'user' as Role, shown: false, busy: false, error: null as string | null,
  /**
   * The question banks the new user sees: nothing ticked and "Include banks imported later" off by default (owner, v0.6.5),
   * so they see none until chosen
   */
  banks: draftOf([], false),
  /** The bank-manager permission (off by default) */
  manager: false,
};
/** The element to focus after the next render */
let focusNext: string | null = null;
/** User id → when their sign-in lock ends (performance.now() milliseconds), for the users whose lock runs */
const lockEnds = new Map<number, number>();
/** User id → when the shorter lock of their other browsers ends, while both are locked (the note then changes) */
const otherEnds = new Map<number, number>();
/** The one-second timer that counts the locks down while the page is open */
let ticker: ReturnType<typeof setInterval> | null = null;
/** The quiet re-read of the list while the page is open and visible */
let poller: ReturnType<typeof setInterval> | null = null;
const POLL_MS = 30_000;

function resetAdd(): void {
  Object.assign(add, {
    open: false, username: '', password: '', role: 'user', shown: false, busy: false, error: null, banks: draftOf([], false), manager: false,
  });
}

onRouteChange((from, to) => {
  if (to.name === 'users') wanted = true;
  if (from.name === 'users' && to.name !== 'users') {
    resetAdd();
    actionError = null;
    stopTicker();
    stopPoller();
  }
});
// Signed in again (the session had ended): read the list afresh instead of showing the failure of the lost session
onSignedIn(() => {
  wanted = true;
  loadError = null;
  actionError = null;
});

const onUsersPage = (): boolean => S.route.name === 'users';

/** Fixes the end of every running lock of a list that just arrived */
function noteLocks(list: readonly UserRow[]): void {
  const now = performance.now();
  lockEnds.clear();
  otherEnds.clear();
  for (const u of list) {
    if (u.lock && u.lock.retryAfter > 0) lockEnds.set(u.id, now + u.lock.retryAfter * 1000);
    if (u.lock?.otherRetryAfter) otherEnds.set(u.id, now + u.lock.otherRetryAfter * 1000);
  }
}

/** Seconds left of `u`'s lock (0 = not locked) */
function lockLeft(u: UserRow): number {
  const end = lockEnds.get(u.id);
  return end == null ? 0 : secondsLeft(end, performance.now());
}

function stopTicker(): void {
  if (ticker != null) clearInterval(ticker);
  ticker = null;
}

/** Rewrites the "Locked · N min left" tags in place (no redraw, so focus and open forms stay); reads the list again when a lock ends. */
function tick(): void {
  if (!onUsersPage() || lockEnds.size === 0) {
    stopTicker();
    return;
  }
  const now = performance.now();
  let ended = false;
  for (const [id, end] of lockEnds) {
    const left = secondsLeft(end, now);
    if (left <= 0) {
      lockEnds.delete(id);
      ended = true;
      continue;
    }
    const tag = document.getElementById(`us-lock-${id}`);
    const text = t('users.status.locked', { wait: waitShort(left) });
    if (tag && tag.textContent !== text) tag.textContent = text;
  }
  // The other browsers' shorter lock ended: the note no longer says every browser is locked (read the list again)
  for (const [id, end] of otherEnds) {
    if (secondsLeft(end, now) <= 0) {
      otherEnds.delete(id);
      ended = true;
    }
  }
  if (ended) {
    if (lockEnds.size === 0) stopTicker();
    // The next round (tries left) comes from the server; until it arrives the row shows the lock has ended
    if (!loading && busyId == null && !asking) void load();
    else render();
  }
}

function startTicker(): void {
  if (ticker == null && lockEnds.size > 0) ticker = setInterval(tick, 1000);
}

/**
 * Reads the list again without disturbing the administrator: only on this page, with the window visible, the list loaded,
 * and no form, dialog or change open (a redraw would lose what is being typed or where the focus is in it).
 */
function quietRefresh(): void {
  if (!onUsersPage() || S.authView || document.visibilityState !== 'visible') return;
  if (!users || loading || busyId != null || asking || add.open) return;
  void load();
}

function stopPoller(): void {
  if (poller != null) clearInterval(poller);
  poller = null;
}

function startPoller(): void {
  if (poller == null) poller = setInterval(quietRefresh, POLL_MS);
}

// Back from another window or tab: someone may have been locked out meanwhile (the admin is often told on the phone)
document.addEventListener('visibilitychange', quietRefresh);
window.addEventListener('focus', quietRefresh);

async function load(): Promise<void> {
  wanted = false;
  loading = true;
  const mine = ++seq;
  try {
    const list = await fetchUsers();
    if (mine !== seq) return;
    users = list;
    noteLocks(list);
    loadError = null;
  } catch (e) {
    if (mine !== seq) return;
    loadError = e;
  } finally {
    if (mine === seq) loading = false;
  }
  if (onUsersPage()) render();
}

/** The list from a change's answer (newer than any list still being read) */
function adopt(list: UserRow[]): void {
  seq += 1;
  loading = false;
  users = list;
  noteLocks(list);
  loadError = null;
  // The signed-in administrator may have renamed themselves
  const me = S.user && list.find((u) => u.id === S.user?.id);
  if (me && S.user && me.username !== S.user.username) S.user = { ...S.user, username: me.username };
}

/** A refused change about a user that no longer exists: show the real list */
function refreshAfter(e: unknown): void {
  if (e instanceof ApiError && e.status === 404) void load();
}

function focusAfter(root: HTMLElement): void {
  const id = focusNext;
  focusNext = null;
  if (!id) return;
  queueMicrotask(() => {
    if (root.isConnected) document.getElementById(id)?.focus({ preventScroll: true });
  });
}

const fmtAt = (at: string): string => at.slice(0, 16);

// ---------------------------------------------------------------- the rules the backend enforces, shown in advance

function enabledAdmins(list: readonly UserRow[]): number {
  return list.filter((u) => u.role === 'admin' && !u.disabled).length;
}

interface Limits {
  self: boolean;
  /** The last enabled administrator: cannot be disabled, deleted or made a user */
  lastAdmin: boolean;
  why: string | null;
}

function limitsOf(u: UserRow, list: readonly UserRow[]): Limits {
  const self = u.id === S.user?.id;
  const lastAdmin = u.role === 'admin' && !u.disabled && enabledAdmins(list) <= 1;
  return { self, lastAdmin, why: self ? t('users.why.self') : lastAdmin ? t('users.why.lastAdmin') : null };
}

// ---------------------------------------------------------------- role choice (radio semantics, segmented look)

function roleChoice(name: string, value: Role, disabled: boolean, onPick: (r: Role) => void, describedBy?: string): HTMLElement {
  const hint = h('p', { class: 'fld-hint', id: `${name}-hint` }, t(ROLE_DESC[value]));
  return h('fieldset', { class: 'fld us-rolefld', disabled, 'aria-describedby': [`${name}-hint`, describedBy].filter(Boolean).join(' ') },
    h('legend', { class: 'fld-l' }, t('users.field.role')),
    h('div', { class: 'us-seg' }, ROLES.map((r) => h('label', { class: 'us-seg-o' },
      h('input', {
        type: 'radio', name, value: r, checked: r === value,
        onchange: () => {
          hint.textContent = t(ROLE_DESC[r]);
          onPick(r);
        },
      }),
      h('span', null, roleName(r))))),
    hint);
}

// ---------------------------------------------------------------- question banks and the bank-manager permission (v0.6.5)

interface ChecklistOptions {
  /** Prefix of the element ids */
  id: string;
  banks: readonly BankSummary[];
  get: () => AccessDraft;
  set: (d: AccessDraft) => void;
  disabled: boolean;
  /** A short line under the legend */
  hint?: string;
  /** A short line under a bank's name (who uploaded it), or null */
  noteOf?: (b: BankSummary) => string | null;
}

/**
 * The question-bank choice of a plain user (v0.7, as the owner drew it): one line above the list holds "Include banks
 * imported later" and, at its right end, "Select all", which ticks or unticks every listed bank and shows checked, unchecked
 * or indeterminate from the list; then one box per bank. The two settings are independent: "Include banks imported later"
 * adds only the banks imported from now on, and unticking a bank removes only that bank. Clicks change the boxes in place
 * (no redraw), so the keyboard focus stays where it is.
 */
function bankChecklist(o: ChecklistOptions): { el: HTMLFieldSetElement; setDisabled: (off: boolean) => void } {
  let off = o.disabled;
  const boxes: Array<[number, HTMLInputElement]> = [];
  const listId = `${o.id}-list`;
  const later = h('input', {
    type: 'checkbox', id: `${o.id}-later`,
    onchange: () => o.set({ ...o.get(), later: later.checked }),
  });
  // "Select all" only when there is something to select
  const all = o.banks.length
    ? h('input', {
      type: 'checkbox', id: `${o.id}-all`, 'aria-controls': listId,
      onchange: () => {
        o.set(toggleAll(o.get(), o.banks));
        sync();
      },
    })
    : null;
  const sync = (): void => {
    const d = o.get();
    later.checked = d.later;
    later.disabled = off;
    for (const [id, box] of boxes) {
      box.checked = isTicked(d, id);
      box.disabled = off;
    }
    if (all) {
      const state = selectAllState(d, o.banks);
      all.checked = state === 'all';
      all.indeterminate = state === 'some';
      all.disabled = off;
    }
  };
  const list = o.banks.length
    ? h('ul', { class: 'us-banklist', id: listId }, o.banks.map((b) => {
      const box = h('input', {
        type: 'checkbox', id: `${o.id}-b${b.id}`, value: String(b.id),
        onchange: () => {
          o.set(toggleBank(o.get(), b.id, box.checked));
          sync();
        },
      });
      boxes.push([b.id, box]);
      const note = o.noteOf?.(b) ?? null;
      return h('li', null, h('label', { class: 'us-bank', for: box.id }, box,
        h('span', { class: 'us-bank-t' }, h('span', { class: 'us-bank-n' }, b.name), note ? h('span', { class: 'us-bank-h' }, note) : null)));
    }))
    : h('p', { class: 'us-banks-empty' }, t('users.banks.noneYet'));
  const hintId = o.hint ? `${o.id}-hint` : null;
  const el = h('fieldset', { class: 'fld us-banksfld', 'aria-describedby': hintId },
    h('legend', { class: 'fld-l' }, t('users.field.banks')),
    o.hint ? h('p', { class: 'fld-hint', id: hintId }, o.hint) : null,
    h('div', { class: 'us-bankhead' },
      h('label', { class: 'us-bankopt us-later', for: later.id }, later, h('span', null, t('users.banks.later'))),
      all ? h('label', { class: 'us-bankopt us-selall', for: all.id }, all, h('span', null, t('users.banks.selectAll'))) : null),
    list);
  sync();
  return {
    el,
    setDisabled: (v: boolean) => {
      off = v;
      sync();
    },
  };
}

/** Who uploaded a bank, as the checklist of `u` says it: their own upload (a bank manager sees it anyway), or the owner's name */
function bankNoteFor(u: Pick<UserRow, 'id' | 'canManageBanks' | 'role'> | null): (b: BankSummary) => string | null {
  return (b) => {
    if (u && b.ownerId === u.id && u.role === 'user' && u.canManageBanks) return t('users.banks.theirOwn');
    const o = ownerView(b, S.user?.id);
    return o.kind === 'you' ? t('users.banks.uploadedByYou') : o.kind === 'other' ? t('users.banks.uploadedBy', { name: o.name }) : null;
  };
}

/** The bank manager checkbox; administrators always manage every bank, so for them it shows ticked and cannot be changed */
function managerBox(id: string, value: boolean, role: Role, disabled: boolean, onChange: (v: boolean) => void): {
  el: HTMLElement; setRole: (r: Role) => void;
} {
  const box = h('input', { type: 'checkbox', id, onchange: () => onChange(box.checked) });
  const hint = h('span', { class: 'dlg-check-h', id: `${id}-hint` });
  box.setAttribute('aria-describedby', hint.id);
  const setRole = (r: Role): void => {
    // The stored flag stays as it was chosen; it matters again only if the administrator is made a plain user later
    box.checked = r === 'admin' ? true : value;
    box.disabled = disabled || r === 'admin';
    hint.textContent = t(r === 'admin' ? 'users.field.managerAdmin' : 'users.field.managerHint');
  };
  box.addEventListener('change', () => {
    value = box.checked;
  });
  setRole(role);
  return {
    el: h('label', { class: 'dlg-check us-check us-manager', for: id }, box,
      h('span', null, h('span', { class: 'dlg-check-l' }, t('users.field.manager')), hint)),
    setRole,
  };
}

// ---------------------------------------------------------------- add a user

async function submitAdd(): Promise<void> {
  if (add.busy) return;
  const name = add.username.trim();
  if (!usernameOk(name)) {
    add.error = t('users.check.username');
    focusNext = 'us-add-name';
    render();
    return;
  }
  if (!passwordLengthOk(add.password)) {
    add.error = t('auth.check.length');
    focusNext = 'us-add-pass';
    render();
    return;
  }
  add.busy = true;
  add.error = null;
  render();
  try {
    const access = accessOf(add.banks, S.banks);
    adopt(await createUser(name, add.password, add.role, access.banks, access.newBanks, add.manager));
    toast(t('users.created', { name }));
    resetAdd();
    focusNext = 'us-add-btn';
  } catch (e) {
    add.error = errorText(e);
    add.busy = false;
    focusNext = e instanceof ApiError && e.status === 409 ? 'us-add-name' : null;
    // A chosen bank was deleted meanwhile: show the banks as they are now (the boxes of the others stay ticked)
    if (e instanceof ApiError && e.status === 400) refreshLibrary().then(() => onUsersPage() && render(), () => {});
  }
  if (onUsersPage()) render();
}

function addForm(): HTMLElement {
  const name = field({
    id: 'us-add-name', label: t('users.field.username'), value: add.username, hint: t('users.field.usernameHint'), maxlength: 40,
    autocomplete: 'off', disabled: add.busy,
    onInput: (v) => {
      add.username = v;
    },
  });
  const pass = field({
    id: 'us-add-pass', label: t('users.field.password'), type: 'password', value: add.password, autocomplete: 'new-password',
    hint: t('users.field.passwordHint'), shown: add.shown, disabled: add.busy,
    onInput: (v) => {
      add.password = v;
    },
    onShow: (v) => {
      add.shown = v;
    },
  });
  // The bank choice and the permission only matter for plain users: for an administrator they are shown as "always"
  const banks = bankChecklist({
    id: 'us-add-banks', banks: S.banks, disabled: add.busy || add.role === 'admin', hint: t('users.field.banksHint'),
    get: () => add.banks,
    set: (d) => {
      add.banks = d;
    },
    noteOf: bankNoteFor(null),
  });
  const adminNote = h('p', { class: 'us-why us-adminnote', hidden: add.role !== 'admin' }, t('users.banks.admin'));
  const manager = managerBox('us-add-manager', add.manager, add.role, add.busy, (v) => {
    add.manager = v;
  });
  return h('form', {
    class: 'us-add', id: 'us-add', novalidate: true, 'aria-labelledby': 'us-add-h', 'aria-busy': add.busy ? 'true' : null,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      void submitAdd();
    },
  },
  h('h3', { id: 'us-add-h' }, t('users.add.title')),
  h('div', { class: 'us-add-grid' }, name.el, pass.el, roleChoice('us-add-role', add.role, add.busy, (r) => {
    add.role = r;
    banks.setDisabled(add.busy || r === 'admin');
    adminNote.hidden = r !== 'admin';
    manager.setRole(r);
  })),
  h('div', { class: 'us-add-access' }, banks.el, h('div', { class: 'us-add-side' }, manager.el, adminNote)),
  h('p', { class: 'us-add-note' }, t('users.add.note')),
  add.error ? h('div', { class: 'err', role: 'alert' }, h('p', null, add.error)) : null,
  h('div', { class: 'acts' },
    h('button', { class: 'btn primary', type: 'submit', disabled: add.busy }, t(add.busy ? 'users.creating' : 'users.create')),
    h('button', {
      class: 'btn quiet', type: 'button', disabled: add.busy,
      onclick: () => {
        resetAdd();
        focusNext = 'us-add-btn';
        render();
      },
    }, t('common.cancel'))));
}

// ---------------------------------------------------------------- row actions

/**
 * Where the keyboard focus goes after a change of `id`'s row: the same control when it still exists and can be used (the
 * toggle now says Enable / Disable), else the row's name; when the row is gone (deleted), the name of the row that took its
 * place, else the one before it, else the list heading.
 */
function focusAfterChange(id: number, before: readonly UserRow[] | null, control: string): string {
  const list = users ?? [];
  if (list.some((u) => u.id === id)) {
    // The control may be gone (Unlock disappears with the lock) or disabled now
    const el = document.getElementById(control);
    return el instanceof HTMLButtonElement && !el.disabled ? control : `us-name-${id}`;
  }
  const at = before?.findIndex((u) => u.id === id) ?? -1;
  const next = at >= 0 ? before?.slice(at + 1).find((u) => list.some((x) => x.id === u.id)) ?? null : null;
  const prev = at > 0 ? before?.slice(0, at).reverse().find((u) => list.some((x) => x.id === u.id)) ?? null : null;
  const near = next ?? prev;
  return near ? `us-name-${near.id}` : 'us-list-h';
}

/** Runs one change of a user's row: the row shows it is busy, the answer replaces the list, a failure is shown above it. */
async function change(u: UserRow, run: () => Promise<UserRow[]>, done: string, control: string): Promise<void> {
  if (busyId != null) return;
  const before = users;
  busyId = u.id;
  actionError = null;
  render();
  try {
    adopt(await run());
    toast(done);
  } catch (e) {
    actionError = e;
    refreshAfter(e);
  }
  busyId = null;
  // Decided after the redraw, when the buttons show their new state
  focusNext = null;
  if (!onUsersPage()) return;
  render();
  const target = focusAfterChange(u.id, before, control);
  document.getElementById(target)?.focus({ preventScroll: true });
}

async function guarded(fn: () => Promise<void>): Promise<void> {
  if (asking || busyId != null) return;
  asking = true;
  try {
    await fn();
  } finally {
    asking = false;
  }
}

function editUser(u: UserRow, lim: Limits): Promise<void> {
  return guarded(async () => {
    const draft = { username: u.username, role: u.role, enabled: !u.disabled, canManageBanks: u.canManageBanks };
    const locked = lim.self || lim.lastAdmin;
    // The bank-manager permission (v0.6.5): changing it does not sign the user out; it applies at their next request
    const manager = managerBox('us-edit-manager', u.canManageBanks, u.role, false, (v) => {
      draft.canManageBanks = v;
    });
    const name = field({
      id: 'us-edit-name', label: t('users.field.username'), value: u.username, hint: t('users.field.usernameHint'), maxlength: 40,
      onInput: (v) => {
        draft.username = v;
      },
    });
    const enabled = h('input', {
      type: 'checkbox', id: 'us-edit-enabled', checked: draft.enabled, disabled: locked,
      onchange: () => {
        draft.enabled = enabled.checked;
      },
    });
    const content: Child = [
      name.el,
      roleChoice('us-edit-role', u.role, locked, (r) => {
        draft.role = r;
        manager.setRole(r);
      }, locked ? 'us-edit-why' : undefined),
      h('label', { class: 'dlg-check us-check us-enabled' }, enabled,
        h('span', null, h('span', { class: 'dlg-check-l' }, t('users.field.enabled')), h('span', { class: 'dlg-check-h' }, t('users.field.enabledHint')))),
      manager.el,
      locked ? h('p', { class: 'us-why', id: 'us-edit-why' }, lim.why) : h('p', { class: 'us-dlg-note' }, t('users.edit.signOutNote')),
    ];
    let saved: string | null = null;
    const ok = await formDialog({
      title: t('users.edit.title', { name: u.username }),
      content,
      ok: t('users.edit.save'),
      busy: t('users.edit.saving'),
      submit: async () => {
        const next = draft.username.trim();
        if (!usernameOk(next)) return t('users.check.username');
        const body = editChange(u, { ...draft, username: next });
        if (!body) {
          saved = t('users.edit.nothing');
          return null;
        }
        try {
          adopt(await updateUser(body));
          saved = t('users.edit.done', { name: next });
          return null;
        } catch (e) {
          refreshAfter(e);
          return errorText(e);
        }
      },
    });
    if (ok && saved) toast(saved);
    focusNext = `us-edit-${u.id}`;
    if (onUsersPage()) render();
  });
}

function resetPassword(u: UserRow): Promise<void> {
  return guarded(async () => {
    const draft = { password: '' };
    const pass = field({
      id: 'us-reset-pass', label: t('users.reset.field'), type: 'password', autocomplete: 'new-password', hint: t('users.field.passwordHint'),
      onInput: (v) => {
        draft.password = v;
      },
    });
    const ok = await formDialog({
      title: t('users.reset.title', { name: u.username }),
      body: t('users.reset.body', { name: u.username }),
      content: pass.el,
      ok: t('users.reset.ok'),
      busy: t('users.reset.busy'),
      submit: async () => {
        if (!passwordLengthOk(draft.password)) return t('auth.check.length');
        try {
          adopt(await resetUserPassword(u.id, draft.password));
          return null;
        } catch (e) {
          refreshAfter(e);
          return errorText(e);
        }
      },
    });
    if (ok) toast(t('users.reset.done', { name: u.username }));
    focusNext = `us-reset-${u.id}`;
    if (onUsersPage()) render();
  });
}

/**
 * The current bank list for a dialog: read afresh (an administrator may have imported or deleted a bank in another tab), or
 * the list in memory when that fails. Only the summaries are read, not the questions.
 */
async function currentBanks(): Promise<BankSummary[]> {
  try {
    return await fetchBanks();
  } catch {
    return S.banks;
  }
}

/** "Question banks" of a plain user's row: the checklist dialog (the chosen banks, and whether later ones are included) */
function chooseBanks(u: UserRow): Promise<void> {
  return guarded(async () => {
    const banks = await currentBanks();
    if (!onUsersPage()) return;
    let draft = draftOf(u.banks, u.newBanks);
    const list = bankChecklist({
      id: 'us-bdlg', banks, disabled: false,
      get: () => draft,
      set: (d) => {
        draft = d;
      },
      noteOf: bankNoteFor(u),
    });
    let saved: string | null = null;
    const ok = await formDialog({
      title: t('users.banks.title', { name: u.username }),
      body: t('users.banks.body', { name: u.username }),
      content: list.el,
      ok: t('users.edit.save'),
      busy: t('users.edit.saving'),
      submit: async () => {
        const next = accessOf(draft, banks);
        if (sameAccess(next, { banks: u.banks, newBanks: u.newBanks })) {
          saved = t('users.edit.nothing');
          return null;
        }
        try {
          adopt(await updateUser({ id: u.id, banks: next.banks, newBanks: next.newBanks }));
          saved = t('users.banks.done', { name: u.username });
          return null;
        } catch (e) {
          refreshAfter(e);
          // A ticked bank was deleted meanwhile (400): the page's bank names follow the server again
          if (e instanceof ApiError && e.status === 400) void refreshLibrary().catch(() => {});
          return errorText(e);
        }
      },
    });
    if (ok && saved) toast(saved);
    focusNext = `us-banks-${u.id}`;
    if (onUsersPage()) render();
  });
}

function toggleDisabled(u: UserRow): Promise<void> {
  return guarded(async () => {
    if (!u.disabled && !(await ask({
      title: t('users.disable.title', { name: u.username }), body: t('users.disable.body', { name: u.username }),
      ok: t('users.disable.ok'), danger: true,
    }))) return;
    await change(u, () => updateUser({ id: u.id, disabled: !u.disabled }), t(u.disabled ? 'users.enabled' : 'users.disabled', { name: u.username }),
      `us-toggle-${u.id}`);
  });
}

function unlock(u: UserRow, locked: boolean): Promise<void> {
  // No confirmation: unlocking only gives the user their five tries back (a running lock ends, or the next one moves away)
  return change(u, () => unlockUser(u.id), t(locked ? 'users.unlocked' : 'users.triesReset', { name: u.username }), `us-unlock-${u.id}`);
}

function removeUser(u: UserRow): Promise<void> {
  return guarded(async () => {
    // The banks the user imported are not deleted: the server gives them to the administrator who deletes the user
    const owned = ownedBankNames(u, S.banks);
    // (v0.7) Either "the question banks are not affected" in the same paragraph, or, for a user who imported banks, a second
    // paragraph saying they move to you: never both, they would contradict each other
    // (v0.7) Exams and practice sessions are both study records: the count covers both
    const body = t('users.delete.body', { n: u.records });
    const text = u.ownedBanks.length
      ? [body, tn('users.delete.banks', u.ownedBanks.length, {
        name: u.username, names: (owned.length ? owned : [t('common.unknownBank')]).join(t('common.listSep')),
      })].join('\n')
      : body + (uiLang() === 'en' ? ' ' : '') + t('users.delete.noBanks');
    if (!(await ask({
      title: t('users.delete.title', { name: u.username }),
      body: text,
      ok: t('users.delete.ok'), danger: true,
    }))) return;
    await change(u, () => deleteUser(u.id), t('users.deleted', { name: u.username }), `us-delete-${u.id}`);
  });
}

// ---------------------------------------------------------------- list

function mini(id: string, label: string, aria: string, disabled: boolean, why: string | null, onclick: () => void, danger = false): HTMLElement {
  return h('button', {
    class: 'btn us-mini' + (danger ? ' danger' : ''), id, type: 'button', disabled, 'aria-label': aria,
    title: disabled && why ? why : aria, 'aria-describedby': disabled && why ? id.replace(/^us-\w+-/, 'us-why-') : null, onclick,
  }, label);
}

/** `text` with every `YYYY-MM-DD HH:MM` kept on one line (a line break after a hyphen of the date reads badly) */
function withUnbrokenTimes(text: string): Child[] {
  return text.split(/(\d{4}-\d{2}-\d{2} \d{2}:\d{2})/).map((part, i) => (i % 2 ? h('span', { class: 'us-nw' }, part) : part));
}

/** Unlock at the end of the row's lock note, described by it (what it ends and for which browsers) */
function unlockButton(u: UserRow, locked: boolean, blocked: boolean, noteId: string | null): HTMLElement {
  const b = mini(`us-unlock-${u.id}`, t('users.act.unlock'), t(locked ? 'users.act.unlockAria' : 'users.act.unlockTriesAria', { name: u.username }),
    blocked, null, () => void unlock(u, locked));
  if (noteId) b.setAttribute('aria-describedby', noteId);
  // A small button in the run of the lock note (v0.7)
  b.classList.add('us-change');
  return b;
}

/**
 * The row's line about question banks (v0.6.5): administrators see every bank; a plain user sees the chosen banks (counted
 * and named, so the names can be read on a phone too) or none yet, and (v0.7) whether the banks imported later are added
 * (every bank there is now plus the later ones reads "all"), plus a bank manager's own uploads.
 * v0.7: a plain user's line ends in a small "Change" button that opens the bank dialog (it used to be a "Question banks"
 * action beside Edit, which pushed Delete onto a second line on phones). Its accessible name says what it changes.
 */
function bankLine(u: UserRow, blocked: boolean): HTMLElement {
  const v = accessView(u, S.banks);
  const sep = t('common.listSep');
  const main = v.kind === 'admin' ? t('users.banks.admin')
    : v.kind === 'all' ? t('users.banks.all')
      : v.kind === 'some' ? t(v.later ? 'users.banks.someLater' : 'users.banks.some', { n: v.names.length, names: v.names.join(sep) })
        : t(v.later ? 'users.banks.noneLater' : 'users.banks.none');
  // Administrators see every bank: their rows have no bank choice
  const change = u.role === 'user'
    ? h('button', {
      class: 'btn us-mini us-change', id: `us-banks-${u.id}`, type: 'button', disabled: blocked,
      'aria-label': t('users.act.banksAria', { name: u.username }), title: t('users.act.banksAria', { name: u.username }),
      onclick: () => void chooseBanks(u),
    }, t('users.banks.change'))
    : null;
  return h('p', { class: 'us-banks' + (v.kind === 'none' && !v.later && !v.own.length ? ' none' : ''), id: `us-access-${u.id}` },
    h('span', null, main),
    change ? [' ', change] : null,
    v.own.length ? h('span', { class: 'us-banks-own' }, t('users.banks.own', { names: v.own.join(sep) })) : null);
}

function userRow(u: UserRow, list: readonly UserRow[]): HTMLElement {
  const lim = limitsOf(u, list);
  const busy = busyId === u.id;
  const blocked = busyId != null;
  const left = lockLeft(u);
  const lock = lockView(u.lock, left);
  const locked = lock.state === 'locked';
  const tags = [
    h('span', { class: 'tag ' + (u.role === 'admin' ? 'b' : 'k') }, roleName(u.role)),
    lim.self ? h('span', { class: 'tag k' }, t('users.you')) : null,
    u.disabled ? h('span', { class: 'tag r' }, t('users.status.disabled')) : null,
    // The explanation of both throttle tags is the visible note under the row (a title tooltip cannot be reached on a phone
    // or with the keyboard)
    locked
      ? h('span', { class: 'tag r us-lock', id: `us-lock-${u.id}` }, t('users.status.locked', { wait: waitShort(left) }))
      : null,
    !u.disabled && !locked ? h('span', { class: 'tag k' }, t('users.status.normal')) : null,
    // Past round 1 without a running lock: how many tries are left before the next (longer) lock
    lock.state === 'tries' ? h('span', { class: 'tag k' }, tn('users.status.tries', lock.triesLeft)) : null,
    !u.disabled && u.mustChangePassword ? h('span', { class: 'tag k' }, t('users.status.mustChange')) : null,
    // v0.6.5: a plain user who may import and manage their own question banks (administrators always may)
    u.role === 'user' && u.canManageBanks ? h('span', { class: 'tag b' }, t('users.tag.manager')) : null,
  ];
  const access = bankLine(u, blocked);
  const note = lockNote(u.lock, lock, {
    name: u.username, enabled: !u.disabled, mayHaveKnownBrowsers: u.lastLoginAt != null && !u.mustChangePassword,
  }).map((p) => (p.n == null ? t(p.key, p.params) : tn(p.key, p.n, p.params))).join(uiLang() === 'en' ? ' ' : '');
  const noteId = `us-locknote-${u.id}`;
  const meta = [
    t('users.meta.created', { at: fmtAt(u.createdAt) }),
    u.lastLoginAt ? t('users.meta.lastLogin', { at: fmtAt(u.lastLoginAt) }) : t('users.meta.never'),
    tn('users.meta.records', u.records),
  ];
  return h('li', { class: 'us-row' + (u.disabled ? ' off' : ''), 'aria-busy': busy ? 'true' : null },
    icon(ICONS.user, 'ico us-ico'),
    h('div', { class: 'us-main' },
      h('div', { class: 'us-line' },
        h('h3', { class: 'us-name', id: `us-name-${u.id}`, tabindex: '-1', title: u.username }, u.username),
        h('span', { class: 'us-tags' }, tags)),
      h('p', { class: 'us-meta' }, meta.map((m, i) => [
        i ? ' ' : null,
        h('span', { class: 'us-nw' }, m, i < meta.length - 1 ? h('span', { class: 'us-dot', 'aria-hidden': 'true' }, '·') : null),
      ])),
      access,
      // v0.7: Unlock sits at the end of the lock note (like "Change" in the bank line), so the row keeps its four actions on
      // one line on phones
      note || lock.unlock
        ? h('p', { class: 'us-locknote' },
          note ? h('span', { id: noteId }, withUnbrokenTimes(note)) : null,
          lock.unlock ? [note ? ' ' : null, unlockButton(u, locked, blocked, note ? noteId : null)] : null)
        : null,
      lim.why ? h('p', { class: 'us-why', id: `us-why-${u.id}` }, lim.why) : null),
    h('div', { class: 'us-acts' },
      busy ? h('span', { class: 'us-busy', role: 'status' }, t('users.act.busy')) : null,
      mini(`us-edit-${u.id}`, t('users.act.edit'), t('users.act.editAria', { name: u.username }), blocked, null, () => void editUser(u, lim)),
      mini(`us-reset-${u.id}`, t('users.act.reset'), t('users.act.resetAria', { name: u.username }), blocked || lim.self, lim.why,
        () => void resetPassword(u)),
      u.disabled
        ? mini(`us-toggle-${u.id}`, t('users.act.enable'), t('users.act.enableAria', { name: u.username }), blocked, null, () => void toggleDisabled(u))
        : mini(`us-toggle-${u.id}`, t('users.act.disable'), t('users.act.disableAria', { name: u.username }), blocked || lim.self || lim.lastAdmin, lim.why,
          () => void toggleDisabled(u)),
      mini(`us-delete-${u.id}`, t('common.delete'), t('users.act.deleteAria', { name: u.username }), blocked || lim.self || lim.lastAdmin, lim.why,
        () => void removeUser(u), true)));
}

function listPanel(): HTMLElement {
  const list = users;
  const head = h('div', { class: 'us-head' },
    h('h2', { id: 'us-list-h', tabindex: '-1' }, t('users.list.title'), list ? h('span', { class: 'us-count' }, tn('users.count', list.length)) : null),
    add.open ? null : h('button', {
      class: 'btn us-mini us-addbtn', id: 'us-add-btn', type: 'button', disabled: !list,
      onclick: () => {
        add.open = true;
        focusNext = 'us-add-name';
        render();
      },
    }, icon([{ d: 'M12 5.5v13' }, { d: 'M5.5 12h13' }], 'ico us-plus'), t('users.add')));

  let body: Child;
  if (!list) {
    body = loadError
      ? h('div', { class: 'err', role: 'alert' },
        h('p', null, t('users.err.load', { reason: errorText(loadError) })),
        h('div', { class: 'acts' }, h('button', { class: 'btn', type: 'button', onclick: () => void load() }, t('common.retry'))))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  } else {
    body = h('ul', { class: 'us-rows' }, list.map((u) => userRow(u, list)));
  }
  return h('section', { class: 'panel us-list', 'aria-labelledby': 'us-list-h', 'aria-busy': loading && !list ? 'true' : null },
    head,
    add.open ? addForm() : null,
    actionError ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('users.err.action', { reason: errorText(actionError) }))) : null,
    body);
}

export function renderUsers(): HTMLElement {
  if (wanted && !loading) void load();
  startTicker();
  startPoller();
  const root = h('div', { class: 'page page-users' },
    pageHead(t('users.title')),
    h('p', { class: 'us-lead' }, t('users.lead')),
    listPanel());
  focusAfter(root);
  return root;
}
