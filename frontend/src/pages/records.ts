// Study records: the list of exams and (v0.6.5) practice sessions with filters (#/records), one record in detail
// (#/records/<id>) and the wrong-answer review (#/records/wrong). Page-local UI state (filters, detail loading, the review
// filter) lives in this module; the filter rules are in domain/records-filter.ts.
import '../styles/records.css';
import { ApiError } from '../api/client.ts';
import { deleteAllRecords, deleteRecord } from '../api/progress.ts';
import { bankName, modeName, pageHead, recordBankNames } from '../components/common.ts';
import { aiPlusNow, naTag } from '../components/ai-plus-controls.ts';
import { openAiSummary } from '../components/ai-summary.ts';
import { iconButton, withReason } from '../components/icon-button.ts';
import { miniIcon } from '../components/mini-icons.ts';
import { kindTag } from '../components/kind-tag.ts';
import { controlState, recordSummaryExtra, unavailableParts, type AiPlusView, type ControlState } from '../domain/ai-plus.ts';
import { kindOf } from '../domain/attempt.ts';
import {
  NO_FILTERS, filterCount, focusAfterDelete, isFiltered, isKindFilter, matchesRecord, pageRecords, recordsPageSize, type RecordFilters,
} from '../domain/records-filter.ts';
import { pager } from '../components/pager.ts';
import { matches, PHONE_MQ } from '../utils/media.ts';
import { UR, cachedUserRecord, clearUserRecords, enterUserRecords, loadUserRecord, loadUserRecords, newListUi, type ListUi } from '../state/user-records.ts';
import { reviewItem, reviewParts, stripKey, type ReviewFilter, type ReviewParts, type ReviewRow } from '../components/review.ts';
import { defaultFilter, resultCounts } from '../domain/exam-timer.ts';
import { inNotebook, masterStreakOf, percentOf, shuffle } from '../domain/grading.ts';
import { errorText } from '../i18n/errors.ts';
import { t, tn, type MsgKey } from '../i18n/index.ts';
import { routeHash, type Route } from '../router.ts';
import { applyRecordsDeleted, loadRecord, navigate, onRouteChange, onSignedIn, refreshRecords, render, startExam } from '../state/actions.ts';
import { S, bankIdsOf } from '../state/store.ts';
import type { Mode, Question, RecordBank, RecordDetail, RecordItem, RecordSummary, UserRecordDetail, UserRef } from '../types/api.ts';
import { ask, askCheck, dialogTitleId } from '../utils/dialog.ts';
import { h, scrollToY } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';
import { liftToastAbove, toast } from '../utils/toast.ts';

// ---------------------------------------------------------------- page-local state

/** The own list's UI state: filters, page, scroll, folded filter bar, detail filter (kept while the app is open). The admin's
 *  view of another user keeps its own in state/user-records.ts and never touches this. */
const own: ListUi = newListUi();
/** The list UI state of the page being drawn: `tg` = the user an administrator is reading (null = the own records) */
const uiOf = (tg: number | null): ListUi => (tg == null ? own : UR.ui);
/** The user an administrator is reading, from a route (null on the own pages) */
const targetOf = (r: Route): number | null => (r.name === 'userRecords' || r.name === 'userRecord' ? r.user : null);
/** A delete request is running */
let busy = false;
/** Loading the record list when it is not in the store yet */
const listLoading = new Set<number | 'own'>();
const listError = new Map<number | 'own', unknown>();
/** Exam detail being fetched / failed to fetch, by record id (a failure is forgotten when the route changes, so coming back retries) */
const pending = new Set<number>();
const failures = new Map<number, unknown>();
/** Bank filter of the wrong-answer review */
let wrongBank: number | 'all' = 'all';
/** After a delete (RC-11): the record whose date link takes the focus, null = the list heading; undefined = nothing to do */
let pendingFocus: number | null | undefined;
/** The observer of the record detail's floating question-number button (RC-9) */
let jumpWatch: IntersectionObserver | null = null;

const isListRoute = (r: Route): boolean => r.name === 'records' || r.name === 'userRecords';
const isDetailRoute = (r: Route): boolean => r.name === 'record' || r.name === 'userRecord';
const sameDetail = (a: Route, b: Route): boolean =>
  (a.name === 'record' && b.name === 'record' && a.id === b.id)
  || (a.name === 'userRecord' && b.name === 'userRecord' && a.id === b.id && a.user === b.user);

onRouteChange((from, to) => {
  jumpWatch?.disconnect();
  jumpWatch = null;
  failures.clear();
  listError.clear();
  // The administrator's view of a user: another user starts fresh, leaving the view forgets everything
  const toUser = targetOf(to);
  if (toUser != null) enterUserRecords(toUser);
  else if (targetOf(from) != null) clearUserRecords();
  if (isDetailRoute(to) && !sameDetail(from, to)) uiOf(targetOf(to)).detailFilter = null;
  // Back from an exam to the list: return to where the list was (same page: the page number is kept too). The hooks run
  // before render() and the scroll to the top, so scrollY is still the list's position here, and the animation frame
  // comes after both.
  if (isListRoute(from) && isDetailRoute(to)) {
    uiOf(targetOf(from)).listY = window.scrollY;
  } else if (isDetailRoute(from) && !isDetailRoute(to)) {
    // (the target state may just have been cleared: a fresh one is harmless here)
    const ui = uiOf(targetOf(from));
    const y = ui.listY;
    ui.listY = 0;
    if (isListRoute(to) && y > 0) requestAnimationFrame(() => window.scrollTo(0, y));
  }
});

// Signed in again (the session had ended): try what failed while it was gone
onSignedIn(() => {
  failures.clear();
  listError.clear();
});

const isMode = (m: string): m is Mode => m === 'all' || m === 'weak' || m === 'retry';
const uniq = <T>(xs: readonly T[]): T[] => [...new Set(xs)];
const isQuestion = (q: Question | undefined): q is Question => !!q;

