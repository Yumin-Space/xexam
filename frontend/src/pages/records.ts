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
  NO_FILTERS, RECORDS_PAGE, filterCount, focusAfterDelete, isFiltered, isKindFilter, matchesRecord, nextLimit, pageRows, type RecordFilters,
} from '../domain/records-filter.ts';
import { reviewItem, reviewParts, type ReviewFilter, type ReviewParts, type ReviewRow } from '../components/review.ts';
import { inNotebook, masterStreakOf, percentOf, shuffle } from '../domain/grading.ts';
import { errorText } from '../i18n/errors.ts';
import { t, tn, type MsgKey } from '../i18n/index.ts';
import { routeHash, type Route } from '../router.ts';
import { applyRecordsDeleted, loadRecord, navigate, onRouteChange, onSignedIn, refreshRecords, render, startExam } from '../state/actions.ts';
import { S, bankIdsOf } from '../state/store.ts';
import type { Mode, Question, RecordBank, RecordDetail, RecordItem, RecordSummary } from '../types/api.ts';
import { ask, askCheck, dialogTitleId } from '../utils/dialog.ts';
import { h, scrollToY, svgMark } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';
import { liftToastAbove, toast } from '../utils/toast.ts';

// ---------------------------------------------------------------- page-local state

/** List filters (kept while the app is open) */
const filters: RecordFilters = { ...NO_FILTERS };
/** A delete request is running */
let busy = false;
/** Loading the record list when it is not in the store yet */
let listLoading = false;
let listError: { e: unknown } | null = null;
/** Exam detail being fetched / failed to fetch, by record id (a failure is forgotten when the route changes, so coming back retries) */
const pending = new Set<number>();
const failures = new Map<number, unknown>();
/** "Wrong only / all" on the detail page; back to "wrong only" whenever another record is opened */
let detailFilter: ReviewFilter = 'wrong';
/** Bank filter of the wrong-answer review */
let wrongBank: number | 'all' = 'all';
/** Scroll position of the list when an exam was opened from it (0 = nothing to restore) */
let listY = 0;
/** The phone filter bar is unfolded (RC-1; kept while the app is open, like the filters) */
let filtersOpen = false;
/** How many records the list shows (RC-6); back to one page whenever a filter changes. Kept when a record is opened. */
let shownLimit = RECORDS_PAGE;
/** After a delete (RC-11): the record whose date link takes the focus, null = the list heading; undefined = nothing to do */
let pendingFocus: number | null | undefined;
/** The observer of the record detail's floating question-number button (RC-9) */
let jumpWatch: IntersectionObserver | null = null;

onRouteChange((from, to) => {
  jumpWatch?.disconnect();
  jumpWatch = null;
  failures.clear();
  listError = null;
  if (to.name === 'record' && !(from.name === 'record' && from.id === to.id)) detailFilter = 'wrong';
  // Back from an exam to the list: return to where the list was. The hooks run before render() and the scroll to the
  // top, so scrollY is still the list's position here, and the animation frame comes after both.
  if (from.name === 'records' && to.name === 'record') {
    listY = window.scrollY;
  } else if (from.name === 'record' && to.name !== 'record') {
    const y = listY;
    listY = 0;
    if (to.name === 'records' && y > 0) requestAnimationFrame(() => window.scrollTo(0, y));
  }
});

// Signed in again (the session had ended): try what failed while it was gone
onSignedIn(() => {
  failures.clear();
  listError = null;
});

const isMode = (m: string): m is Mode => m === 'all' || m === 'weak' || m === 'retry';
const uniq = <T>(xs: readonly T[]): T[] => [...new Set(xs)];
const isQuestion = (q: Question | undefined): q is Question => !!q;

/** Active questions by id (S.bank only holds active questions) */
function activeById(): Map<string, Question> {
  return new Map((S.bank?.questions ?? []).map((q) => [q.id, q]));
}

