// AI settings page, Users tab: every administrator and every user with the AI+ feature: badge, role, whether the feature is
// paused (Pause / Resume, no confirmation; a paused user stays listed and keeps their data; this page never changes the AI+
// feature of User management), counts, the request counts against the caps, the per-user feature boxes, the free-mode switch of
// an administrator, and the read-only viewer of the user's chats and memories. Below 900 px each user is a small card. A box of
// a feature that is switched off for everyone is shown disabled with the tip "Switched off for everyone" (v1.9: the global
// switches apply to administrators too; an administrator's boxes stay ticked and disabled, per-user boxes never limit them).
import { fetchAiUsers, resumeAiUser, setAiAdminFree, setAiUserFeatures, suspendAiUser } from '../../api/ai.ts';
import { aiBadge } from '../../components/ai-badge.ts';
import { iconButton } from '../../components/icon-button.ts';
import { pagedTable, type PtColumn } from '../../components/paged-table.ts';
import { miniIcon } from '../../components/mini-icons.ts';
import { roleName } from '../../components/shell.ts';
import { ALL_ON, FEATURE_GROUPS, featureLabelKey, itemOn, offList, readFeatures, withItem, type FeatureKey } from '../../domain/ai-features.ts';
import { badgeState, serviceReason } from '../../domain/badge-state.ts';
import { blockTag, callsFigures, jobTime } from '../../domain/ai-settings.ts';
import { errorText } from '../../i18n/errors.ts';
import { localeTag, t, uiLang, type MsgKey } from '../../i18n/index.ts';
import { waitText } from '../../i18n/wait.ts';
import { refreshAi } from '../../state/actions.ts';
import { setAiFreeMode } from '../../state/ai.ts';
import { S } from '../../state/store.ts';
import type { AiAdminUserRow, AiSettings } from '../../types/api.ts';
import { h, type Child } from '../../utils/dom.ts';
import { toast } from '../../utils/toast.ts';
import { aiDown } from './status.ts';
import { A, num, redraw, when } from './state.ts';
import { loadUsage } from './usage.ts';
import { openViewer } from './viewer.ts';

export async function loadUsers(): Promise<void> {
  try {
    A.users = await fetchAiUsers();
    A.usersError = null;
  } catch (e) {
    if (aiDown(e)) return;
    A.usersError = e;
  }
  redraw();
}

/**
 * Pauses or resumes a user's AI+ feature (round 2; no confirmation; the API calls are still named suspend / resume). It never
 * changes the AI+ feature of User management: a paused user stays listed here, keeps reading and deleting their own chats
 * and memories, and their running answer stops at once. A success toast only after the server confirmed it.
 */
async function setFunction(u: AiAdminUserRow, suspend: boolean): Promise<void> {
  if (A.fnBusy != null) return;
  A.fnBusy = u.id;
  redraw();
  let down = false;
  try {
    await (suspend ? suspendAiUser(u.id) : resumeAiUser(u.id));
    toast(t(suspend ? 'ai.set.fn.suspendDone' : 'ai.set.fn.resumeDone', { name: u.username }));
  } catch (e) {
    down = aiDown(e);
    if (!down) toast(errorText(e), 6000);
  }
  A.fnBusy = null;
  if (down) return;
  await loadUsers();
  void refreshAi(true);
  document.getElementById(`ais-fn-${u.id}`)?.focus();
}

/** The badge of a listed user (one rule for every badge, domain/badge-state.ts): this page is shown only while ai-service is
 *  connected, so AI+ is available exactly when the AI+ feature is on */
function badgeFor(u: Pick<AiAdminUserRow, 'role' | 'ai' | 'suspended' | 'features'>, enabled: boolean): HTMLElement | null {
  const why = serviceReason(S.aiService);
  const off = u.role === 'admin' ? [] : offList(readFeatures(u.features));
  return aiBadge(badgeState({
    role: u.role, service: u.ai, suspended: u.suspended === true, available: enabled, ...(why ? { unavailableReason: why } : {}),
  }), { admin: true, off });
}

/** The text of a user's cap tag when tapped: who, which cap, and when the user can use AI again */
function blockedText(name: string, q: { cap: 'minute' | 'hour' | 'day'; limit: number; retryAt: number }): string {
  const nowSec = Date.now() / 1000;
  const cap = t(q.cap === 'minute' ? 'ai.cap.minute' : q.cap === 'hour' ? 'ai.cap.hour' : 'ai.cap.day', { n: num(q.limit) });
  const left = Math.max(1, Math.ceil(q.retryAt - nowSec));
  const when = q.cap === 'day'
    ? t('ai.set.calls.whenDay')
    : left < 60 ? t('ai.set.calls.whenSoon') : t('ai.set.calls.whenWait', { wait: waitText(left), time: jobTime(q.retryAt) });
  return t('ai.set.calls.blocked', { name, cap, when });
}

