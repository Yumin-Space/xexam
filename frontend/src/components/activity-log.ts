// Activity tab of the log page (v1.3, administrators): who did what, from where. Filters (user, action group, result), the
// entries (a table-like grid on wide screens, one card per entry on phones, a tap opens the full entry), "load earlier",
// auto-refresh every 10 s while the tab is visible (pausable), and a settings popover (days to keep, visits of normal users).
// The IP column shows only while the sign-in info eye (User management) is open; the location is never made up.
// The component keeps its filters and entries between redraws of the page and draws its own list, so a refresh never
// redraws the toolbar (a select keeps its focus).
import { fetchActivity, saveActivitySettings } from '../api/activity.ts';
import { miniSwitch } from './mini-switch.ts';
import { iconButton } from './icon-button.ts';
import { ACTIVITY_GROUPS, ACTIVITY_REFRESH_MS, actionFilter, detailText, deviceLabel, looksLikeProxy, mergeNewer, userFilter } from '../domain/activity.ts';
import { countryNameIn, placeText } from '../domain/geo.ts';
import { errorText } from '../i18n/errors.ts';
import { localizeServerMessage } from '../i18n/server-messages.ts';
import { hasKey, t, uiLang, type MsgKey } from '../i18n/index.ts';
import type { ActivityEntry, ActivityPage, ActivitySettings } from '../types/api.ts';
import { toast } from '../utils/toast.ts';
import { h } from '../utils/dom.ts';

// ---------------------------------------------------------------- state (kept while the app runs)

let entries: ActivityEntry[] = [];
let next: number | null = null;
let newest: number | null = null;
let facets: { id: number; username: string; deleted: boolean }[] = [];
let settings: ActivitySettings | null = null;
let loaded = false;
let loading = false;
let readError: unknown = null;
let paused = false;
let settingsOpen = false;
let settingsBusy = false;
let proxyHint = false;
const filters = { user: '', action: '', result: '' as '' | 'ok' | 'fail' };
const openIds = new Set<number>();
/** Bumped by every first read (new filter, refresh button): the answer of an older read is dropped */
let epoch = 0;
/** Bumped by every new draw and by `stop()`: an older view stops refreshing */
let viewId = 0;
let timer: number | undefined;

const key = (k: string): MsgKey => k as MsgKey;
const lbl = (k: string, fallback: string): string => (hasKey(uiLang(), k) ? t(key(k)) : fallback);

const actionLabel = (code: string): string => lbl(`act.a.${code}`, code);
function reasonText(r: string): string {
  return hasKey(uiLang(), `act.reason.${r}`) ? t(key(`act.reason.${r}`)) : localizeServerMessage(r, uiLang());
}
function actorText(e: ActivityEntry): string {
  if (e.actor === 'system') return t('act.actor.system');
  if (e.actor === 'anon' && !e.username) return t('act.actor.anon');
  return (e.username ?? t('act.actor.anon')) + (e.deleted ? ` (${t('act.deleted')})` : '');
}
function objectText(e: ActivityEntry): string {
  const obj = e.object ? (e.action === 'page.view' && hasKey(uiLang(), `nav.${e.object}`) ? t(key(`nav.${e.object}`)) : e.object) : '';
  const d = detailText(e.detail);
  return [obj, d].filter(Boolean).join(' · ');
}
const ipOn = (): boolean => settings?.signinInfo === true;

// ---------------------------------------------------------------- reading

function query(extra: { before?: number; after?: number; facets?: boolean }): Parameters<typeof fetchActivity>[0] {
  const a = actionFilter(filters.action);
  const u = userFilter(filters.user);
  return { ...extra, ...(u !== undefined ? { user: u } : {}), ...(a.group ? { group: a.group } : {}), ...(a.action ? { action: a.action } : {}), ...(filters.result ? { result: filters.result } : {}) };
}

function take(a: ActivityPage): void {
  settings = a.settings;
  proxyHint = looksLikeProxy(a.entries.map((e) => e.ip));
  if (a.facets) facets = a.facets.users;
}