/** A "back" link in the page head, named like the page it leads to (RC-10; a real link: it can also be opened in a new tab) */
function backButton(label: string, to: Route): HTMLElement {
  return h('a', { class: 'btn quiet rec-back', href: routeHash(to) }, miniIcon('back', 'rec-back-ic'), label);
}

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
function summaryContent(rows: readonly RecordSummary[], total: number): HTMLElement[] {
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
  if (!state.usable) return withReason(btn, greyText(state, t('aiplus.summary')));
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

function recordRow(r: RecordSummary, view: AiPlusView, onDelete: (r: RecordSummary) => void): HTMLElement {
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
      navigate({ name: 'record', id: r.id });
    },
  },
  view.show ? h('td', { role: 'cell', class: 'rec-ai' }, summaryControl(r, controlState(view, recordSummaryExtra(r)))) : null,
  cell(t('records.col.time'), 'rec-time num', h('a', { href: `#/records/${r.id}`, 'data-rec': r.id }, r.at)),
  // The kind tag leads the cell, so exams and practice tell apart in the list and in the phone cards alike
  h('td', { role: 'cell', class: 'rec-banks', 'data-label': t('records.col.banks') },
    kindTag(kindOf(r)), h('span', { class: 'rec-bnames' }, recordBankNames(r.banks)), mode ? h('span', { class: 'rec-mode' }, mode) : null),
  cell(t('records.col.answered'), 'num rec-st rec-st-ans' + (full ? ' rec-full' : ''), `${r.answered ?? t('common.none')} / ${r.total}`),
  cell(t('records.col.duration'), 'num rec-st rec-st-dur', fmtTime(r.seconds)),
  cell(t('records.col.score'), 'num rec-st rec-st-ok', `${r.correct} / ${r.total}`),
  cell(t('records.col.accuracy'), 'num rec-pct' + band, `${r.percent}%`),
  h('td', { role: 'cell', class: 'rec-act' }, deleteButton(r, () => onDelete(r))));
}

function recordTable(rows: readonly RecordSummary[], view: AiPlusView, onDelete: (r: RecordSummary) => void): { el: HTMLElement; body: HTMLElement } {
  const th = (label: string, hidden = false): HTMLElement =>
    h('th', { role: 'columnheader', scope: 'col' }, hidden ? h('span', { class: 'sr-only' }, label) : label);
  const body = h('tbody', { role: 'rowgroup' }, rows.map((r) => recordRow(r, view, onDelete)));
  const el = h('div', { class: 'rec-scroll' },
    h('table', { class: 'rec-table' + (view.show ? ' has-ai' : ''), role: 'table' },
      h('caption', { class: 'sr-only' }, t('records.table.caption')),
      h('thead', { role: 'rowgroup' }, h('tr', { role: 'row' },
        view.show ? th(t('aiplus.summary'), true) : null,
        th(t('records.col.time')), th(t('records.col.banks')), th(t('records.col.answered')), th(t('records.col.duration')),
        th(t('records.col.score')), th(t('records.col.accuracy')), th(t('records.col.actions'), true))),
      body));
  return { el, body };
}

/** A labelled filter control (explicit for/id, so the accessible name is just the label) */
function field(id: string, label: string, control: HTMLElement): HTMLElement {
  control.id = id;
  return h('div', { class: 'rec-field' }, h('label', { for: id }, label), control);
}

function clearFilters(): void {
  Object.assign(filters, NO_FILTERS);
  shownLimit = RECORDS_PAGE;
  render();
}

