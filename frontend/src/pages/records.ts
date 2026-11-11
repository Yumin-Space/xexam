// Study records: the list of exams and (v0.6.5) practice sessions with filters (#/records), one record in detail
// (#/records/<id>) and the wrong-answer review (#/records/wrong). Page-local UI state (filters, detail loading, the review
// filter) lives in this module; the filter rules are in domain/records-filter.ts.
import '../styles/records.css';
import { ApiError } from '../api/client.ts';
import { deleteAllRecords, deleteRecord } from '../api/progress.ts';
import { bankName, modeName, pageHead, recordBankNames } from '../components/common.ts';
import { aiPlusNow, aiTableNote, summaryButton } from '../components/ai-plus-controls.ts';
import { kindTag } from '../components/kind-tag.ts';
import { controlState, recordSummaryExtra, type AiPlusView } from '../domain/ai-plus.ts';
import { kindOf } from '../domain/attempt.ts';
import { NO_FILTERS, isFiltered, isKindFilter, matchesRecord, type RecordFilters } from '../domain/records-filter.ts';
import { reviewBlock, reviewItem, type ReviewFilter, type ReviewRow } from '../components/review.ts';
import { inNotebook, masterStreakOf, percentOf, shuffle } from '../domain/grading.ts';
import { errorText } from '../i18n/errors.ts';
import { t, tn } from '../i18n/index.ts';
import { routeHash, type Route } from '../router.ts';
import { applyRecordsDeleted, loadRecord, navigate, onRouteChange, onSignedIn, refreshRecords, render, startExam } from '../state/actions.ts';
import { S, bankIdsOf } from '../state/store.ts';
import type { Mode, Question, RecordBank, RecordDetail, RecordItem, RecordSummary } from '../types/api.ts';
import { ask, askCheck } from '../utils/dialog.ts';
import { h } from '../utils/dom.ts';
import { fmtTime } from '../utils/format.ts';
import { toast } from '../utils/toast.ts';

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

