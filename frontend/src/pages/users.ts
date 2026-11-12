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
// v0.8: each plain user's row has an "AI+ service" switch (no confirmation; switching off keeps their AI data) and the AI+
// badge; the New user form has an AI+ box under the role. Round 2 (owner): the switches and the box exist only while AI+
// is available (ai-service connected and the AI+ service on in AI settings; `aiAvailable` of every list answer,
// domain/user-admin.ts); a switch shows its new position only after the server said yes, and a refusal of a page drawn
// before AI+ went away ("Turn on the AI+ service in AI settings first.") is shown as it is, never as a success. The badge
// follows domain/badge-state.ts: bright, dark with the reason in a tooltip, or none. The question-bank choice (New user form and the "Change" dialog) is the
// compact block the owner drew: one row "Manage banks · Select all · Deselect all" directly on top of the list, "Include
// banks imported later" as the first entry of the list, the explanations as tooltips (hover and keyboard focus).
// v1.0 (phones first): every row action is an icon button on every width (named with the user, the reason it is greyed as tip /
// toast), the row is compact (no default tags, no per-row sentences: the own-account / last-administrator reason and the lock
// explanation are (i) tips, the facts are one line), the AI+ switch sits in the action row (never under a button), the bank line
// is one 44 px row that opens the bank dialog, a new user's row is scrolled into view and highlighted, and a paused AI+ tag links
// to AI settings.
import '../styles/users.css';
import { fetchBanks } from '../api/bank.ts';
import { ApiError } from '../api/client.ts';
import { createUser, deleteUser, fetchUsers, resetUserPassword, unlockUser, updateUser, type UsersList } from '../api/users.ts';
import { aiBadge } from '../components/ai-badge.ts';
import { pageHead } from '../components/common.ts';
import { field, passwordLengthOk, usernameOk } from '../components/fields.ts';
import { iconButton, shortLabel, withReason } from '../components/icon-button.ts';
import { miniIcon, type MiniIcon } from '../components/mini-icons.ts';
import { miniSwitch } from '../components/mini-switch.ts';
import { icon, ICONS, roleName } from '../components/shell.ts';
import { holdTip, infoTip, tipEl, withTip } from '../components/tooltip.ts';
import { badgeState } from '../domain/badge-state.ts';
import { lockNote, lockView } from '../domain/lock.ts';
import { SEARCH_FROM, aiLineKind, createAi, filterUsers, isServiceOff, lockUntilLabel, shortAt } from '../domain/user-admin.ts';
import { secondsLeft } from '../domain/wait.ts';
import { errorText } from '../i18n/errors.ts';
import { t, tn, uiLang } from '../i18n/index.ts';
import { onRouteChange, onSignedIn, refreshAi, refreshLibrary, render } from '../state/actions.ts';
import {
  accessChange, accessOf, accessView, deselectAllBanks, draftOf, editChange, isTicked, ownedBankNames, ownerView, toggleAll,
  toggleBank, toolState, type AccessDraft,
} from '../state/bank-access.ts';
import { aiStatus } from '../state/ai.ts';
import { S } from '../state/store.ts';
import type { BankSummary, Role, UserRow } from '../types/api.ts';
import { ask, formDialog } from '../utils/dialog.ts';
import { h, scrollToY, type Child } from '../utils/dom.ts';
import { TOUCH_MQ, matches } from '../utils/media.ts';
import { toast } from '../utils/toast.ts';

const ROLES: readonly Role[] = ['user', 'admin'];
const ROLE_DESC: Record<Role, 'users.role.userDesc' | 'users.role.adminDesc'> = { user: 'users.role.userDesc', admin: 'users.role.adminDesc' };

// ---------------------------------------------------------------- page-local state

let users: UserRow[] | null = null;
/** v0.8 round 2: AI+ is available, from the newest list answer (false until the first one) */
let aiAvailable = false;
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
   * so they see none until chosen; with (v0.8) "Manage banks", the bank-manager permission (off by default)
   */
  banks: draftOf([], false),
  /** v0.8: the AI+ service (off by default; sent only while the box is drawn, i.e. while AI+ is available) */
  ai: false,
};
/** The element to focus after the next render */
let focusNext: string | null = null;
/** US-8: the row of the user just created: scrolled into view, highlighted for 2 s, its name focused (after the next render) */
let newRowId: number | null = null;
let newRowShown = false;
/** US-11: the search text (a list of more than SEARCH_FROM users has a search field) */
let searchText = '';
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
    open: false, username: '', password: '', role: 'user', shown: false, busy: false, error: null, banks: draftOf([], false), ai: false,
  });
}