/** Active questions by id (S.bank only holds active questions) */
function activeById(): Map<string, Question> {
  return new Map((S.bank?.questions ?? []).map((q) => [q.id, q]));
}

/** A "back" link in the page head, named like the page it leads to (RC-10; a real link: it can also be opened in a new tab) */
/** `name`: the accessible name when the visible label is a short form (A-OWNER-87) */
function backButton(label: string, to: Route, name?: string): HTMLElement {
  return h('a', { class: 'btn quiet rec-back', href: routeHash(to), 'aria-label': name ?? null }, miniIcon('back', 'rec-back-ic'), label);
}

/** The list a detail page goes back to */
const listRoute = (tg: number | null): Route => (tg == null ? { name: 'records' } : { name: 'userRecords', user: tg });

// ---------------------------------------------------------------- deleting

/** `next`: where the focus goes after the list is redrawn (RC-11): a record id, or null = the list heading */
async function removeOne(r: RecordSummary, after?: () => void, next?: number | null): Promise<void> {
  if (busy) return;
  const ok = await ask({
    title: t('records.deleteOne.title'),
    body: t(kindOf(r) === 'practice' ? 'records.deleteOne.bodyPractice' : 'records.deleteOne.body', { at: r.at, pct: r.percent }),
    ok: t('records.deleteOne.ok'),
    cancel: t('common.cancel'),
    danger: true,
  });
  if (!ok) return;
  busy = true;
  render();
  try {
    const res = await deleteRecord(r.id);
    if (next !== undefined) pendingFocus = next;
    applyRecordsDeleted(res);
    toast(t('records.deleteOne.done'));
    after?.();
  } catch (e) {
    toast(t('records.deleteFailed', { reason: errorText(e) }), 6000);
    // The record may already be gone (deleted in another tab): bring the list up to date
    if (e instanceof ApiError && e.status === 404) refreshRecords().then(render, () => {});
  } finally {
    busy = false;
    render();
  }
}

async function removeAll(): Promise<void> {
  const n = S.records?.length ?? 0;
  if (busy || !n) return;
  const r = await askCheck({
    title: t('records.deleteAll.title'),
    body: tn('records.deleteAll.body', n),
    ok: t('records.deleteAll.ok'),
    cancel: t('common.cancel'),
    danger: true,
    check: { label: t('records.deleteAll.check'), hint: t('records.deleteAll.checkHint') },
  });
  if (!r.ok) return;
  busy = true;
  render();
  try {
    applyRecordsDeleted(await deleteAllRecords(r.checked));
    toast(r.checked ? t('records.deleteAll.doneStats') : t('records.deleteAll.done'), 4500);
  } catch (e) {
    toast(t('records.deleteFailed', { reason: errorText(e) }), 6000);
  } finally {
    busy = false;
    render();
  }
}

// ---------------------------------------------------------------- list (#/records)

/** Every bank that appears in the records (first occurrence = newest name); existing banks first, then deleted ones */
function banksInRecords(records: readonly RecordSummary[]): RecordBank[] {
  const seen = new Map<number, RecordBank>();
  for (const r of records) for (const b of r.banks) if (!seen.has(b.id)) seen.set(b.id, b);
  return [...seen.values()].sort((a, b) => Number(a.deleted) - Number(b.deleted) || a.id - b.id);
}

/** The overview follows every filter, the type included: its first number counts exams, practice sessions or both */
function summaryContent(rows: readonly RecordSummary[], total: number, filters: Readonly<RecordFilters>): HTMLElement[] {
  const none = t('common.none');
  const kpi = (value: string, label: string): HTMLElement => h('div', { class: 'rec-kpi' }, h('b', { class: 'num' }, value), h('span', null, label));
  const avg = rows.length ? percentOf(rows.reduce((s, r) => s + r.percent, 0), rows.length * 100) : null;
  const best = rows.length ? Math.max(...rows.map((r) => r.percent)) : null;
  const recent = rows.slice(0, 12).reverse();
  const scope = isFiltered(filters) ? tn('records.summary.scopeFiltered', rows.length) : tn('records.summary.scopeAll', total);
  const countLabel = filters.kind === 'exam' ? 'records.summary.exams' : filters.kind === 'practice' ? 'records.summary.practices' : 'records.summary.records';
  // Up to 640 px the heading and the scope line are only hidden visually (RC-2): the panel stays named, the row stays short
  return [
    h('h2', { id: 'rec-sum-h' }, t('records.summary.heading')),
    h('p', { class: 'rec-scope' }, scope),
    h('div', { class: 'rec-sum-grid' },
      kpi(String(rows.length), t(countLabel)),
      kpi(avg == null ? none : `${avg}%`, t('records.summary.average')),
      kpi(best == null ? none : `${best}%`, t('records.summary.best')),
      recent.length ? h('div', { class: 'rec-chart' },
        h('div', {
          class: 'bars', role: 'img',
          'aria-label': t('records.summary.chart', { list: recent.map((x) => `${x.percent}%`).join(t('common.listSep')) }),
        }, recent.map((x) => h('i', { style: `height:${Math.max(x.percent, 3)}%`, title: `${x.at} · ${x.percent}%` }))),
        h('span', null, tn('records.summary.recent', recent.length))) : null),
  ];
}

// ---------------------------------------------------------------- AI summary controls (v1.0, RC-3 / RC-4 / RC-5 / RC-8)

/** The text of a greyed control (its tip and the toast of a tap): `label` names the control */
function greyText(state: ControlState, label: string): string {
  const p = unavailableParts(state);
  if (!p) return '';
  return p.badgeReasonKey ? t('aiplus.tip.na', { label, reason: t(p.badgeReasonKey as MsgKey) }) : t(p.tipKey as MsgKey);
}