onRouteChange((from, to) => {
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

/** A "back" link in the page head (a real link: it can also be opened in a new tab) */
function backButton(label: string, to: Route): HTMLElement {
  return h('a', { class: 'btn quiet rec-back', href: routeHash(to) }, h('span', { 'aria-hidden': 'true' }, '←'), label);
}

// ---------------------------------------------------------------- deleting

async function removeOne(r: RecordSummary, after?: () => void): Promise<void> {
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
    applyRecordsDeleted(await deleteRecord(r.id));
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

/** The AI summary cell of a row (v0.9): only for users with the AI+ feature; the one note above the table carries the tag */
function aiCell(r: RecordSummary, view: AiPlusView): HTMLElement {
  const btn = summaryButton({ id: r.id, at: r.at, kind: kindOf(r) }, controlState(view, recordSummaryExtra(r)),
    { anchorKey: `rec-ai-${r.id}`, quiet: true, tag: false });
  return h('td', { role: 'cell', class: 'rec-ai' }, btn);
}

function recordRow(r: RecordSummary, view: AiPlusView): HTMLElement {
  const cell = (label: string, cls: string, ...kids: Array<Node | string | null>): HTMLElement =>
    h('td', { role: 'cell', class: cls, 'data-label': label }, kids);
  const mode = modeName(r.mode);
  const band = r.percent >= 80 ? ' hi' : r.percent < 60 ? ' lo' : '';
  return h('tr', {
    role: 'row',
    // The whole row opens the detail; the link in the first cell is the keyboard / screen-reader way in
    onclick: (e: MouseEvent) => {
      if (e.target instanceof Element && e.target.closest('a, button')) return;
      navigate({ name: 'record', id: r.id });
    },
  },
  view.show ? aiCell(r, view) : null,
  cell(t('records.col.time'), 'rec-time num', h('a', { href: `#/records/${r.id}` }, r.at)),
  // The kind tag leads the cell, so exams and practice tell apart in the list and in the phone cards alike
  cell(t('records.col.banks'), 'rec-banks', kindTag(kindOf(r)), ' ', recordBankNames(r.banks), mode ? h('span', { class: 'rec-mode' }, mode) : null),
  cell(t('records.col.answered'), 'num', `${r.answered ?? t('common.none')} / ${r.total}`),
  cell(t('records.col.duration'), 'num', fmtTime(r.seconds)),
  cell(t('records.col.score'), 'num', `${r.correct} / ${r.total}`),
  cell(t('records.col.accuracy'), 'num rec-pct' + band, `${r.percent}%`),
  h('td', { role: 'cell', class: 'rec-act' },
    h('button', {
      class: 'btn quiet rec-del', type: 'button', disabled: busy,
      'aria-label': t('records.row.deleteLabel', { at: r.at }),
      onclick: () => void removeOne(r),
    }, t('common.delete'))));
}

function recordTable(rows: readonly RecordSummary[], view: AiPlusView): HTMLElement {
  const th = (label: string, hidden = false): HTMLElement =>
    h('th', { role: 'columnheader', scope: 'col' }, hidden ? h('span', { class: 'sr-only' }, label) : label);
  return h('div', { class: 'rec-scroll' },
    h('table', { class: 'rec-table' + (view.show ? ' has-ai' : ''), role: 'table' },
      h('caption', { class: 'sr-only' }, t('records.table.caption')),
      h('thead', { role: 'rowgroup' }, h('tr', { role: 'row' },
        view.show ? th(t('aiplus.summary'), true) : null,
        th(t('records.col.time')), th(t('records.col.banks')), th(t('records.col.answered')), th(t('records.col.duration')),
        th(t('records.col.score')), th(t('records.col.accuracy')), th(t('records.col.actions'), true))),
      h('tbody', { role: 'rowgroup' }, rows.map((r) => recordRow(r, view)))));
}

/** A labelled filter control (explicit for/id, so the accessible name is just the label) */
function field(id: string, label: string, control: HTMLElement): HTMLElement {
  control.id = id;
  return h('div', { class: 'rec-field' }, h('label', { for: id }, label), control);
}

function clearFilters(): void {
  Object.assign(filters, NO_FILTERS);
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
        h('h2', null, t('records.empty.title')),
        h('p', { class: 'empty' }, t('records.empty.body')),
        h('div', { class: 'acts' }, h('button', { class: 'btn primary', type: 'button', onclick: () => navigate({ name: 'home' }) }, t('records.empty.start')))),
      retention);
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
  const resetBtn = h('button', { class: 'btn quiet', type: 'button', onclick: clearFilters }, t('records.filter.reset'));
  const listBox = h('div', { class: 'rec-list' });
  const view = aiPlusNow();

  const update = (): void => {
    const rows = all.filter((r) => matchesRecord(r, filters));
    summary.replaceChildren(...summaryContent(rows, all.length));
    count.textContent = tn('records.count', all.length, { shown: rows.length });
    badRange.hidden = !(filters.from && filters.to && filters.from > filters.to);
    resetBtn.hidden = !isFiltered(filters);
    listBox.replaceChildren(rows.length
      ? recordTable(rows, view)
      : h('div', { class: 'rec-nomatch' },
        h('p', { class: 'empty' }, t('records.noMatch')),
        h('button', { class: 'btn', type: 'button', onclick: clearFilters }, t('records.filter.reset'))));
  };
  bankSel.addEventListener('change', () => {
    filters.bank = bankSel.value;
    update();
  });
  kindSel.addEventListener('change', () => {
    filters.kind = isKindFilter(kindSel.value) ? kindSel.value : 'all';
    update();
  });
  fromIn.addEventListener('change', () => {
    filters.from = fromIn.value;
    update();
  });
  toIn.addEventListener('change', () => {
    filters.to = toIn.value;
    update();
  });
  scoreSel.addEventListener('change', () => {
    const v = scoreSel.value;
    filters.score = v === 'low' || v === 'mid' || v === 'high' ? v : 'any';
    update();
  });

  const filterPanel = h('section', { class: 'panel rec-filter', 'aria-labelledby': 'rec-filter-h' },
    h('h2', { id: 'rec-filter-h' }, t('records.filter.heading')),
    // Type and bank first, so that From and To share a row in the two-column phone layout (accuracy comes last)
    h('div', { class: 'rec-filters' },
      field('rec-f-kind', t('records.filter.kind'), kindSel),
      field('rec-f-bank', t('records.filter.bank'), bankSel),
      field('rec-f-from', t('records.filter.from'), fromIn),
      field('rec-f-to', t('records.filter.to'), toIn),
      field('rec-f-score', t('records.filter.score'), scoreSel)),
    badRange,
    h('div', { class: 'rec-bar' }, count, resetBtn));

  const listPanel = h('section', { class: 'panel rec-listpanel' + (view.show ? ' has-ai' : ''), 'aria-labelledby': 'rec-list-h' },
    h('h2', { id: 'rec-list-h' }, t('records.list.heading')),
    // v0.9: when the AI summary is greyed for everyone, one note says why (instead of a tag on every row)
    view.show ? aiTableNote(controlState(view)) : null,
    listBox);

  update();
  // "Delete all" is rarely needed: at the end of the page, away from the study actions in the page head
  const deleteAllBtn = h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void removeAll() }, t('records.deleteAll'));
  page.append(summary, filterPanel, listPanel, h('div', { class: 'rec-foot' }, retention, deleteAllBtn));
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
        backButton(t('records.detail.back'), { name: 'records' }))));
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
  if (hasDetail) {
    if (wrongIds.length) {
      retryBtn = retryButton();
      if (!retryQs.length) hints.push(t('records.detail.retryNone'));
      else if (retryQs.length < wrongIds.length) hints.push(tn('records.detail.retrySkip', wrongIds.length - retryQs.length));
    }
    retakeBtn = h('button', {
      class: wrongIds.length && retryQs.length ? 'btn' : 'btn primary', type: 'button', disabled: !retakeQs.length,
      onclick: () => void startExam(shuffle(retakeQs), mode, kind),
    }, t(kind === 'practice' ? 'records.detail.retakePractice' : 'records.detail.retake'));
    if (!retakeQs.length) hints.push(t('records.detail.retakeNone'));
    else if (retakeQs.length < allIds.length) hints.push(tn('records.detail.retakeSkip', allIds.length - retakeQs.length));
  }
  // A quiet text button, set apart from the study actions (the dialog still asks before anything is deleted)
  const delBtn = h('button', {
    class: 'btn quiet rec-del', type: 'button', disabled: busy,
    onclick: () => void removeOne(rec, () => navigate({ name: 'records' })),
  }, t('records.detail.delete'));

  const page = h('div', { class: 'page page-records page-record' },
    pageHead(t(kind === 'practice' ? 'records.detail.titlePractice' : 'records.detail.title'), backButton(t('records.detail.back'), { name: 'records' })),
    h('section', { class: 'score rec-score' },
      h('div', { class: 'big num', role: 'img', 'aria-label': t('records.detail.scoreLabel', { pct: rec.percent }) }, String(rec.percent), h('small', null, '%')),
      h('div', { class: 'says' },
        h('p', null, t('records.detail.summary', { correct: rec.correct, total: rec.total })),
        meta,
        h('div', { class: 'acts' }, retryBtn, retakeBtn, delBtn),
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
  page.append(...reviewBlock(rows, { get: () => detailFilter, set: (f) => { detailFilter = f; } }));
  // After reading through the answers, practise the wrong ones without scrolling back up
  if (wrongIds.length && retryQs.length) page.append(h('div', { class: 'acts wr-foot' }, retryButton()));
  return page;
}

function renderDetail(id: number): HTMLElement {
  const d = S.recordCache.get(id);
  if (d) return detailView(d);
  if (failures.has(id)) return detailError(id, failures.get(id));
  if (!pending.has(id)) fetchDetail(id);
  return h('div', { class: 'page page-records' },
    pageHead(t('records.detail.title'), backButton(t('records.detail.back'), { name: 'records' })),
    h('p', { class: 'dim', role: 'status' }, t('records.detail.loading')));
}

// ---------------------------------------------------------------- wrong-answer review (#/records/wrong)

/** Wrong count, attempts, current streak, and how many more correct answers in a row take it out of the notebook */
function notebookStats(q: Question, masterStreak: number): HTMLElement | null {
  const st = S.progress?.questions?.[q.id];
  if (!st) return null;
  return h('div', { class: 'stats wr-stats' },
    h('p', null, t('records.wrong.stats', { wrong: st.wrong, attempts: st.attempts, streak: st.streak })),
    h('p', { class: 'wr-left' }, tn('records.wrong.left', Math.max(1, masterStreak - st.streak))));
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
    pageHead(t('records.wrong.title'), backButton(t('records.wrong.back'), { name: 'records' }), all.length ? practise() : null));

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