/**
 * C-7: "AI requests 1 min 2/5 · 1 h 7/20 · today 37/100" and, while a cap blocks, a warn tag ("Used up · back at 14:05" /
 * "Used up today") that shows the cap text when tapped. The label in front is only for the card layout (the table has the
 * column name).
 */
function callsCell(u: AiAdminUserRow): HTMLElement {
  const td = h('td', { role: 'cell', class: 'ais-calls', 'data-l': t('ai.set.col.calls') });
  const fig = u.exists ? callsFigures(u.calls, A.limits?.limits) : null;
  if (!fig || !u.calls) return td;
  td.append(h('span', { class: 'ais-callsline' },
    h('span', { class: 'ais-calls-lbl' }, `${t('ai.set.calls.lbl')} `),
    ...[t('ai.set.calls.min', { v: fig.minute }), t('ai.set.calls.hour', { v: fig.hour }), t('ai.set.calls.today', { v: fig.day })]
      .flatMap((x, i): Child[] => [i ? ' · ' : '', h('span', { class: 'ais-calls-fig' }, x)])));
  const tag = blockTag(u.calls);
  const q = u.calls.blocked;
  if (tag && q) {
    td.append(h('button', {
      type: 'button', class: 'ais-blocktag',
      onclick: () => toast(blockedText(u.username, q), 7000),
    }, t(tag.kind === 'today' ? 'ai.set.calls.tagToday' : 'ai.set.calls.tagTimed', { time: jobTime(tag.retryAt) })));
  }
  return td;
}

/** A feature switched off for everyone (the box pairs hint + analysis: off when either global switch is off) */
const globalOff = (k: FeatureKey): boolean => A.settings != null && !itemOn(A.settings.featureSwitches, k);

/** The feature boxes of one user (FP-4): two groups, each on one row; administrators' are ticked and disabled. AI hint and AI
 *  analysis are one box that writes both keys. */
function featuresLine(u: AiAdminUserRow): HTMLElement {
  const admin = u.role === 'admin';
  const f = admin ? ALL_ON : readFeatures(u.features);
  const busy = A.featBusy === u.id;
  const box = (k: FeatureKey): HTMLElement => {
    const full = t(featureLabelKey(k) as MsgKey);
    const input = h('input', {
      type: 'checkbox', class: 'ais-fbox', id: `ais-feat-${u.id}-${k}`, checked: itemOn(f, k), disabled: admin || busy || globalOff(k),
      'aria-label': `${full}: ${u.username}`, onchange: () => void setFeature(u, k, input.checked),
    });
    return h('label', { class: 'ais-fl', title: !admin && globalOff(k) ? t('ai.set.switches.globalOff') : full }, input, h('span', null, t(featureLabelKey(k, true) as MsgKey)));
  };
  const colon = uiLang() === 'en' ? ':' : '：';
  const err = A.featErr.get(u.id);
  const row = h('div', { class: 'ais-feats', role: 'group', 'aria-label': `${t('ai.set.feat.title')}: ${u.username}`, title: admin ? t('ai.set.feat.adminAll') : null },
    h('span', { class: 'ais-feat-t' }, t('ai.set.feat.title')),
    h('div', { class: 'ais-fg' }, FEATURE_GROUPS.main.map(box)),
    h('div', { class: 'ais-fg ais-fg-banks' }, h('span', { class: 'ais-fg-l' }, t('ai.feature.group.banks') + colon), FEATURE_GROUPS.banks.map(box)),
    admin ? h('span', { class: 'sr-only' }, t('ai.set.feat.adminAll')) : null,
    admin ? freeSwitch(u) : null,
    err ? h('p', { class: 'ais-ferr ais-feat-err', role: 'alert' }, err) : null);
  tidyFeatureGroups(row);
  return row;
}

/** v2.9: a group of boxes stays on ONE row when it fits; when the font is wider than the room (Android phones, Japanese) it
 *  becomes a two-column grid instead of a ragged wrap. Measured, so it follows the real font and width. */