/**
 * The AI 总结 control of a record, the same one in the list (cell / card) and on the detail page: the sparkle icon button
 * with its text label; CSS hides the text where only the icon fits (phone cards, the detail row under PHONE). Usable: opens
 * the summary overlay. Greyed (SH-17): flat, the reason as tip and as the toast of a tap. null without the AI+ feature.
 */
function summaryControl(r: RecordSummary, state: ControlState): HTMLElement | null {
  if (!state.show) return null;
  const anchorKey = `rec-ai-${r.id}`;
  const btn = iconButton({
    icon: 'sparkle', label: t('aiplus.row.summaryLabel', { at: r.at }), tone: 'ai', focusKey: anchorKey,
    onClick: state.usable
      ? (e) => openAiSummary({ rec: { id: r.id, at: r.at, kind: kindOf(r) }, anchorKey, byKeyboard: e.detail === 0 })
      : undefined,
  });
  btn.classList.add('rec-lblbtn', 'rec-ai-btn');
  btn.append(h('span', { class: 'rec-lbl', 'aria-hidden': 'true' }, t('aiplus.summary')));
  if (!state.usable) {
    // v1.4 (owner): greyed exactly like a greyed AI 生成题 button, with the same 无法使用 tag when the AI+ feature itself is
    // unavailable (SH-17); the reason stays in the tip
    if (state.tag) btn.append(h('span', { class: 'rec-na', 'aria-hidden': 'true' }, t('aiplus.na')));
    return withReason(btn, greyText(state, t('aiplus.summary')));
  }
  btn.setAttribute('data-aip-opener', '');
  btn.setAttribute('aria-haspopup', 'dialog');
  return btn;
}

/** One note above the list when AI 总结 is greyed for everyone: the 无法使用 tag and the reason once (RC-5) */
function aiNote(state: ControlState): HTMLElement | null {
  const tag = naTag(state);
  if (!tag) return null;
  const p = unavailableParts(state);
  const text = p?.badgeReasonKey === 'ai.badge.suspended' ? t('aiplus.summaryPaused') : greyText(state, t('aiplus.summary'));
  return h('p', { class: 'aip-tablenote' }, tag, ' ', text);
}

/** The red trash button of a record (SH-16): icon only in the list; the detail page adds the text where there is room */
function deleteButton(r: RecordSummary, onClick: () => void, label = t('records.row.deleteLabel', { at: r.at })): HTMLButtonElement {
  const btn = iconButton({ icon: 'trash', label, tone: 'danger', onClick });
  btn.disabled = busy;
  return btn;
}

/** `view` / `onDelete` null = the administrator's read-only view of another user (`tg`): no AI 总结, no delete, links go to the target routes */
function recordRow(r: RecordSummary, view: AiPlusView | null, onDelete: ((r: RecordSummary) => void) | null, tg: number | null): HTMLElement {
  const cell = (label: string, cls: string, ...kids: Array<Node | string | null>): HTMLElement =>
    h('td', { role: 'cell', class: cls, 'data-label': label }, kids);
  const mode = modeName(r.mode);
  const band = r.percent >= 80 ? ' hi' : r.percent < 60 ? ' lo' : '';
  // "Answered n of m" is shown on a phone card only when something was left out (RC-3); the table always has the column
  const full = r.answered == null || r.answered >= r.total;
  return h('tr', {
    role: 'row',
    class: full ? null : 'rec-part',
    // The whole row opens the detail; the link in the date cell is the keyboard / screen-reader way in
    onclick: (e: MouseEvent) => {
      if (e.target instanceof Element && e.target.closest('a, button')) return;
      navigate(tg == null ? { name: 'record', id: r.id } : { name: 'userRecord', user: tg, id: r.id });
    },
  },
  view?.show ? h('td', { role: 'cell', class: 'rec-ai' }, summaryControl(r, controlState(view, recordSummaryExtra(r), 'summary'))) : null,
  cell(t('records.col.time'), 'rec-time num', h('a', { href: tg == null ? `#/records/${r.id}` : routeHash({ name: 'userRecord', user: tg, id: r.id }), 'data-rec': r.id }, r.at)),
  // The kind tag leads the cell, so exams and practice tell apart in the list and in the phone cards alike
  h('td', { role: 'cell', class: 'rec-banks', 'data-label': t('records.col.banks') },
    kindTag(kindOf(r)), r.ended === 'timeUp' ? h('span', { class: 'tag kind-tag k rec-timeup' }, t('records.timeUpTag')) : null,
    h('span', { class: 'rec-bnames' }, recordBankNames(r.banks)), mode ? h('span', { class: 'rec-mode' }, mode) : null),
  cell(t('records.col.answered'), 'num rec-st rec-st-ans' + (full ? ' rec-full' : ''), `${r.answered ?? t('common.none')} / ${r.total}`),
  cell(t('records.col.duration'), 'num rec-st rec-st-dur', fmtTime(r.seconds)),
  cell(t('records.col.score'), 'num rec-st rec-st-ok', `${r.correct} / ${r.total}`),
  cell(t('records.col.accuracy'), 'num rec-pct' + band, `${r.percent}%`),
  onDelete ? h('td', { role: 'cell', class: 'rec-act' }, deleteButton(r, () => onDelete(r))) : null);
}

function recordTable(rows: readonly RecordSummary[], view: AiPlusView | null, onDelete: ((r: RecordSummary) => void) | null, tg: number | null): { el: HTMLElement; body: HTMLElement } {
  const th = (label: string, hidden = false): HTMLElement =>
    h('th', { role: 'columnheader', scope: 'col' }, hidden ? h('span', { class: 'sr-only' }, label) : label);
  const body = h('tbody', { role: 'rowgroup' }, rows.map((r) => recordRow(r, view, onDelete, tg)));
  const el = h('div', { class: 'rec-scroll' },
    h('table', { class: 'rec-table' + (view?.show ? ' has-ai' : ''), role: 'table' },
      h('caption', { class: 'sr-only' }, t('records.table.caption')),
      h('thead', { role: 'rowgroup' }, h('tr', { role: 'row' },
        view?.show ? th(t('aiplus.summary'), true) : null,
        th(t('records.col.time')), th(t('records.col.banks')), th(t('records.col.answered')), th(t('records.col.duration')),
        th(t('records.col.score')), th(t('records.col.accuracy')), onDelete ? th(t('records.col.actions'), true) : null)),
      body));
  return { el, body };
}

