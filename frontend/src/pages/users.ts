// User management (#/users, administrators only; the router sends everyone else home). One compact row per user with small
// actions, an inline form to add a user, small dialogs to edit a user and to reset a password, confirmations before disabling
// and deleting. The backend enforces every rule; this page shows the blocked actions disabled with the reason (your own
// account, the last enabled administrator) and always redraws the list from the server's answer.
// Every row shows the account status as tags: Normal (enabled, not locked), Disabled, and "Locked · N min left" while the
// login throttle locks the name (both when both apply), plus "Must choose a new password". A locked row has an Unlock action.
// The time left is counted down here: each lock's end is fixed on the browser's monotonic clock when the list arrives
// (performance.now() + retryAfter, so the computer clocks of server and browser need not agree), a one-second timer rewrites the
// tags while the page is open and a lock runs, and when a lock ends the list is read again (the next round, tries left).
import '../styles/users.css';
import { ApiError } from '../api/client.ts';
import { createUser, deleteUser, fetchUsers, resetUserPassword, unlockUser, updateUser } from '../api/users.ts';
import { pageHead } from '../components/common.ts';
import { field, passwordLengthOk, usernameOk } from '../components/fields.ts';
import { icon, ICONS, roleName } from '../components/shell.ts';
import { secondsLeft } from '../domain/wait.ts';
import { errorText } from '../i18n/errors.ts';
import { t, tn } from '../i18n/index.ts';
import { waitShort } from '../i18n/wait.ts';
import { onRouteChange, onSignedIn, render } from '../state/actions.ts';
import { S } from '../state/store.ts';
import type { Role, UserRow, UserUpdate } from '../types/api.ts';
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
const add = { open: false, username: '', password: '', role: 'user' as Role, shown: false, busy: false, error: null as string | null };
/** The element to focus after the next render */
let focusNext: string | null = null;
/** User id → when their sign-in lock ends (performance.now() milliseconds), for the users whose lock runs */
const lockEnds = new Map<number, number>();
/** The one-second timer that counts the locks down while the page is open */
let ticker: ReturnType<typeof setInterval> | null = null;

function resetAdd(): void {
  Object.assign(add, { open: false, username: '', password: '', role: 'user', shown: false, busy: false, error: null });
}

onRouteChange((from, to) => {
  if (to.name === 'users') wanted = true;
  if (from.name === 'users' && to.name !== 'users') {
    resetAdd();
    actionError = null;
    stopTicker();
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
  for (const u of list) {
    if (u.lock && u.lock.retryAfter > 0) lockEnds.set(u.id, now + u.lock.retryAfter * 1000);
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
    adopt(await createUser(name, add.password, add.role));
    toast(t('users.created', { name }));
    resetAdd();
    focusNext = 'us-add-btn';
  } catch (e) {
    add.error = errorText(e);
    add.busy = false;
    focusNext = e instanceof ApiError && e.status === 409 ? 'us-add-name' : null;
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
  })),
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
    const draft = { username: u.username, role: u.role, enabled: !u.disabled };
    const locked = lim.self || lim.lastAdmin;
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
      }, locked ? 'us-edit-why' : undefined),
      h('label', { class: 'dlg-check us-enabled' }, enabled,
        h('span', null, h('span', { class: 'dlg-check-l' }, t('users.field.enabled')), h('span', { class: 'dlg-check-h' }, t('users.field.enabledHint')))),
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
        const body: UserUpdate = { id: u.id };
        if (next !== u.username) body.username = next;
        if (draft.role !== u.role) body.role = draft.role;
        if (draft.enabled === u.disabled) body.disabled = !draft.enabled;
        if (Object.keys(body).length === 1) {
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

function unlock(u: UserRow): Promise<void> {
  // No confirmation: unlocking only gives the user their five tries back
  return change(u, () => unlockUser(u.id), t('users.unlocked', { name: u.username }), `us-unlock-${u.id}`);
}

function removeUser(u: UserRow): Promise<void> {
  return guarded(async () => {
    if (!(await ask({
      title: t('users.delete.title', { name: u.username }),
      body: t('users.delete.body', { records: tn('users.meta.records', u.records) }),
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

function userRow(u: UserRow, list: readonly UserRow[]): HTMLElement {
  const lim = limitsOf(u, list);
  const busy = busyId === u.id;
  const blocked = busyId != null;
  const left = lockLeft(u);
  const locked = left > 0;
  // Past round 1 without a running lock: how many tries are left before the next (longer) lock
  const tries = !locked && u.lock && u.lock.retryAfter === 0 && u.lock.round > 1 ? u.lock.triesLeft : null;
  const tags = [
    h('span', { class: 'tag ' + (u.role === 'admin' ? 'b' : 'k') }, roleName(u.role)),
    lim.self ? h('span', { class: 'tag k' }, t('users.you')) : null,
    u.disabled ? h('span', { class: 'tag r' }, t('users.status.disabled')) : null,
    locked
      ? h('span', {
        class: 'tag r us-lock', id: `us-lock-${u.id}`,
        title: t('users.status.lockedTitle', { at: u.lock?.until ?? '', round: u.lock?.round ?? 1, name: u.username }),
      }, t('users.status.locked', { wait: waitShort(left) }))
      : null,
    !u.disabled && !locked ? h('span', { class: 'tag k' }, t('users.status.normal')) : null,
    tries != null ? h('span', { class: 'tag k', title: tn('users.status.triesTitle', tries) }, tn('users.status.tries', tries)) : null,
    !u.disabled && u.mustChangePassword ? h('span', { class: 'tag k' }, t('users.status.mustChange')) : null,
  ];
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
      lim.why ? h('p', { class: 'us-why', id: `us-why-${u.id}` }, lim.why) : null),
    h('div', { class: 'us-acts' },
      busy ? h('span', { class: 'us-busy', role: 'status' }, t('users.act.busy')) : null,
      locked ? mini(`us-unlock-${u.id}`, t('users.act.unlock'), t('users.act.unlockAria', { name: u.username }), blocked, null, () => void unlock(u)) : null,
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
  const root = h('div', { class: 'page page-users' },
    pageHead(t('users.title')),
    h('p', { class: 'us-lead' }, t('users.lead')),
    listPanel());
  focusAfter(root);
  return root;
}