async function readFirst(redraw: () => void): Promise<void> {
  const mine = ++epoch;
  loading = true;
  readError = null;
  redraw();
  try {
    const a = await fetchActivity(query({ facets: true }));
    if (mine !== epoch) return;
    take(a);
    entries = a.entries;
    next = a.next;
    newest = a.newest ?? entries[0]?.id ?? null;
    loaded = true;
  } catch (e) {
    if (mine !== epoch) return;
    readError = e;
  } finally {
    if (mine === epoch) loading = false;
  }
  redraw();
}

async function readOlder(redraw: () => void): Promise<void> {
  if (loading || next == null) return;
  const mine = epoch;
  loading = true;
  redraw();
  try {
    const a = await fetchActivity(query({ before: next }));
    if (mine !== epoch) return;
    take(a);
    entries = [...entries, ...a.entries.filter((e) => !entries.some((x) => x.id === e.id))];
    next = a.next;
  } catch (e) {
    if (mine !== epoch) return;
    readError = e;
  } finally {
    if (mine === epoch) loading = false;
  }
  redraw();
}

async function readNewer(redraw: () => void): Promise<void> {
  if (loading || newest == null) return;
  const mine = epoch;
  try {
    const a = await fetchActivity(query({ after: newest }));
    if (mine !== epoch) return;
    take(a);
    if (a.entries.length) {
      entries = mergeNewer(entries, a.entries);
      newest = a.newest ?? entries[0]?.id ?? newest;
      redraw();
    }
  } catch {
    // the next round tries again; the list on screen stays
  }
}

// ---------------------------------------------------------------- drawing

function tag(e: ActivityEntry): HTMLElement {
  return h('span', { class: `act-tag ${e.result === 'ok' ? 'ok' : 'fail'}` }, t(e.result === 'ok' ? 'act.result.ok' : 'act.result.fail'));
}

function placeOf(e: ActivityEntry): string | null {
  return placeText(e.location, countryNameIn(uiLang()));
}

function timeText(e: ActivityEntry): string {
  return e.count > 1 && e.lastTime && e.lastTime !== e.time ? `${e.time} → ${e.lastTime.slice(11)}` : e.time;
}

function row(e: ActivityEntry, redraw: () => void): HTMLElement {
  const open = openIds.has(e.id);
  const dev = deviceLabel(e.ua);
  const ipLine = ipOn() ? [e.ip ?? '—', dev ?? '—'].join(' · ') : (dev ?? '—');
  const main = h('button', {
    type: 'button', class: 'act-main', 'aria-expanded': String(open),
    onclick: () => {
      if (openIds.has(e.id)) openIds.delete(e.id);
      else openIds.add(e.id);
      redraw();
    },
  },
  h('span', { class: 'act-c act-time' }, timeText(e), e.count > 1 ? h('span', { class: 'act-n' }, t('act.merged', { n: e.count })) : null),
  h('span', { class: 'act-c act-user' }, actorText(e)),
  h('span', { class: 'act-c act-act' }, actionLabel(e.action)),
  h('span', { class: 'act-c act-obj' }, objectText(e)),
  h('span', { class: 'act-c act-res' }, tag(e), e.result === 'fail' && e.reason ? h('span', { class: 'act-why' }, reasonText(e.reason)) : null),
  h('span', { class: 'act-c act-dev' }, ipLine));
  const li = h('li', { class: `act-row${open ? ' open' : ''}` }, main);
  if (open) {
    const place = placeOf(e);
    li.append(h('dl', { class: 'act-detail', 'aria-label': t('act.detail.title') },
      h('dt', null, t('act.detail.ua')), h('dd', null, e.ua ?? '—'),
      ipOn() ? h('dt', null, t('act.col.ip')) : null, ipOn() ? h('dd', null, e.ip ?? '—') : null,
      ipOn() ? h('dt', null, t('act.detail.place')) : null, ipOn() ? h('dd', null, place ?? t('users.fact.noPlace')) : null,
      e.detail ? h('dt', null, t('act.col.object')) : null, e.detail ? h('dd', null, objectText(e)) : null));
  }
  return li;
}