/** A labelled filter control (explicit for/id, so the accessible name is just the label) */
function field(id: string, label: string, control: HTMLElement): HTMLElement {
  control.id = id;
  return h('div', { class: 'fld rec-field' }, h('label', { class: 'fld-l', for: id }, label), control);
}

function clearFilters(ui: ListUi): void {
  Object.assign(ui.filters, NO_FILTERS);
  ui.page = 1;
  render();
}

/** The record list is not in the store (should not happen after boot): load it, with an error state and retry */
function listPlaceholder(tg: number | null): HTMLElement {
  const key = tg ?? 'own';
  if (listError.has(key)) {
    const e = listError.get(key);
    return h('div', { class: 'err', role: 'alert' },
      h('p', null, t('records.loadFailed')),
      h('p', null, errorText(e)),
      h('div', { class: 'acts' },
        h('button', { class: 'btn', type: 'button', onclick: () => { listError.delete(key); render(); } }, t('common.retry')),
        tg == null ? null : backButton(t('records.target.back'), { name: 'users' })));
  }
  if (!listLoading.has(key)) {
    listLoading.add(key);
    (tg == null ? refreshRecords() : loadUserRecords(tg).then(() => undefined)).then(() => {
      listLoading.delete(key);
      render();
    }, (e: unknown) => {
      listLoading.delete(key);
      listError.set(key, e);
      render();
    });
  }
  return h('p', { class: 'dim', role: 'status' }, t('records.loading'));
}

/** After a delete the list is drawn again: put the focus where RC-11 says (the next card's date link, else the heading) */
function restoreFocusAfterDelete(page: HTMLElement): void {
  const target = pendingFocus;
  pendingFocus = undefined;
  if (target === undefined) return;
  requestAnimationFrame(() => {
    const el = target === null
      ? page.querySelector<HTMLElement>('#rec-list-h, .rec-empty h2')
      : page.querySelector<HTMLElement>(`a[data-rec="${target}"]`);
    el?.focus();
  });
}

/** The name an administrator sees for the user being read: the display name, else the login */
const nameOf = (u: UserRef): string => u.displayName || u.username;
/** The target's name for a read-only page: from the loaded record / list (null until one of them has arrived) */
const targetName = (tg: number | null, d?: UserRecordDetail): string | null => {
  if (tg == null) return null;
  const u = d?.user ?? (UR.uid === tg ? UR.list?.user : undefined);
  return u ? nameOf(u) : null;
};
/** The label of the back link to the list: the target's list title for an administrator, else "Study records" */
const listBackLabel = (name: string | null): string => (name ? t('records.target.title', { name }) : t('records.title'));

/** "View only" under the page head of the administrator's view */
const targetNote = (): HTMLElement => h('p', { class: 'dim rec-target-note' }, t('records.target.note'));

/** The page head of the list: own (with the wrong-answer review) or the read-only view of user `tg` */
function listHead(tg: number | null): HTMLElement {
  if (tg == null) {
    return pageHead(t('records.title'),
      h('button', { class: 'btn', type: 'button', onclick: () => navigate({ name: 'wrong' }) }, t('records.reviewWrong')));
  }
  const who = UR.uid === tg ? UR.list?.user : undefined;
  return pageHead(who ? t('records.target.title', { name: nameOf(who) }) : t('records.title'), backButton(t('records.target.back'), { name: 'users' }));
}