function tidyFeatureGroups(row: HTMLElement): void {
  if (typeof ResizeObserver === 'undefined') return;
  const fit = (): void => {
    if (!row.isConnected || row.getClientRects().length === 0) return;
    for (const g of row.querySelectorAll<HTMLElement>('.ais-fg:not(.ais-fg-free)')) {
      g.classList.remove('ais-fg-grid');
      const tops = new Set(Array.from(g.querySelectorAll('label'), (l) => Math.round(l.getBoundingClientRect().top)));
      if (tops.size > 1) g.classList.add('ais-fg-grid');
    }
  };
  let last = 0;
  new ResizeObserver(() => {
    const w = Math.round(row.getBoundingClientRect().width);
    if (w === last) return;
    last = w;
    fit();
  }).observe(row);
  void document.fonts?.ready.then(fit);
}

/** A-OWNER-21: an administrator row's AI Chat free mode switch (ordinary users never get one); saved at once */
function freeSwitch(u: AiAdminUserRow): HTMLElement {
  const on = u.freeMode === true;
  const input = h('input', {
    type: 'checkbox', role: 'switch', class: 'ais-fbox', id: `ais-free-${u.id}`, checked: on, disabled: A.featBusy != null,
    'aria-label': `${t('ai.set.free')}: ${u.username}`, onchange: () => void setFree(u, input.checked),
  });
  return h('div', { class: 'ais-fg ais-fg-free' }, h('label', { class: 'ais-fl', title: t('ai.set.free.tip') }, input, h('span', null, t('ai.set.free'))));
}

async function setFree(u: AiAdminUserRow, on: boolean): Promise<void> {
  if (A.featBusy != null) return;
  A.featBusy = u.id;
  A.featErr.delete(u.id);
  redraw();
  try {
    const row = await setAiAdminFree(u.id, on);
    if (A.users) A.users = A.users.map((x) => (x.id === u.id ? { ...x, ...row } : x));
    A.featSaved = t('ai.set.free.saved', { name: u.username, state: t(row.freeMode ? 'settings.on' : 'settings.off') });
    if (u.username === S.user?.username) setAiFreeMode(row.freeMode === true);
  } catch (e) {
    if (!aiDown(e)) A.featErr.set(u.id, t('ai.set.feat.failed', { reason: errorText(e) }));
  }
  A.featBusy = null;
  redraw();
  document.getElementById(`ais-free-${u.id}`)?.focus();
}

/** One box changed: all six values go to the server at once; a failure puts the box back and shows the error under the line */
async function setFeature(u: AiAdminUserRow, key: FeatureKey, on: boolean): Promise<void> {
  if (A.featBusy != null) return;
  const next = withItem(readFeatures(u.features), key, on);
  A.featBusy = u.id;
  A.featErr.delete(u.id);
  redraw();
  try {
    const row = await setAiUserFeatures(u.id, next);
    if (A.users) A.users = A.users.map((x) => (x.id === u.id ? { ...x, ...row } : x));
    A.featSaved = t('ai.set.feat.saved', { name: u.username });
    void refreshAi(true);
  } catch (e) {
    if (!aiDown(e)) A.featErr.set(u.id, t('ai.set.feat.failed', { reason: errorText(e) }));
  }
  A.featBusy = null;
  redraw();
  document.getElementById(`ais-feat-${u.id}-${key}`)?.focus();
}

/**
 * The users with AI+ (AS-2). Wider than 900 px a table (the name wraps, the role tag next to a short "Always on" tag, the
 * calls line, a narrow last column of icon buttons); up to 900 px every row is a card of three lines: name, badge and the
 * eye; role, state and the pause / play button; the counts line and the calls line. The cells of the card's counts line are
 * the same cells as the table's counts, so the stylesheet shows one or the other.
 */