function headRow(): HTMLElement {
  return h('div', { class: 'act-head', role: 'presentation' },
    ...(['time', 'user', 'action', 'object', 'result'] as const).map((c) => h('span', { class: `act-c act-${c === 'action' ? 'act' : c === 'object' ? 'obj' : c === 'result' ? 'res' : c}` }, t(key(`act.col.${c}`)))),
    h('span', { class: 'act-c act-dev' }, t(ipOn() ? 'act.col.ip' : 'act.col.device')));
}

function select(id: string, label: string, value: string, options: [string, string][], onChange: (v: string) => void): HTMLElement {
  const sel = h('select', { id, class: 'act-sel', 'aria-label': label, onchange: () => onChange(sel.value) },
    options.map(([v, text]) => h('option', { value: v, selected: v === value }, text)));
  return sel;
}

function settingsPanel(redraw: () => void): HTMLElement | null {
  if (!settingsOpen || !settings) return null;
  const s = settings;
  const days = h('input', {
    id: 'act-days', type: 'number', min: '7', max: '3650', value: String(s.retentionDays), class: 'act-days', disabled: settingsBusy,
    'aria-describedby': 'act-days-h',
  });
  const apply = async (patch: { retentionDays?: number; userVisits?: boolean }): Promise<void> => {
    settingsBusy = true;
    redraw();
    try {
      settings = { ...s, ...(await saveActivitySettings(patch)) };
      toast(t('act.settings.saved'));
    } catch (e) {
      toast(errorText(e));
    } finally {
      settingsBusy = false;
    }
    redraw();
  };
  const save = h('button', {
    type: 'button', class: 'btn', disabled: settingsBusy,
    onclick: () => {
      const n = Math.trunc(Number(days.value));
      if (!Number.isFinite(n) || n < 7 || n > 3650) {
        toast(t('act.settings.days.hint'));
        return;
      }
      void apply({ retentionDays: n });
    },
  }, t('users.edit.save'));
  return h('div', { class: 'act-pop panel', role: 'group', 'aria-label': t('act.settings') },
    h('div', { class: 'act-pop-row' },
      h('label', { for: 'act-days', class: 'fld-l' }, t('act.settings.days')), days, save),
    h('p', { class: 'fld-hint', id: 'act-days-h' }, t('act.settings.days.hint')),
    h('div', { class: 'act-pop-row' },
      h('span', { class: 'fld-l', id: 'act-visits-l' }, t('act.settings.visits')),
      miniSwitch({ id: 'act-visits', checked: s.userVisits, labelledBy: 'act-visits-l', describedBy: 'act-visits-h', disabled: settingsBusy, onToggle: (on) => void apply({ userVisits: on }) })),
    h('p', { class: 'fld-hint', id: 'act-visits-h' }, t('act.settings.visits.hint')));
}

export interface ActivityView {
  el: HTMLElement;
  stop(): void;
}