/** `tg`: the user an administrator is reading (read only: no delete, no AI, no study buttons); null = the own records */
function renderList(tg: number | null): HTMLElement {
  const ui = uiOf(tg);
  const filters = ui.filters;
  const ro = tg != null;
  const all = ro ? (UR.uid === tg ? UR.list?.records : undefined) : S.records;
  const retentionDays = ro ? (UR.list?.retention ?? 0) : S.recordsRetention;
  const page = h('div', { class: 'page page-records' }, listHead(tg), ro ? targetNote() : null);
  const retention = h('p', { class: 'rec-retention dim' }, t('records.retention', { n: retentionDays }));

  if (!all) {
    page.append(listPlaceholder(tg));
    return page;
  }
  if (!all.length) {
    page.append(
      h('section', { class: 'panel rec-empty' },
        h('h2', { tabindex: '-1' }, t(ro ? 'records.target.empty' : 'records.empty.title')),
        ro ? null : h('p', { class: 'empty' }, t('records.empty.body')),
        ro ? null : h('div', { class: 'acts' }, h('button', { class: 'btn primary', type: 'button', onclick: () => navigate({ name: 'study' }) }, t('records.empty.start')))),
      retention);
    restoreFocusAfterDelete(page);
    return page;
  }

  const banks = banksInRecords(all);
  if (filters.bank !== 'all' && !banks.some((b) => String(b.id) === filters.bank)) filters.bank = 'all';

  // Filter controls: changing one updates the summary and the list in place (no full re-render, focus stays put)
  const bankSel = h('select', { class: 'rec-bank-sel' },
    h('option', { value: 'all' }, t('records.filter.allBanks')),
    banks.map((b) => h('option', { value: String(b.id) }, b.deleted ? t('common.deletedBank', { name: b.name }) : b.name)));
  bankSel.value = filters.bank;
  const kindSel = h('select', null,
    h('option', { value: 'all' }, t('records.filter.allKinds')),
    h('option', { value: 'exam' }, t('records.filter.exams')),
    h('option', { value: 'practice' }, t('records.filter.practice')));
  kindSel.value = filters.kind;
  const fromIn = h('input', { type: 'date', value: filters.from });
  const toIn = h('input', { type: 'date', value: filters.to });
  const scoreSel = h('select', null,
    h('option', { value: 'any' }, t('records.filter.anyScore')),
    h('option', { value: 'low' }, t('records.filter.low')),
    h('option', { value: 'mid' }, t('records.filter.mid')),
    h('option', { value: 'high' }, t('records.filter.high')));
  scoreSel.value = filters.score;

  const summary = h('section', { class: 'panel rec-sum', 'aria-labelledby': 'rec-sum-h' });
  const count = h('p', { class: 'rec-count', role: 'status' });
  const badRange = h('p', { class: 'rec-warn' }, t('records.filter.badRange'));
  // Up to 400 px only the x shows (the text stays as the accessible name)
  const resetBtn = h('button', { class: 'btn quiet rec-reset', type: 'button', title: t('records.filter.reset'), onclick: () => clearFilters(ui) },
    miniIcon('x', 'rec-ri'), h('span', { class: 'rec-rtxt' }, t('records.filter.reset')));
  // Up to 640 px the filters fold behind this button (RC-1): [筛选 (n)] · the count · 清除筛选
  const toggleText = h('span', { class: 'rec-ftxt' });
  const toggle = h('button', {
    class: 'btn rec-ftoggle', type: 'button', 'aria-expanded': String(ui.filtersOpen), 'aria-controls': 'rec-filters',
    onclick: () => {
      ui.filtersOpen = !ui.filtersOpen;
      filterPanel.classList.toggle('open', ui.filtersOpen);
      toggle.setAttribute('aria-expanded', String(ui.filtersOpen));
    },
  }, miniIcon('filter', 'rec-fi'), toggleText, miniIcon('chevDown', 'rec-fchev'));
  const listBox = h('div', { class: 'rec-list' });
  // Which rows are on screen, read out for screen readers (v2.7)
  const range = h('p', { class: 'sr-only', role: 'status', 'aria-live': 'polite' });
  const deleteAllBtn = ro ? null : h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void removeAll() }, t('records.deleteAll'));
  const view = ro ? null : aiPlusNow();
  /** The rows of the current page, in order (the neighbour of a deleted record is looked up here) */
  let shownIds: number[] = [];

  const onDelete = ro ? null : (r: RecordSummary): void => {
    void removeOne(r, undefined, focusAfterDelete(shownIds, r.id));
  };

  const update = (): void => {
    const rows = all.filter((r) => matchesRecord(r, filters));
    // Numbered pages (v2.7): 10 on a phone, 20 elsewhere; a page past the end (after a delete) falls back to the last one
    const pg = pageRecords(rows, ui.page, recordsPageSize(matches(PHONE_MQ)));
    ui.page = pg.page;
    shownIds = pg.items.map((r) => r.id);
    const n = filterCount(filters);
    summary.replaceChildren(...summaryContent(rows, all.length, filters));
    count.textContent = tn('records.count', all.length, { shown: rows.length });
    range.textContent = rows.length ? t('records.range', { from: pg.from, to: pg.to, total: pg.total }) : '';
    toggleText.textContent = n ? `${t('records.filter.heading')} (${n})` : t('records.filter.heading');
    badRange.hidden = !(filters.from && filters.to && filters.from > filters.to);
    resetBtn.hidden = !isFiltered(filters);
    // "Delete all" would also delete what the filters hide: it is offered only on the full list (RC-11)
    if (deleteAllBtn) deleteAllBtn.hidden = isFiltered(filters);
    if (!rows.length) {
      listBox.replaceChildren(h('div', { class: 'rec-nomatch' },
        h('p', { class: 'empty' }, t('records.noMatch')),
        h('button', { class: 'btn', type: 'button', onclick: () => clearFilters(ui) }, t('records.filter.reset'))));
      return;
    }
    const table = recordTable(pg.items, view, onDelete, tg);
    listBox.replaceChildren(table.el, pager({
      id: 'rec', page: pg.page, pages: pg.pages, scrollTarget: listBox, preventScroll: true,
      onGo: (p) => {
        ui.page = p;
        update();
      },
    }));
  };
  const refilter = (): void => {
    ui.page = 1;
    update();
  };
  bankSel.addEventListener('change', () => {
    filters.bank = bankSel.value;
    refilter();
  });
  kindSel.addEventListener('change', () => {
    filters.kind = isKindFilter(kindSel.value) ? kindSel.value : 'all';
    refilter();
  });
  fromIn.addEventListener('change', () => {
    filters.from = fromIn.value;
    refilter();
  });
  toIn.addEventListener('change', () => {
    filters.to = toIn.value;
    refilter();
  });
  scoreSel.addEventListener('change', () => {
    const v = scoreSel.value;
    filters.score = v === 'low' || v === 'mid' || v === 'high' ? v : 'any';
    refilter();
  });

  const filterPanel = h('section', { class: 'panel rec-filter' + (ui.filtersOpen ? ' open' : ''), 'aria-labelledby': 'rec-filter-h' },
    h('h2', { id: 'rec-filter-h' }, t('records.filter.heading')),
    h('div', { class: 'rec-bar' }, toggle, count, resetBtn),
    // Type and bank first, so that From and To share a row in the two-column phone layout (accuracy comes last)
    h('div', { class: 'rec-filters', id: 'rec-filters' },
      field('rec-f-kind', t('records.filter.kind'), kindSel),
      field('rec-f-bank', t('records.filter.bank'), bankSel),
      field('rec-f-from', t('records.filter.from'), fromIn),
      field('rec-f-to', t('records.filter.to'), toIn),
      field('rec-f-score', t('records.filter.score'), scoreSel)),
    badRange);

  const listPanel = h('section', { class: 'panel rec-listpanel' + (view?.show ? ' has-ai' : ''), 'aria-labelledby': 'rec-list-h' },
    h('h2', { id: 'rec-list-h', tabindex: '-1' }, t('records.list.heading')),
    // When AI 总结 is greyed for everyone, one note says why (instead of a tag on every row)
    view?.show ? aiNote(controlState(view, null, 'summary')) : null,
    range,
    listBox);

  update();
  // "Delete all" is rarely needed: at the end of the page, away from the study actions in the page head
  page.append(summary, filterPanel, listPanel, h('div', { class: 'rec-foot' }, retention, deleteAllBtn));
  restoreFocusAfterDelete(page);
  return page;
}