export function usersSection(s: AiSettings): HTMLElement {
  let body: Child;
  if (!A.users) {
    body = A.usersError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.usersErr', { reason: errorText(A.usersError) })))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  } else if (!A.users.length) {
    body = h('p', { class: 'ais-note' }, t('ai.set.user.noUsers'));
  } else {
    // Explicit table roles: up to 900 px the CSS turns the rows into cards (display: grid), which would drop the table
    // semantics in some browsers. `data-l` is the column name shown in front of a value inside a card.
    const cell = (label: MsgKey, cls: string, ...kids: Child[]): HTMLElement => h('td', { role: 'cell', class: cls || null, 'data-l': t(label) }, ...kids);
    const row = (u: AiAdminUserRow): HTMLElement => {
      const off = u.suspended === true;
      const view = iconButton({ icon: 'eye', label: t('ai.set.user.viewAria', { name: u.username }), onClick: () => void openViewer(u, () => {
        void loadUsers();
        void loadUsage();
      }) });
      view.id = `ais-view-${u.id}`;
      view.classList.add('ais-bview');
      // Administrators are never paused; a deleted user has nothing to pause
      let fn: HTMLButtonElement | null = null;
      if (u.exists && u.role !== 'admin') {
        fn = iconButton({
          icon: off ? 'playCircle' : 'pauseCircle', label: t(off ? 'ai.set.fn.resumeAria' : 'ai.set.fn.suspendAria', { name: u.username }),
          onClick: () => void setFunction(u, !off),
        });
        fn.id = `ais-fn-${u.id}`;
        fn.classList.add('ais-bfn');
        fn.disabled = A.fnBusy != null;
      }
      const role = u.exists
        ? [h('span', { class: 'tag k' }, roleName(u.role)),
          u.role === 'admin' ? h('span', { class: 'tag g', title: t('ai.set.user.adminAlways') }, t('ai.set.user.alwaysTag')) : null]
        : [];
      const state = u.exists && u.role !== 'admin'
        ? h('span', { class: 'ais-fn-state ' + (off ? 'off' : 'on') }, t(off ? 'ai.set.fn.suspended' : 'ai.set.fn.active'))
        : null;
      return h('tr', { role: 'row', class: u.exists ? null : 'gone' },
        h('th', { scope: 'row', role: 'rowheader', class: 'ais-user' }, h('span', { class: 'ais-uname', title: u.username }, u.username)),
        cell('ai.set.col.badge', 'ais-badge', u.exists ? badgeFor(u, s.enabled) : h('span', { class: 'dim' }, t('ai.set.user.deleted'))),
        cell('ai.set.col.role', 'ais-role', role),
        cell('ai.set.col.function', 'ais-state', state),
        cell('ai.set.user.chats', 'n ais-w', num(u.chats)), cell('ai.set.user.memories', 'n ais-w', num(u.memories)),
        cell('ai.set.user.lastUsed', 'ais-when ais-w', when(u.lastUsedAt)),
        h('td', { role: 'cell', class: 'ais-cline' }, t('ai.set.user.line', { c: num(u.chats), m: num(u.memories), last: when(u.lastUsedAt) })),
        callsCell(u),
        h('td', { role: 'cell', class: 'ais-acts' }, view, fn));
    };
    const c = (key: string, label: MsgKey, cls = '', text = false): PtColumn => ({ key, label: t(label), cls, text });
    const plain = (key: string, label: MsgKey, cls = ''): PtColumn => ({ key, label: t(label), cls, plain: true });
    const numCmp = (f: (r: AiAdminUserRow) => number) => (a: AiAdminUserRow, b: AiAdminUserRow): number => f(a) - f(b);
    body = pagedTable<AiAdminUserRow>({
      id: 'ai-users', rows: A.users, name: (u) => u.username, cardsAt: 900, roles: true, tableClass: 'ais-table ais-users',
      defaultSort: { key: 'user', dir: 'asc' },
      // The name and the AI+ badge in columns of their own, so names of any length line up (owner's request)
      columns: [
        c('user', 'ai.set.col.user', '', true), plain('badge', 'ai.set.col.badge'), c('role', 'ai.set.col.role', '', true), plain('fn', 'ai.set.col.function'),
        c('chats', 'ai.set.user.chats', 'n ais-w'), c('memories', 'ai.set.user.memories', 'n ais-w'), c('last', 'ai.set.user.lastUsed', 'ais-w'),
        { key: 'line', label: `${t('ai.set.user.chats')} · ${t('ai.set.user.memories')} · ${t('ai.set.user.lastUsed')}`, cls: 'ais-cline', plain: true, srOnly: true },
        plain('calls', 'ai.set.col.calls'),
        { key: 'acts', label: t('ai.set.user.view'), cls: 'ais-acts', plain: true, srOnly: true },
      ],
      cmp: {
        user: (a, b) => a.username.localeCompare(b.username, localeTag()),
        role: (a, b) => roleName(a.role).localeCompare(roleName(b.role), localeTag()),
        chats: numCmp((u) => u.chats), memories: numCmp((u) => u.memories), last: numCmp((u) => u.lastUsedAt ?? 0),
      },
      renderRow: (u) => [row(u), u.exists
        ? h('tr', { role: 'row', class: 'ais-featrow' }, h('td', { role: 'cell', colspan: '10' }, featuresLine(u)))
        : null],
    });
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-users-h' },
    h('h2', { id: 'ais-users-h' }, t('ai.set.sec.users')),
    // the polite text after a features change (no toast, FP-4)
    h('p', { class: 'saved-note ais-feat-saved', role: 'status', 'aria-live': 'polite' }, ...(A.featSaved ? [miniIcon('check', 'saved-ic'), A.featSaved] : [])),
    body);
}