/** Draws the tab and starts its refresh; `stop()` ends the refresh (the page calls it when the tab or the page is left) */
export function activityLog(): ActivityView {
  const mine = ++viewId;
  const toolbar = h('div', { class: 'act-bar' });
  const body = h('div', { class: 'act-body' });
  const root = h('section', { class: 'panel act-panel', 'aria-label': t('logs.tab.activity') }, toolbar, body);

  const redrawBar = (): void => {
    const users: [string, string][] = [
      ['', t('act.filter.user.all')], ['system', t('act.filter.user.system')], ['anon', t('act.filter.user.anon')],
      ...facets.map((u): [string, string] => [String(u.id), u.username + (u.deleted ? ` (${t('act.deleted')})` : '')]),
    ];
    const actions: [string, string][] = [
      ['', t('act.filter.action.all')],
      ...ACTIVITY_GROUPS.map((g): [string, string] => [`g:${g}`, t(key(`act.group.${g}`))]),
    ];
    const results: Array<['' | 'ok' | 'fail', MsgKey]> = [['', 'act.filter.result.all'], ['ok', 'act.filter.result.ok'], ['fail', 'act.filter.result.fail']];
    const refilter = (): void => void readFirst(redraw);
    toolbar.replaceChildren(
      select('act-user', t('act.filter.user'), filters.user, users, (v) => {
        filters.user = v;
        refilter();
      }),
      select('act-action', t('act.filter.action'), filters.action, actions, (v) => {
        filters.action = v;
        refilter();
      }),
      h('div', { class: 'seg act-seg', role: 'group', 'aria-label': t('act.col.result') }, results.map(([v, k]) =>
        h('button', {
          type: 'button', 'aria-pressed': String(filters.result === v),
          onclick: () => {
            if (filters.result === v) return;
            filters.result = v;
            refilter();
          },
        }, t(k)))),
      h('span', { class: 'act-tools' },
        iconButton({ icon: 'refresh', label: t('act.refresh'), focusKey: 'act-refresh', onClick: () => void readFirst(redraw) }),
        iconButton({
          icon: paused ? 'play' : 'pause', label: t(paused ? 'act.resume' : 'act.pause'), focusKey: 'act-pause',
          onClick: () => {
            paused = !paused;
            schedule();
            redrawBar();
          },
        }),
        iconButton({
          icon: 'more', label: t('act.settings'), focusKey: 'act-settings',
          onClick: () => {
            settingsOpen = !settingsOpen;
            redraw();
          },
        })));
  };

  const redrawBody = (): void => {
    const kids: Array<HTMLElement | null> = [settingsPanel(redraw)];
    if (settings) kids.push(h('p', { class: 'fld-hint act-note' }, t('act.note', { n: settings.retentionDays })));
    if (!ipOn() && loaded) kids.push(h('p', { class: 'act-ipoff', role: 'note' }, t('act.ipOff')));
    else if (proxyHint) kids.push(h('p', { class: 'act-ipoff', role: 'note' }, t('act.ipHint')));
    if (readError) {
      kids.push(h('div', { class: 'err act-err', role: 'alert' }, h('p', null, t('act.readError'), ' ', errorText(readError)),
        h('button', { type: 'button', class: 'btn', onclick: () => void readFirst(redraw) }, t('common.retry'))));
    } else if (!loaded) {
      kids.push(h('p', { class: 'act-empty', role: 'status' }, '…'));
    } else if (!entries.length) {
      const filtered = filters.user !== '' || filters.action !== '' || filters.result !== '';
      kids.push(h('p', { class: 'act-empty' }, t(filtered ? 'act.noMatch' : 'act.empty')));
    } else {
      kids.push(headRow(), h('ol', { class: 'act-list' }, entries.map((e) => row(e, redraw))));
      if (next != null) {
        kids.push(h('button', { type: 'button', class: 'btn act-more', disabled: loading, onclick: () => void readOlder(redraw) }, t('act.more')));
      }
    }
    body.replaceChildren(...kids.filter((k): k is HTMLElement => k != null));
  };

  const redraw = (): void => {
    if (mine !== viewId) return;
    redrawBar();
    redrawBody();
  };

  function schedule(): void {
    window.clearTimeout(timer);
    if (paused || mine !== viewId) return;
    timer = window.setTimeout(() => {
      if (mine !== viewId) return;
      if (!root.isConnected) {
        // drawn but not yet in the document (or already gone): try again later, never read for a hidden tab
        schedule();
        return;
      }
      const go = document.visibilityState === 'hidden' ? Promise.resolve() : readNewer(redraw);
      void go.then(schedule);
    }, ACTIVITY_REFRESH_MS);
  }

  redraw();
  void readFirst(redraw).then(schedule);
  return {
    el: root,
    stop: () => {
      window.clearTimeout(timer);
      viewId++;
    },
  };
}