// ---------------------------------------------------------------- detail (#/records/<id>)

function fetchDetail(id: number, tg: number | null): void {
  pending.add(id);
  (tg == null ? loadRecord(id) : loadUserRecord(tg, id)).then(() => {
    failures.delete(id);
  }, (e: unknown) => {
    failures.set(id, e);
  }).finally(() => {
    pending.delete(id);
    const r = S.route;
    if ((r.name === 'record' || r.name === 'userRecord') && r.id === id) render();
  });
}

function detailError(id: number, e: unknown, tg: number | null): HTMLElement {
  const notFound = e instanceof ApiError && (e.status === 404 || e.status === 400);
  return h('div', { class: 'page page-records' },
    pageHead(notFound ? t('records.detail.notFound') : t('records.detail.loadFailed')),
    h('div', { class: 'err', role: 'alert' },
      h('p', null, errorText(e)),
      h('div', { class: 'acts rec-err-acts' },
        notFound ? null : h('button', {
          class: 'btn primary', type: 'button',
          onclick: () => {
            failures.delete(id);
            render();
          },
        }, t('common.retry')),
        backButton(listBackLabel(targetName(tg)), listRoute(tg)))));
}

/** A question that no longer exists (deleted with its bank): a small card that still says whether it was right */
function goneCard(it: RecordItem, no: number): HTMLElement {
  const unanswered = !it.answer || it.answer.every((x) => x == null);
  const status = it.correct
    ? h('span', { class: 'badge ok' }, t('result.q.correct'))
    : it.unanswered
      ? h('span', { class: 'badge open' }, t('result.q.notCompleted'))
      : h('span', { class: 'badge ng' }, unanswered ? t('result.q.unanswered') : t('result.q.wrong'));
  return h('article', { class: 'rv rv-gone' + (it.correct ? ' good' : ''), tabindex: '-1', 'aria-label': t('result.q.no', { n: no }) },
    h('div', { class: 'paper' },
      h('div', { class: 'phead' }, h('b', { class: 'num' }, t('result.q.no', { n: no })), status),
      h('p', { class: 'dim' }, t('records.detail.gone'))));
}