onRouteChange((from, to) => {
  if (to.name === 'users') wanted = true;
  if (from.name === 'users' && to.name !== 'users') {
    resetAdd();
    actionError = null;
    searchText = '';
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

/** Watches the running locks (the tag names the end time, so nothing is rewritten); reads the list again when a lock ends. */
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
    const answer = await fetchUsers();
    if (mine !== seq) return;
    take(answer);
    loadError = null;
  } catch (e) {
    if (mine !== seq) return;
    loadError = e;
  } finally {
    if (mine === seq) loading = false;
  }
  if (onUsersPage()) render();
}

/** The list and the AI+ availability of an answer (every answer has both) */
function take(answer: UsersList): void {
  users = answer.users;
  aiAvailable = answer.aiAvailable;
  noteLocks(answer.users);
}

/** The list from a change's answer (newer than any list still being read) */
function adopt(answer: UsersList): void {
  seq += 1;
  loading = false;
  take(answer);
  const list = answer.users;
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

/** US-6: on a touch screen, holding an option shows its tip without toggling it (the hover / focus tip stays for the mouse) */
function holdOnTouch<T extends HTMLElement>(el: T, text: string): T {
  return matches(TOUCH_MQ) ? holdTip(el, text) : el;
}

interface ChecklistOptions {
  /** Prefix of the element ids */
  id: string;
  banks: readonly BankSummary[];
  get: () => AccessDraft;
  set: (d: AccessDraft) => void;
  /** A request runs: every control off */
  disabled: boolean;
  /** The role the choice is for: an administrator always sees and manages every bank (Manage banks hidden, the list off) */
  role: Role;
  /** A short line under a bank's name (who uploaded it), or null */
  noteOf?: (b: BankSummary) => string | null;
}

interface Checklist {
  el: HTMLFieldSetElement;
  setDisabled: (off: boolean) => void;
  setRole: (r: Role) => void;
  /** Where a dialog puts the focus: the first bank's box, or "Include banks imported later" when there are no banks */
  first: HTMLInputElement;
}

/**
 * The question-bank choice (v0.8, as the owner drew it), in the New user form and the "Change" dialog:
 *   ┌ ☐ Manage banks   ☐ Select all   ☐ Deselect all ┐   one row directly on top of the list, sharing its border
 *   │ ☐ Include banks imported later                  │   the first entry of the list, with an icon and a thin divider
 *   │ ☐ General knowledge   uploaded by …             │
 *   └ …                                               ┘
 * The explanations are tooltips (hover and keyboard focus), no paragraphs. "Select all" / "Deselect all" change only the
 * listed banks; "Include banks imported later" and "Manage banks" stay as they are. "Deselect all" with nothing ticked shows
 * ticked and refuses the click (still focusable, so its tip can be read). For an administrator Manage banks is hidden and
 * the rest is off (administrators always see and manage every bank; the legend's tip then says so). Clicks change the boxes
 * in place (no redraw), so the keyboard focus stays where it is.
 */
function bankChecklist(o: ChecklistOptions): Checklist {
  let busy = o.disabled;
  let role = o.role;
  const id = o.id;
  const listId = `${id}-list`;
  const boxes: Array<[number, HTMLInputElement]> = [];

  // The legend's explanation: a small info button next to the legend (not inside it)
  const legendTip = tipEl(`${id}-tip-banks`, '');
  const legendBtn = h('button', {
    type: 'button', class: 'tip-btn', 'aria-label': t('users.tip.banksBtn'), 'aria-describedby': legendTip.id,
  }, miniIcon('info'));

  const manage = h('input', {
    type: 'checkbox', id: `${id}-manage`, 'aria-describedby': `${id}-tip-manage`,
    onchange: () => o.set({ ...o.get(), manage: manage.checked }),
  });
  const manageWrap = withTip(
    holdOnTouch(h('label', { class: 'us-tool', for: manage.id }, manage, miniIcon('key'), h('span', null, shortLabel('users.banks.manage', 'users.tools.manageShort'))), t('users.tip.manage')),
    tipEl(`${id}-tip-manage`, t('users.tip.manage')));
  manageWrap.id = `${id}-manage-wrap`;

  const all = h('input', {
    type: 'checkbox', id: `${id}-all`, 'aria-controls': listId, 'aria-describedby': `${id}-tip-all`,
    onchange: () => {
      o.set(toggleAll(o.get(), o.banks));
      sync();
    },
  });
  const none = h('input', {
    type: 'checkbox', id: `${id}-none`, 'aria-controls': listId, 'aria-describedby': `${id}-tip-none`,
    // Nothing is ticked: the box shows ticked and does nothing (aria-disabled, still focusable)
    onclick: (e: MouseEvent) => {
      if (none.getAttribute('aria-disabled') === 'true') e.preventDefault();
    },
    onchange: () => {
      o.set(deselectAllBanks(o.get()));
      sync();
    },
  });
  const later = h('input', {
    type: 'checkbox', id: `${id}-later`, 'aria-describedby': `${id}-tip-later`,
    onchange: () => o.set({ ...o.get(), later: later.checked }),
  });

  const rows = o.banks.map((b) => {
    const box = h('input', {
      type: 'checkbox', id: `${id}-b${b.id}`, value: String(b.id),
      onchange: () => {
        o.set(toggleBank(o.get(), b.id, box.checked));
        sync();
      },
    });
    boxes.push([b.id, box]);
    const note = o.noteOf?.(b) ?? null;
    return h('li', null, h('label', { class: 'us-bank', for: box.id }, box,
      h('span', { class: 'us-bank-t' }, h('span', { class: 'us-bank-n' }, b.name), note ? h('span', { class: 'us-bank-h' }, note) : null)));
  });

  const sync = (): void => {
    const d = o.get();
    const admin = role === 'admin';
    const off = busy || admin;
    const noBanks = o.banks.length === 0;
    legendTip.textContent = t(admin ? 'users.tip.adminBanks' : 'users.tip.banks');
    manageWrap.hidden = admin;
    manage.checked = d.manage;
    manage.disabled = busy;
    later.checked = d.later;
    later.disabled = off;
    for (const [bankId, box] of boxes) {
      box.checked = isTicked(d, bankId);
      box.disabled = off;
    }
    const state = toolState(d, o.banks);
    all.checked = !noBanks && state === 'all';
    all.indeterminate = state === 'some';
    all.disabled = off || noBanks;
    none.checked = state === 'none';
    none.disabled = off || noBanks;
    if (state === 'none' && !none.disabled) none.setAttribute('aria-disabled', 'true');
    else none.removeAttribute('aria-disabled');
  };

  const el = h('fieldset', { class: 'fld us-banksfld' },
    h('legend', { class: 'fld-l' }, t('users.field.banks')),
    withTip(legendBtn, legendTip, 'us-legend-tip'),
    h('div', { class: 'us-banktools' },
      manageWrap,
      withTip(holdOnTouch(h('label', { class: 'us-tool', for: all.id }, all, miniIcon('checkAll'),
        h('span', null, shortLabel('users.banks.selectAll', 'users.tools.allShort'))), t('users.tip.selectAll')),
      tipEl(`${id}-tip-all`, t('users.tip.selectAll'))),
      withTip(holdOnTouch(h('label', { class: 'us-tool', for: none.id }, none, miniIcon('squareMinus'),
        h('span', null, shortLabel('users.banks.deselectAll', 'users.tools.noneShort'))), t('users.tip.deselectAll')),
      tipEl(`${id}-tip-none`, t('users.tip.deselectAll')))),
    h('ul', { class: 'us-banklist', id: listId },
      // Always the first entry, also when there are no banks yet
      h('li', { class: 'us-later-row' }, withTip(
        holdOnTouch(h('label', { class: 'us-bank us-later', for: later.id }, later, miniIcon('clockPlus'),
          h('span', { class: 'us-bank-n' }, t('users.banks.later'))), t('users.tip.later')),
        tipEl(`${id}-tip-later`, t('users.tip.later')))),
      rows,
      o.banks.length ? null : h('li', { class: 'us-banks-empty' }, t('users.banks.noneYet'))));
  sync();
  return {
    el,
    first: boxes[0]?.[1] ?? later,
    setDisabled: (v: boolean) => {
      busy = v;
      sync();
    },
    setRole: (r: Role) => {
      role = r;
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

/**
 * v0.8: the New user form's AI+ service box, on its own line under the role (never inside the bank choice); its
 * explanation is a tooltip. Hidden for an administrator (administrators always have AI while AI+ is available). Round 2:
 * drawn only while AI+ is available (null otherwise; the form then creates the user without the service).
 */
function aiBox(): { el: HTMLElement; setRole: (r: Role) => void } | null {
  if (!aiAvailable) return null;
  const box = h('input', {
    type: 'checkbox', id: 'us-add-ai', checked: add.ai, disabled: add.busy, 'aria-describedby': 'us-add-ai-tip',
    onchange: () => {
      add.ai = box.checked;
    },
  });
  const el = withTip(
    h('label', { class: 'dlg-check us-check us-aibox', for: box.id }, box, miniIcon('sparkle'), h('span', { class: 'dlg-check-l' }, t('users.field.ai'))),
    tipEl('us-add-ai-tip', t('users.tip.ai')), 'us-aiwrap');
  const setRole = (r: Role): void => {
    el.hidden = r === 'admin';
  };
  setRole(add.role);
  return { el, setRole };
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
    const admin = add.role === 'admin';
    // v0.8 round 2: the AI+ service only when its box is drawn (AI+ available, a plain user) and ticked
    const ai = createAi(aiAvailable, add.role, add.ai);
    // An administrator always sees and manages every bank and has AI while AI+ is available: nothing of that is stored
    const answer = await createUser({
      username: name, password: add.password, role: add.role, banks: access.banks, newBanks: access.newBanks,
      canManageBanks: admin ? false : access.canManageBanks, ai,
    });
    adopt(answer);
    if (ai) void refreshAi(true);
    toast(t('users.created', { name }));
    resetAdd();
    // US-8: the new row is scrolled into view, highlighted for 2 s and its name focused (renderUsers)
    newRowId = answer.users.find((u) => u.username === name)?.id ?? null;
    newRowShown = false;
    searchText = '';
    focusNext = newRowId == null ? 'us-add-btn' : null;
  } catch (e) {
    add.error = errorText(e);
    add.busy = false;
    // AI+ went away while the form was open (409 ai.serviceOff, nothing created): the list is read again, so the AI+ box
    // disappears and the form can be sent again without it. Any other 409 is a username that is taken.
    const serviceOff = isServiceOff(e);
    if (serviceOff) void load();
    focusNext = e instanceof ApiError && e.status === 409 && !serviceOff ? 'us-add-name' : null;
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
  // The bank choice, Manage banks and AI+ only matter for plain users (administrators always have all of it)
  const banks = bankChecklist({
    id: 'us-add-banks', banks: S.banks, disabled: add.busy, role: add.role,
    get: () => add.banks,
    set: (d) => {
      add.banks = d;
    },
    noteOf: bankNoteFor(null),
  });
  const ai = aiBox();
  return h('form', {
    class: 'us-add', id: 'us-add', novalidate: true, 'aria-labelledby': 'us-add-h', 'aria-busy': add.busy ? 'true' : null,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      void submitAdd();
    },
  },
  h('h3', { id: 'us-add-h' }, t('users.add.title')),
  h('div', { class: 'us-add-grid' }, name.el, pass.el, h('div', { class: 'us-rolecol' }, roleChoice('us-add-role', add.role, add.busy, (r) => {
    add.role = r;
    banks.setRole(r);
    ai?.setRole(r);
  }), ai?.el ?? null)),
  h('div', { class: 'us-add-access' }, banks.el),
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

/**
 * Runs one change of a user's row: the row shows it is busy, the answer replaces the list, a failure is shown above it.
 * The success toast comes only with the server's yes. v0.8 round 2: an AI+ switch of a page drawn before AI+ went away is
 * refused (409 ai.serviceOff): that refusal is the toast, and the list is read again (its answer has no AI+ switches).
 */
async function change(u: UserRow, run: () => Promise<UsersList>, done: string, control: string): Promise<void> {
  if (busyId != null) return;
  const before = users;
  busyId = u.id;
  actionError = null;
  render();
  let serviceOff = false;
  try {
    adopt(await run());
    toast(done);
  } catch (e) {
    serviceOff = isServiceOff(e);
    if (serviceOff) {
      toast(errorText(e));
      void load();
    } else {
      actionError = e;
      refreshAfter(e);
    }
  }
  busyId = null;
  // Decided after the redraw, when the buttons show their new state
  focusNext = null;
  if (!onUsersPage()) return;
  render();
  // (the AI+ switch disappears with the list that is being read again: the row's name keeps the focus)
  const target = serviceOff ? `us-name-${u.id}` : focusAfterChange(u.id, before, control);
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
    // (v0.8: "Manage banks" moved into the bank choice of the "Change" dialog; the stored flag is kept as it is here)
    const draft = { username: u.username, role: u.role, enabled: !u.disabled, canManageBanks: u.canManageBanks };
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
      h('label', { class: 'dlg-check us-check us-enabled' }, enabled,
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

/**
 * "Change" in a plain user's bank line: the bank choice in a dialog (the chosen banks, whether later ones are included and,
 * v0.8, Manage banks). It opens with the focus on the first bank (never on Manage banks).
 */
function chooseBanks(u: UserRow): Promise<void> {
  return guarded(async () => {
    const banks = await currentBanks();
    if (!onUsersPage()) return;
    let draft = draftOf(u.banks, u.newBanks, u.canManageBanks);
    const list = bankChecklist({
      id: 'us-bdlg', banks, disabled: false, role: u.role,
      get: () => draft,
      set: (d) => {
        draft = d;
      },
      noteOf: bankNoteFor(u),
    });
    let saved: string | null = null;
    const ok = await formDialog({
      title: t('users.banks.title', { name: u.username }),
      content: list.el,
      focus: list.first,
      ok: t('users.edit.save'),
      busy: t('users.edit.saving'),
      submit: async () => {
        const ch = accessChange(u, draft, banks);
        if (!ch.body) {
          saved = t('users.edit.nothing');
          return null;
        }
        try {
          adopt(await updateUser(ch.body));
          saved = ch.banks
            ? t('users.banks.done', { name: u.username })
            : t(draft.manage ? 'users.banks.manageOn' : 'users.banks.manageOff', { name: u.username });
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

/**
 * v0.8: the AI+ service switch of a plain user's row. No confirmation either way: switching it off keeps the user's chats
 * and memories (they can still read and delete them), and stops an answer being written for them. Round 2: the switch
 * keeps its old position until the server answered; "on" / "off" is said only after a 200 (change()).
 */
function setAi(u: UserRow, on: boolean): Promise<void> {
  return change(u, async () => {
    const answer = await updateUser({ id: u.id, ai: on });
    void refreshAi(true);
    return answer;
  }, t(on ? 'users.ai.done.on' : 'users.ai.done.off', { name: u.username }), `us-ai-${u.id}`);
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

/**
 * One icon action of a row (US-1): named with the user (tip on a mouse device, tap-and-hold on touch). `reason`: the action is
 * not available for a rule the administrator must be able to read (your own account, the last administrator): the button is
 * greyed (aria-disabled), its tip says why and a tap shows it as a toast. `blocked`: a change of the list runs (really disabled).
 */
function act(id: string, ic: MiniIcon, aria: string, blocked: boolean, reason: string | null, onclick: () => void, danger = false): HTMLElement {
  const tone = danger ? 'danger' : 'default';
  if (reason) {
    const b = iconButton({ icon: ic, label: aria, tone });
    b.id = id;
    return withReason(b, reason);
  }
  const b = iconButton({ icon: ic, label: aria, tone, onClick: onclick });
  b.id = id;
  b.disabled = blocked;
  return b;
}

/**
 * The row's line about question banks, only for a plain user (an administrator sees every bank: no sentence, US-2): one 44 px
 * tappable row (the chosen banks counted and named, a chevron) that opens the bank dialog (US-4); its accessible name says what
 * it changes. v0.6.5: the names are written out so they can be read on a phone; v0.7: whether the banks imported later are
 * added, plus a bank manager's own uploads.
 */
function bankLine(u: UserRow, blocked: boolean): HTMLElement | null {
  const v = accessView(u, S.banks);
  if (u.role !== 'user' || v.kind === 'admin') return null;
  const sep = t('common.listSep');
  const main = v.kind === 'all' ? t('users.banks.all')
    : v.kind === 'some' ? t(v.later ? 'users.banks.someLater' : 'users.banks.some', { n: v.names.length, names: v.names.join(sep) })
      : t(v.later ? 'users.banks.noneLater' : 'users.banks.none');
  const none = v.kind === 'none' && !v.later && !v.own.length;
  return h('button', {
    class: 'us-bankrow' + (none ? ' none' : ''), id: `us-banks-${u.id}`, type: 'button', disabled: blocked,
    'aria-label': t('users.act.banksAria', { name: u.username }), onclick: () => void chooseBanks(u),
  },
  h('span', { class: 'us-bank-sum', id: `us-access-${u.id}` }, main,
    v.own.length ? h('span', { class: 'us-banks-own' }, t('users.banks.own', { names: v.own.join(sep) })) : null),
  miniIcon('fwd', 'us-mi us-bank-go'));
}

/**
 * v0.8: the AI+ switch of a plain user, in the action row at its left (US-3: never under a button, so no two tap areas share
 * ground); administrators always have AI while AI+ is available (no switch, no sentence). Round 2: nothing while AI+ is not
 * available (the stored switch is kept).
 */
function aiSwitch(u: UserRow, blocked: boolean): HTMLElement | null {
  if (aiLineKind(aiAvailable, u.role) !== 'switch') return null;
  return h('div', { class: 'us-ailine' },
    h('span', { class: 'us-ai-l', 'aria-hidden': 'true' },
      h('span', { class: 'l-full' }, t('users.field.ai')), h('span', { class: 'l-short' }, t('users.field.aiShort'))),
    miniSwitch({
      id: `us-ai-${u.id}`, checked: u.ai, label: t('users.ai.aria', { name: u.username }), disabled: blocked,
      onToggle: (on) => void setAi(u, on),
    }));
}

/**
 * v0.8: the badge in a row (round 2, domain/badge-state.ts): bright while the user can use AI now; dark, with the reason
 * in a tooltip, for a plain user with the AI+ service whose AI+ function is suspended or while AI+ is not available; none
 * without the service (and for an administrator while AI+ is not available). Shown whether or not the switch is.
 */
function rowBadge(u: UserRow): HTMLElement | null {
  return aiBadge(badgeState({ role: u.role, service: u.ai, suspended: u.aiSuspended, available: aiAvailable }), { admin: true });
}

function userRow(u: UserRow, list: readonly UserRow[]): HTMLElement {
  const lim = limitsOf(u, list);
  const busy = busyId === u.id;
  const blocked = busyId != null;
  const left = lockLeft(u);
  const lock = lockView(u.lock, left);
  const locked = lock.state === 'locked';
  // The lock's full explanation is an (i) tip next to its tag (US-10); the Unlock button is described by it
  const note = lockNote(u.lock, lock, {
    name: u.username, enabled: !u.disabled, mayHaveKnownBrowsers: u.lastLoginAt != null && !u.mustChangePassword,
  }).map((p) => (p.n == null ? t(p.key, p.params) : tn(p.key, p.n, p.params))).join(uiLang() === 'en' ? ' ' : '');
  const noteId = `us-locknote-${u.id}`;
  const lockTip = note ? infoTip(noteId, t('users.lock.aboutAria', { name: u.username }), note, 'us-tip') : null;
  // The reason some actions are greyed (your own account, the last administrator): one (i) tip on the row (US-2)
  const whyTip = lim.why ? infoTip(`us-why-${u.id}`, t('users.why.aboutAria', { name: u.username }), lim.why, 'us-tip') : null;
  const tags = [
    // The default tags (plain user, normal) are not drawn: only what differs from the usual (US-2)
    u.role === 'admin' ? h('span', { class: 'tag b' }, roleName(u.role)) : null,
    lim.self ? h('span', { class: 'tag k' }, t('users.you')) : null,
    u.disabled ? h('span', { class: 'tag r' }, t('users.status.disabled')) : null,
    locked && u.lock?.until
      ? h('span', { class: 'tag r us-lock', id: `us-lock-${u.id}` }, t('users.status.lockedUntil', { at: lockUntilLabel(u.lock.until, u.lock.retryAfter), n: u.lock.round }))
      : null,
    // Past round 1 without a running lock: how many tries are left before the next (longer) lock
    lock.state === 'tries' ? h('span', { class: 'tag k' }, tn('users.status.tries', lock.triesLeft)) : null,
    lockTip?.el ?? null,
    !u.disabled && u.mustChangePassword ? h('span', { class: 'tag k' }, t('users.status.mustChange')) : null,
    // v0.6.5: a plain user who may import and manage their own question banks (administrators always may)
    u.role === 'user' && u.canManageBanks ? h('span', { class: 'tag b' }, t('users.tag.manager')) : null,
    rowBadge(u),
    // US-9: a paused AI+ function links to the users tab of AI settings (the two-switch model stays)
    aiAvailable && u.role === 'user' && u.ai && u.aiSuspended
      ? h('a', { class: 'tag k us-ai-paused', id: `us-ai-paused-${u.id}`, href: `#/ai-settings?tab=users&user=${u.id}` }, t('users.ai.pausedGo'))
      : null,
    whyTip?.el ?? null,
  ];
  // The facts in one line (US-2): the last sign-in and the number of study records
  const facts = [
    u.lastLoginAt ? t('users.meta.lastLogin', { at: shortAt(u.lastLoginAt) }) : t('users.meta.never'),
    tn('users.meta.recordsShort', u.records),
  ];
  const unlockBtn = lock.unlock
    ? iconButton({
      icon: 'unlock', label: t(locked ? 'users.act.unlockAria' : 'users.act.unlockTriesAria', { name: u.username }),
      describedBy: lockTip ? noteId : undefined, onClick: () => void unlock(u, locked),
    })
    : null;
  if (unlockBtn) {
    unlockBtn.id = `us-unlock-${u.id}`;
    unlockBtn.disabled = blocked;
  }
  const toggleRule = lim.self || lim.lastAdmin ? lim.why : null;
  return h('li', {
    class: 'us-row' + (u.disabled ? ' off' : '') + (u.id === newRowId && !newRowShown ? ' us-new' : ''),
    'aria-busy': busy ? 'true' : null, 'data-name': u.username.toLowerCase(),
  },
  icon(ICONS.user, 'ico us-ico'),
  h('div', { class: 'us-main' },
    h('div', { class: 'us-line' },
      h('h3', { class: 'us-name', id: `us-name-${u.id}`, tabindex: '-1', title: u.username }, u.username),
      h('span', { class: 'us-tags' }, tags)),
    h('p', { class: 'us-meta' }, facts.map((m, i) => [
      i ? ' ' : null,
      h('span', { class: 'us-nw' }, m, i < facts.length - 1 ? h('span', { class: 'us-dot', 'aria-hidden': 'true' }, '·') : null),
    ])),
    bankLine(u, blocked)),
  h('div', { class: 'us-acts' },
    aiSwitch(u, blocked),
    h('div', { class: 'us-icons' },
      busy ? h('span', { class: 'us-busy', role: 'status' }, t('users.act.busy')) : null,
      act(`us-edit-${u.id}`, 'pencil', t('users.act.editAria', { name: u.username }), blocked, null, () => void editUser(u, lim)),
      act(`us-reset-${u.id}`, 'keyRound', t('users.act.resetAria', { name: u.username }), blocked, lim.self ? lim.why : null, () => void resetPassword(u)),
      u.disabled
        ? act(`us-toggle-${u.id}`, 'userOn', t('users.act.enableAria', { name: u.username }), blocked, null, () => void toggleDisabled(u))
        : act(`us-toggle-${u.id}`, 'userOff', t('users.act.disableAria', { name: u.username }), blocked, toggleRule, () => void toggleDisabled(u)),
      unlockBtn,
      act(`us-delete-${u.id}`, 'trash', t('users.act.deleteAria', { name: u.username }), blocked, toggleRule, () => void removeUser(u), true))));
}

function addButton(list: UsersList['users'] | null): HTMLElement {
  const b = iconButton({
    icon: 'plus', label: t('users.add'), showLabel: true, onClick: () => {
      add.open = true;
      focusNext = 'us-add-name';
      render();
    },
  });
  b.id = 'us-add-btn';
  b.classList.add('us-addbtn');
  b.disabled = !list;
  return b;
}

function listPanel(): HTMLElement {
  const list = users;
  const lead = infoTip('us-lead-tip', t('users.lead.aria'), t('users.lead'), 'us-tip');
  const head = h('div', { class: 'us-head' },
    h('h2', { id: 'us-list-h', tabindex: '-1' }, t('users.list.title'), list ? h('span', { class: 'us-count' }, tn('users.count', list.length)) : null, lead.el),
    add.open ? null : addButton(list));

  let body: Child;
  if (!list) {
    body = loadError
      ? h('div', { class: 'err', role: 'alert' },
        h('p', null, t('users.err.load', { reason: errorText(loadError) })),
        h('div', { class: 'acts' }, h('button', { class: 'btn', type: 'button', onclick: () => void load() }, t('common.retry'))))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  } else {
    const rows = h('ul', { class: 'us-rows' }, list.map((u) => userRow(u, list)));
    const none = h('p', { class: 'dim us-nomatch', role: 'status', hidden: true }, t('users.search.none'));
    // US-11: a search field above a long list; it hides rows in place (no redraw, the field keeps its focus)
    const apply = (): void => {
      const keep = new Set(filterUsers(list, searchText).map((u) => u.username.toLowerCase()));
      let shown = 0;
      for (const li of Array.from(rows.children) as HTMLElement[]) {
        const on = keep.has(li.dataset.name ?? '');
        li.hidden = !on;
        if (on) shown += 1;
      }
      none.hidden = shown > 0;
    };
    const search = list.length > SEARCH_FROM
      ? h('input', {
        type: 'search', class: 'us-search', id: 'us-search', value: searchText, placeholder: t('users.search'), 'aria-label': t('users.search'),
        autocomplete: 'off', spellcheck: 'false',
        oninput: (e: Event) => {
          searchText = (e.target as HTMLInputElement).value;
          apply();
        },
      })
      : null;
    if (search && searchText) queueMicrotask(apply);
    body = [search, rows, none];
  }
  return h('section', { class: 'panel us-list', 'aria-labelledby': 'us-list-h', 'aria-busy': loading && !list ? 'true' : null },
    head,
    add.open ? addForm() : null,
    actionError ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('users.err.action', { reason: errorText(actionError) }))) : null,
    body);
}

/**
 * One short line while the AI+ switches are hidden (AI+ not available): it says why. Only administrators reach this page,
 * so it may name ai-service. Nothing before the first list answer (aiAvailable is not known yet) or while the list failed.
 */
function aiHiddenNote(): HTMLElement | null {
  if (aiAvailable || !users || loadError) return null;
  const disconnected = aiStatus().state === 'unavailable';
  return h('p', { class: 'us-ainote', id: 'us-ainote', role: 'note' }, t(disconnected ? 'users.ai.hiddenDisconnected' : 'users.ai.hidden'));
}

/** US-8: scroll the new user's row into view, highlight it for 2 s (CSS) and focus its name; once per created user */
function showNewRow(root: HTMLElement): void {
  const id = newRowId;
  if (id == null || newRowShown) return;
  newRowShown = true;
  queueMicrotask(() => {
    const name = root.isConnected ? document.getElementById(`us-name-${id}`) : null;
    if (!name) return;
    const topH = parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--top-h')) || 0;
    const y = name.getBoundingClientRect().top + window.scrollY - topH - 72;
    scrollToY(Math.max(0, y), true);
    name.focus({ preventScroll: true });
    window.setTimeout(() => {
      document.querySelector('.us-new')?.classList.remove('us-new');
      if (newRowId === id) newRowId = null;
    }, 2000);
  });
}

export function renderUsers(): HTMLElement {
  if (wanted && !loading) void load();
  startTicker();
  startPoller();
  const root = h('div', { class: 'page page-users' },
    pageHead(t('users.title')),
    aiHiddenNote(),
    listPanel());
  focusAfter(root);
  showNewRow(root);
  return root;
}