/** The record list is not in the store (should not happen after boot): load it, with an error state and retry */
function listPlaceholder(): HTMLElement {
  if (listError) {
    const { e } = listError;
    return h('div', { class: 'err', role: 'alert' },
      h('p', null, t('records.loadFailed')),
      h('p', null, errorText(e)),
      h('div', { class: 'acts' }, h('button', { class: 'btn', type: 'button', onclick: () => { listError = null; render(); } }, t('common.retry'))));
  }
  if (!listLoading) {
    listLoading = true;
    refreshRecords().then(() => {
      listLoading = false;
      render();
    }, (e: unknown) => {
      listLoading = false;
      listError = { e };
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

function renderList(): HTMLElement {
  const all = S.records;
  const head = pageHead(t('records.title'),
    h('button', { class: 'btn', type: 'button', onclick: () => navigate({ name: 'wrong' }) }, t('records.reviewWrong')));
  const page = h('div', { class: 'page page-records' }, head);
  const retention = h('p', { class: 'rec-retention dim' }, t('records.retention', { n: S.recordsRetention }));

  if (!all) {
    page.append(listPlaceholder());
    return page;
  }
  if (!all.length) {
    page.append(
      h('section', { class: 'panel rec-empty' },
        h('h2', { tabindex: '-1' }, t('records.empty.title')),
        h('p', { class: 'empty' }, t('records.empty.body')),
        h('div', { class: 'acts' }, h('button', { class: 'btn primary', type: 'button', onclick: () => navigate({ name: 'home' }) }, t('records.empty.start')))),
      retention);
    restoreFocusAfterDelete(page);
    return page;
  }

  const banks = banksInRecords(all);
  if (filters.bank !== 'all' && !banks.some((b) => String(b.id) === filters.bank)) filters.bank = 'all';

  // Filter controls: changing one updates the summary and the list in place (no full re-render, focus stays put)
  const bankSel = h('select', null,
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
  const resetBtn = h('button', { class: 'btn quiet rec-reset', type: 'button', title: t('records.filter.reset'), onclick: clearFilters },
    miniIcon('x', 'rec-ri'), h('span', { class: 'rec-rtxt' }, t('records.filter.reset')));
  // Up to 640 px the filters fold behind this button (RC-1): [筛选 (n)] · the count · 清除筛选
  const toggleText = h('span', { class: 'rec-ftxt' });
  const toggle = h('button', {
    class: 'btn rec-ftoggle', type: 'button', 'aria-expanded': String(filtersOpen), 'aria-controls': 'rec-filters',
    onclick: () => {
      filtersOpen = !filtersOpen;
      filterPanel.classList.toggle('open', filtersOpen);
      toggle.setAttribute('aria-expanded', String(filtersOpen));
    },
  }, miniIcon('filter', 'rec-fi'), toggleText, miniIcon('chevDown', 'rec-fchev'));
  const listBox = h('div', { class: 'rec-list' });
  const deleteAllBtn = h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void removeAll() }, t('records.deleteAll'));
  const view = aiPlusNow();
  /** The rows on screen, in order (the neighbour of a deleted record is looked up here) */
  let shownIds: number[] = [];

  const onDelete = (r: RecordSummary): void => {
    void removeOne(r, undefined, focusAfterDelete(shownIds, r.id));
  };

  const update = (): void => {
    const rows = all.filter((r) => matchesRecord(r, filters));
    const { shown, rest } = pageRows(rows, shownLimit);
    shownIds = shown.map((r) => r.id);
    const n = filterCount(filters);
    summary.replaceChildren(...summaryContent(rows, all.length));
    count.textContent = tn('records.count', all.length, { shown: rows.length });
    toggleText.textContent = n ? `${t('records.filter.heading')} (${n})` : t('records.filter.heading');
    badRange.hidden = !(filters.from && filters.to && filters.from > filters.to);
    resetBtn.hidden = !isFiltered(filters);
    // "Delete all" would also delete what the filters hide: it is offered only on the full list (RC-11)
    deleteAllBtn.hidden = isFiltered(filters);
    if (!rows.length) {
      listBox.replaceChildren(h('div', { class: 'rec-nomatch' },
        h('p', { class: 'empty' }, t('records.noMatch')),
        h('button', { class: 'btn', type: 'button', onclick: clearFilters }, t('records.filter.reset'))));
      return;
    }
    const table = recordTable(shown, view, onDelete);
    const more = h('button', { class: 'btn rec-more', type: 'button' }, t('records.more', { n: rest }));
    more.hidden = rest <= 0;
    // "Show more" appends the next page to the table that is there (nothing above it is redrawn, the scroll stays)
    more.addEventListener('click', () => {
      const from = shownLimit;
      shownLimit = nextLimit(from, rows.length);
      const add = rows.slice(from, shownLimit);
      shownIds = rows.slice(0, shownLimit).map((r) => r.id);
      table.body.append(...add.map((r) => recordRow(r, view, onDelete)));
      const left = rows.length - shownLimit;
      if (left > 0) more.textContent = t('records.more', { n: left });
      else more.hidden = true;
      const first = add[0];
      if (first) table.body.querySelector<HTMLElement>(`a[data-rec="${first.id}"]`)?.focus();
    });
    listBox.replaceChildren(table.el, more);
  };
  const refilter = (): void => {
    shownLimit = RECORDS_PAGE;
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

  const filterPanel = h('section', { class: 'panel rec-filter' + (filtersOpen ? ' open' : ''), 'aria-labelledby': 'rec-filter-h' },
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

  const listPanel = h('section', { class: 'panel rec-listpanel' + (view.show ? ' has-ai' : ''), 'aria-labelledby': 'rec-list-h' },
    h('h2', { id: 'rec-list-h', tabindex: '-1' }, t('records.list.heading')),
    // When AI 总结 is greyed for everyone, one note says why (instead of a tag on every row)
    view.show ? aiNote(controlState(view)) : null,
    listBox);

  update();
  // "Delete all" is rarely needed: at the end of the page, away from the study actions in the page head
  page.append(summary, filterPanel, listPanel, h('div', { class: 'rec-foot' }, retention, deleteAllBtn));
  restoreFocusAfterDelete(page);
  return page;
}

// ---------------------------------------------------------------- detail (#/records/<id>)

function fetchDetail(id: number): void {
  pending.add(id);
  loadRecord(id).then(() => {
    failures.delete(id);
  }, (e: unknown) => {
    failures.set(id, e);
  }).finally(() => {
    pending.delete(id);
    if (S.route.name === 'record' && S.route.id === id) render();
  });
}

function detailError(id: number, e: unknown): HTMLElement {
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
        backButton(t('records.title'), { name: 'records' }))));
}

/** A question that no longer exists (deleted with its bank): a small card that still says whether it was right */
function goneCard(it: RecordItem, no: number): HTMLElement {
  const unanswered = !it.answer || it.answer.every((x) => x == null);
  const status = it.correct
    ? h('span', { class: 'badge ok' }, t('result.q.correct'))
    : h('span', { class: 'badge ng' }, unanswered ? t('result.q.unanswered') : t('result.q.wrong'));
  return h('article', { class: 'rv rv-gone' + (it.correct ? ' good' : ''), tabindex: '-1', 'aria-label': t('result.q.no', { n: no }) },
    h('div', { class: 'paper' },
      h('div', { class: 'phead' }, h('b', { class: 'num' }, t('result.q.no', { n: no })), status),
      h('p', { class: 'dim' }, t('records.detail.gone'))));
}

function detailView(d: RecordDetail): HTMLElement {
  const rec = d.record;
  const items = [...d.items].sort((a, b) => a.seq - b.seq);
  const active = activeById();
  const wrongItems = items.filter((it) => !it.correct);
  const wrongIds = uniq(wrongItems.map((it) => it.questionId));
  const allIds = uniq(items.map((it) => it.questionId));
  const retryQs = wrongIds.map((id) => active.get(id)).filter(isQuestion);
  const retakeQs = allIds.map((id) => active.get(id)).filter(isQuestion);
  const mode: Mode = isMode(rec.mode) ? rec.mode : 'all';
  // Retry and retake start the same kind as the record: a practice is practised again
  const kind = kindOf(rec);
  const hasDetail = rec.detail && items.length > 0;

  const pair = (label: string, value: string): HTMLElement[] => [h('dt', null, label), h('dd', null, value)];
  const meta = h('dl', { class: 'rec-meta props' },
    pair(t('records.detail.time'), rec.at),
    [h('dt', null, t('records.detail.kind')), h('dd', null, kindTag(kind))],
    pair(t('records.detail.banks'), recordBankNames(rec.banks)),
    rec.sheets.length ? pair(t('records.detail.sheets'), rec.sheets.join(t('common.listSep'))) : null,
    modeName(rec.mode) ? pair(t('records.detail.mode'), modeName(rec.mode)) : null,
    pair(t('records.detail.duration'), fmtTime(rec.seconds)),
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
  if (hasDetail) {
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
  const aiBtn = summaryControl(rec, controlState(aiPlusNow(), recordSummaryExtra(rec)));
  // The red trash button, set apart from the study actions (the dialog still asks before anything is deleted)
  const delBtn = labelled(
    deleteButton(rec, () => void removeOne(rec, () => navigate({ name: 'records' })), t('records.detail.delete')),
    t('records.detail.delete'));
  delBtn.classList.add('rec-del');

  const page = h('div', { class: 'page page-records page-record' },
    pageHead(t(kind === 'practice' ? 'records.detail.titlePractice' : 'records.detail.title'), backButton(t('records.title'), { name: 'records' })),
    h('section', { class: 'score rec-score' },
      h('div', { class: 'big num', role: 'img', 'aria-label': t('records.detail.scoreLabel', { pct: rec.percent }) }, String(rec.percent), h('small', null, '%')),
      h('div', { class: 'says' },
        h('p', null, t('records.detail.summary', { correct: rec.correct, total: rec.total })),
        meta,
        h('div', { class: 'acts rec-acts' }, retryBtn, retakeBtn, aiBtn, delBtn),
        hints.length ? h('div', { class: 'rec-hints' }, hints.map((x) => h('p', null, x))) : null)));

  if (!hasDetail) {
    page.append(h('p', { class: 'rec-note' }, t('records.detail.noDetail')));
    return page;
  }

  const rows: ReviewRow[] = items.map((it, i) => ({
    correct: it.correct,
    render: () => (it.question
      ? reviewItem({ q: it.question, ans: it.answer, correct: it.correct }, {
        no: i + 1,
        retired: !it.question.active,
        // v0.9: AI analysis of this record's questions (the controls build themselves only for users with the AI+ feature)
        ai: { page: 'detail', mode: kind, key: rec.sessionKey ?? null, startedAt: rec.startedAt ?? null, recordId: rec.id },
      })
      : goneCard(it, i + 1)),
  }));
  const parts = reviewParts(rows, { get: () => detailFilter, set: (f) => { detailFilter = f; } });
  page.append(...parts.els);
  // After reading through the answers, practise the wrong ones without scrolling back up
  if (wrongIds.length && retryQs.length) page.append(h('div', { class: 'acts wr-foot' }, retryButton()));
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
  const keys = h('div', { class: 'strip', role: 'group', 'aria-label': t('result.strip.label') }, rows.map((r, i) => h('button', {
    class: r.correct ? 'ok' : 'ng', type: 'button',
    'aria-label': t('result.strip.item', { n: i + 1, verdict: r.correct ? t('result.q.correct') : t('result.q.wrong') }),
    onclick: () => {
      close();
      parts.jump(i);
    },
  }, h('span', { class: 'n' }, i + 1), svgMark(r.correct))));
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

function renderDetail(id: number): HTMLElement {
  const d = S.recordCache.get(id);
  if (d) return detailView(d);
  if (failures.has(id)) return detailError(id, failures.get(id));
  if (!pending.has(id)) fetchDetail(id);
  return h('div', { class: 'page page-records' },
    pageHead(t('records.detail.title'), backButton(t('records.title'), { name: 'records' })),
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
    pageHead(t('records.reviewWrong'), backButton(t('records.title'), { name: 'records' }), all.length ? practise() : null));

  if (!all.length) {
    page.append(h('section', { class: 'panel rec-empty' },
      h('h2', null, t('records.wrong.empty.title')),
      h('p', { class: 'empty' }, t('records.wrong.empty.body')),
      h('p', { class: 'dim' }, tn('records.wrong.intro', ms)),
      h('div', { class: 'acts' }, h('button', { class: 'btn primary', type: 'button', onclick: () => navigate({ name: 'home' }) }, t('records.empty.start')))));
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
    const sel = h('select', null,
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
  if (route.name === 'record') return renderDetail(route.id);
  if (route.name === 'wrong') return renderWrong();
  return renderList();
}