function detailView(d: RecordDetail, tg: number | null): HTMLElement {
  const ro = tg != null;
  const ui = uiOf(tg);
  const rec = d.record;
  const who = ro ? targetName(tg, d as UserRecordDetail) : null;
  const items = [...d.items].sort((a, b) => a.seq - b.seq);
  const active = activeById();
  // v1.1 (TL-7): a record saved by v1.1 flags its not-completed items; older records and practice have none
  const wrongItems = items.filter((it) => !it.correct && !it.unanswered);
  const counts = resultCounts(items);
  const wrongIds = uniq(wrongItems.map((it) => it.questionId));
  const allIds = uniq(items.map((it) => it.questionId));
  const retryQs = wrongIds.map((id) => active.get(id)).filter(isQuestion);
  const retakeQs = allIds.map((id) => active.get(id)).filter(isQuestion);
  const mode: Mode = isMode(rec.mode) ? rec.mode : 'all';
  // Retry and retake start the same kind as the record: a practice is practised again
  const kind = kindOf(rec);
  const hasDetail = rec.detail && items.length > 0;

  const pair = (label: string, value: string, cls?: string): HTMLElement[] => [h('dt', null, label), h('dd', cls ? { class: cls } : null, value)];
  const meta = h('dl', { class: 'rec-meta props' },
    pair(t('records.detail.time'), rec.at),
    [h('dt', null, t('records.detail.kind')), h('dd', null, kindTag(kind))],
    pair(t('records.detail.banks'), recordBankNames(rec.banks), 'rec-bnames'),
    rec.sheets.length ? pair(t('records.detail.sheets'), rec.sheets.join(t('common.listSep')), 'rec-bnames') : null,
    modeName(rec.mode) ? pair(t('records.detail.mode'), modeName(rec.mode)) : null,
    pair(t('records.detail.duration'), rec.limit != null ? `${fmtTime(rec.seconds)} · ${t('result.limitMeta', { min: Math.round(rec.limit / 60) })}` : fmtTime(rec.seconds)),
    pair(t('records.detail.answered'), t('records.detail.answeredValue', { answered: rec.answered ?? t('common.none'), total: rec.total })));

  const hints: string[] = [];
  let retryBtn: HTMLElement | null = null;
  let retakeBtn: HTMLElement | null = null;
  /** "Retry wrong answers" (built twice: in the score block and after the review list) */
  const retryButton = (): HTMLElement => h('button', {
    class: 'btn primary', type: 'button', disabled: !retryQs.length,
    onclick: () => void startExam(shuffle(retryQs), 'retry', kind),
  }, t('records.detail.retryWrong'));
  /** An icon button that also shows its text where there is room (the text goes under PHONE, RC-8) */
  const labelled = (btn: HTMLButtonElement, text: string): HTMLButtonElement => {
    btn.classList.add('rec-lblbtn');
    btn.append(h('span', { class: 'rec-lbl', 'aria-hidden': 'true' }, text));
    return btn;
  };
  // The administrator's read-only view has no retry / retake / delete / AI 总结: those would act on the administrator's own session
  if (hasDetail && !ro) {
    if (wrongIds.length) {
      retryBtn = retryButton();
      if (!retryQs.length) hints.push(t('records.detail.retryNone'));
      else if (retryQs.length < wrongIds.length) hints.push(tn('records.detail.retrySkip', wrongIds.length - retryQs.length));
    }
    const retakeText = t(kind === 'practice' ? 'records.detail.retakePractice' : 'records.detail.retake');
    retakeBtn = labelled(iconButton({
      icon: 'retake', label: retakeText, onClick: () => void startExam(shuffle(retakeQs), mode, kind),
    }), retakeText);
    (retakeBtn as HTMLButtonElement).disabled = !retakeQs.length;
    if (!retakeQs.length) hints.push(t('records.detail.retakeNone'));
    else if (retakeQs.length < allIds.length) hints.push(tn('records.detail.retakeSkip', allIds.length - retakeQs.length));
  }
  // AI 总结: the same control as in the list (AI+ users only; a record without detail has no wrong answers to read)
  const aiBtn = ro ? null : summaryControl(rec, controlState(aiPlusNow(), recordSummaryExtra(rec), 'summary'));
  // The red trash button, set apart from the study actions (the dialog still asks before anything is deleted)
  const delBtn = ro ? null : labelled(
    deleteButton(rec, () => void removeOne(rec, () => navigate({ name: 'records' })), t('records.detail.delete')),
    t('records.detail.delete'));
  delBtn?.classList.add('rec-del');

  const page = h('div', { class: 'page page-records page-record' },
    pageHead(
      who
        ? t(kind === 'practice' ? 'records.target.detailTitlePractice' : 'records.target.detailTitle', { name: who })
        : t(kind === 'practice' ? 'records.detail.titlePractice' : 'records.detail.title'),
      backButton(listBackLabel(who), listRoute(tg))),
    ro ? targetNote() : null,
    h('section', { class: 'score rec-score' },
      h('div', { class: 'big num', role: 'img', 'aria-label': t('records.detail.scoreLabel', { pct: rec.percent }) }, String(rec.percent), h('small', null, '%')),
      h('div', { class: 'says' },
        h('p', null, counts.open > 0
          ? t('result.summaryOpen', { good: rec.correct, wrong: rec.total - rec.correct - counts.open, open: counts.open })
          : t('records.detail.summary', { correct: rec.correct, total: rec.total })),
        counts.open > 0 && rec.ended === 'submit' ? h('p', { class: 'dim rec-opennote' }, t('result.openNote')) : null,
        meta,
        ro ? null : h('div', { class: 'acts rec-acts' }, retryBtn, retakeBtn, aiBtn, delBtn),
        hints.length ? h('div', { class: 'rec-hints' }, hints.map((x) => h('p', null, x))) : null)));

  if (!hasDetail) {
    page.append(h('p', { class: 'rec-note' }, t('records.detail.noDetail')));
    return page;
  }

  const rows: ReviewRow[] = items.map((it, i) => ({
    correct: it.correct,
    unanswered: it.unanswered,
    render: () => (it.question
      ? reviewItem({ q: it.question, ans: it.answer, correct: it.correct, unanswered: it.unanswered }, {
        no: i + 1,
        retired: !it.question.active,
        // v0.9: AI analysis of this record's questions (the controls build themselves only for users with the AI+ feature)
        ...(ro ? { stats: null, ...(who ? { who } : {}) } : { ai: { page: 'detail' as const, mode: kind, key: rec.sessionKey ?? null, startedAt: rec.startedAt ?? null, recordId: rec.id } }),
      })
      : goneCard(it, i + 1)),
  }));
  // The default filter: wrong answers first, else the not-completed ones, else all (TL-7); a record from before v1.1 keeps v1.0's
  const dflt: ReviewFilter = rec.ended ? defaultFilter(counts) : 'wrong';
  const parts = reviewParts(rows, { get: () => ui.detailFilter ?? dflt, set: (f) => { ui.detailFilter = f; } });
  page.append(...parts.els);
  // After reading through the answers, practise the wrong ones without scrolling back up
  if (!ro && wrongIds.length && retryQs.length) page.append(h('div', { class: 'acts wr-foot' }, retryButton()));
  page.append(jumpButton(rows, parts));
  return page;
}

/**
 * The floating question-number button of a long detail page (RC-9, phones): it shows once the answer strip has scrolled
 * out above the screen and hides again near the top. A tap opens a bottom sheet with the strip (a tap on a number jumps
 * to the card and closes the sheet) and 回到顶部.
 */
function jumpButton(rows: readonly ReviewRow[], parts: ReviewParts): HTMLElement {
  const fab = h('button', {
    class: 'rec-fab', type: 'button', 'aria-label': t('records.jump.title'), 'aria-haspopup': 'dialog',
    onclick: () => openJumpSheet(rows, parts),
  }, miniIcon('grid', 'rec-fab-ic'));
  jumpWatch?.disconnect();
  if (typeof IntersectionObserver !== 'undefined') {
    // The button shows once the strip is above the screen, and hides while the page footer is on screen (it would cover the
    // disclaimer's last lines for good)
    let stripAbove = false;
    let footOn = false;
    const apply = (): void => { fab.classList.toggle('show', stripAbove && !footOn); };
    const watch = new IntersectionObserver((entries) => {
      for (const e of entries) {
        if (e.target === parts.strip) stripAbove = !e.isIntersecting && e.boundingClientRect.bottom < 0;
        else footOn = e.isIntersecting;
      }
      apply();
    });
    jumpWatch = watch;
    watch.observe(parts.strip);
    // The shell footer is outside this page and mounted after it is built: look it up on the next tick
    setTimeout(() => {
      const foot = document.querySelector('footer.foot');
      if (foot && jumpWatch === watch) watch.observe(foot);
    }, 0);
  }
  // Toasts sit above the button while it is on screen (a hidden button counts as not visible)
  liftToastAbove(fab);
  return fab;
}

function openJumpSheet(rows: readonly ReviewRow[], parts: ReviewParts): void {
  const titleId = dialogTitleId();
  const close = (): void => d.close();
  const keys = h('div', { class: 'strip', role: 'group', 'aria-label': t('result.strip.label') }, rows.map((r, i) => stripKey(r, i, () => {
    close();
    parts.jump(i);
  })));
  const d = h('dialog', { class: 'dlg rec-jump', 'aria-labelledby': titleId },
    h('h3', { id: titleId, tabindex: '-1' }, t('records.jump.title')),
    keys,
    h('div', { class: 'acts' },
      h('button', {
        class: 'btn', type: 'button',
        onclick: () => {
          close();
          scrollToY(0, true);
          // The button hides at the top: keep the focus on the page heading so the next Tab continues from there
          const head = document.querySelector<HTMLElement>('.page-record h1');
          if (head) { head.tabIndex = -1; head.focus({ preventScroll: true }); }
        },
      }, miniIcon('up', 'rec-up-ic'), t('common.toTop')),
      h('button', { class: 'btn', type: 'button', onclick: close }, t('common.close'))));
  d.addEventListener('close', () => d.remove());
  document.body.append(d);
  d.showModal();
}

function renderDetail(id: number, tg: number | null): HTMLElement {
  const d = tg == null ? S.recordCache.get(id) : cachedUserRecord(tg, id);
  if (d) return detailView(d, tg);
  if (failures.has(id)) return detailError(id, failures.get(id), tg);
  if (!pending.has(id)) fetchDetail(id, tg);
  return h('div', { class: 'page page-records' },
    pageHead(t('records.detail.title'), backButton(listBackLabel(targetName(tg)), listRoute(tg))),
    tg == null ? null : targetNote(),
    h('p', { class: 'dim', role: 'status' }, t('records.detail.loading')));
}

// ---------------------------------------------------------------- wrong-answer review (#/records/wrong)

/** Wrong count, attempts, current streak, and how many more correct answers in a row take it out of the notebook */
function notebookStats(q: Question, masterStreak: number): HTMLElement | null {
  const st = S.progress?.questions?.[q.id];
  if (!st) return null;
  // One row: the stats, then a neutral chip with what is still missing (RC-10)
  return h('div', { class: 'stats wr-stats' },
    h('p', null, t('records.wrong.stats', { wrong: st.wrong, attempts: st.attempts, streak: st.streak })),
    h('span', { class: 'wr-left' }, t('records.wrong.leftChip', { n: Math.max(1, masterStreak - st.streak) })));
}

function renderWrong(): HTMLElement {
  const ms = masterStreakOf(S.progress);
  const all = (S.bank?.questions ?? []).filter((q) => inNotebook(S.progress?.questions?.[q.id], ms));
  const bankIds = bankIdsOf(all);
  if (wrongBank !== 'all' && !bankIds.includes(wrongBank)) wrongBank = 'all';
  const current = (): Question[] => (wrongBank === 'all' ? all : all.filter((q) => q.bank === wrongBank));

  /** "Practise these" (built twice on a long list: in the page head and after the list): starts a practice (immediate
   *  feedback, v0.7), not an exam, of the notebook questions in the current bank filter */
  const practise = (): HTMLElement => h('button', {
    class: 'btn primary', type: 'button',
    onclick: () => {
      const qs = current();
      if (qs.length) void startExam(shuffle(qs), 'weak', 'practice');
    },
  }, t('records.wrong.practise'));
  const page = h('div', { class: 'page page-records page-wrong' },
    pageHead(t('records.wrong.title'), backButton(t('records.wrong.back'), { name: 'records' }, t('records.title')), all.length ? practise() : null));

  if (!all.length) {
    page.append(h('section', { class: 'panel rec-empty' },
      h('h2', null, t('records.wrong.empty.title')),
      h('p', { class: 'empty' }, t('records.wrong.empty.body')),
      h('p', { class: 'dim' }, tn('records.wrong.intro', ms)),
      h('div', { class: 'acts' }, h('button', { class: 'btn primary', type: 'button', onclick: () => navigate({ name: 'study' }) }, t('records.empty.start')))));
    return page;
  }

  const count = h('p', { class: 'wr-count', role: 'status' });
  const list = h('div', { class: 'rvlist' });
  const update = (): void => {
    const qs = current();
    count.textContent = tn('records.wrong.count', qs.length);
    list.replaceChildren(...qs.map((q, i) => reviewItem({ q, ans: null, correct: false }, { no: i + 1, showMine: false, stats: notebookStats(q, ms) })));
  };

  let bankFilter: HTMLElement | null = null;
  if (bankIds.length > 1) {
    const sel = h('select', { class: 'rec-bank-sel' },
      h('option', { value: 'all' }, t('records.wrong.bankOption', { name: t('records.filter.allBanks'), n: all.length })),
      bankIds.map((id) => h('option', { value: String(id) }, t('records.wrong.bankOption', { name: bankName(id), n: all.filter((q) => q.bank === id).length }))));
    sel.value = String(wrongBank);
    sel.addEventListener('change', () => {
      wrongBank = sel.value === 'all' ? 'all' : Number(sel.value);
      update();
    });
    bankFilter = field('wr-f-bank', t('records.filter.bank'), sel);
  }

  update();
  page.append(
    h('p', { class: 'wr-intro' }, tn('records.wrong.intro', ms)),
    h('div', { class: 'wr-tools' }, bankFilter, count),
    list);
  if (all.length > 3) page.append(h('div', { class: 'acts wr-foot' }, practise()));
  return page;
}

// ---------------------------------------------------------------- entry

export function renderRecords(route: Route): HTMLElement {
  // The administrator's read-only view of a user (v2.7): its own state, never S.records / S.recordCache
  if (route.name === 'userRecord') {
    enterUserRecords(route.user);
    return renderDetail(route.id, route.user);
  }
  if (route.name === 'userRecords') {
    enterUserRecords(route.user);
    return renderList(route.user);
  }
  if (route.name === 'record') return renderDetail(route.id, null);
  if (route.name === 'wrong') return renderWrong();
  return renderList(null);
}
