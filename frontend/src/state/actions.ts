// Actions that change application state and trigger a re-render. Page files only call these and never import each other,
// which avoids circular dependencies. The renderer is registered via setRenderer() (done in main.ts).
import { submitAttempt } from '../api/progress.ts';
import { fetchBank, fetchBanks, fetchInfo } from '../api/bank.ts';
import { fetchProgress, fetchRecord, fetchRecords } from '../api/progress.ts';
import { completes, grade, percentOf, stateOf, type Answer } from '../domain/grading.ts';
import { t, tn } from '../i18n/index.ts';
import { errorText } from '../i18n/errors.ts';
import { parseHash, routeHash, sameRoute, type Route } from '../router.ts';
import type { BankDeleted, Mode, Question, QuestionLang, RecordDetail, RecordsDeleted } from '../types/api.ts';
import { ask } from '../utils/dialog.ts';
import { toast } from '../utils/toast.ts';
import { loadSettings, migrateBrowserQuestionLang, updateSettings } from './preferences.ts';
import { clearSaved, loadSaved, saveSession as persist } from './session-storage.ts';
import { S, setBankData } from './store.ts';

let renderer: () => void = () => {};
let refreshers: Array<() => void> = [];
const routeHooks: Array<(from: Route, to: Route) => void> = [];

export const setRenderer = (fn: () => void): void => {
  renderer = fn;
};
export const render = (): void => renderer();

/** Callbacks on the answer page that "only refresh state, not redraw the DOM" (option highlight, answer sheet, progress bar). Cleared before every re-render. */
export const resetRefreshers = (): void => {
  refreshers = [];
};
export const onRefresh = (fn: () => void): void => {
  refreshers.push(fn);
};
export const runRefreshers = (): void => refreshers.forEach((f) => f());

/** Called whenever the route changes (for example the banks page cancels a pending upload preview when it is left). */
export const onRouteChange = (fn: (from: Route, to: Route) => void): void => {
  routeHooks.push(fn);
};

export const elapsed = (): number => {
  const se = S.session;
  return se ? se.base + (Date.now() - se.t0) / 1000 : 0;
};

const scrollTop = (): void => window.scrollTo(0, 0);

export function persistSession(): void {
  const se = S.session;
  if (!se) return;
  persist({ qs: se.qs, answers: se.answers, flags: se.flags, idx: se.idx, elapsedSeconds: elapsed(), mode: se.mode, sheets: se.sheets });
}

// ---------------------------------------------------------------- routing

/** Brings S.route in line with the state: no exam -> no quiz page, no result -> no result page. */
function settle(r: Route): Route {
  if (r.name === 'quiz' && !S.session) {
    // A reload on the quiz page: continue the saved exam if there is one
    const sv = S.phase === 'ready' ? loadSaved(S.bank) : null;
    if (sv) {
      S.session = { qs: sv.qs, answers: sv.answers, flags: sv.flags, idx: sv.idx, base: sv.elapsed, t0: Date.now(), mode: sv.mode, sheets: sv.sheets };
      return r;
    }
    return { name: 'home' };
  }
  if (r.name === 'result' && !S.result) return { name: 'records' };
  return r;
}

function switchTo(next: Route): void {
  const from = S.route;
  if (from.name === 'quiz' && next.name !== 'quiz' && S.session) {
    // Leaving the quiz in any way (back button, link): keep the answers
    persistSession();
    S.session = null;
  }
  cancelAdvance();
  S.route = next;
  S.navOpen = false;
  for (const fn of routeHooks) fn(from, next);
}

function onHashChange(): void {
  const wanted = parseHash(location.hash);
  const next = settle(wanted);
  if (!sameRoute(next, wanted)) history.replaceState(null, '', routeHash(next));
  if (sameRoute(next, S.route)) {
    render();
    return;
  }
  switchTo(next);
  render();
  scrollTop();
}

/** Go to a page. Same page: just re-render. */
export function navigate(r: Route): void {
  const hash = routeHash(r);
  if (location.hash === hash || (hash === '#/' && (location.hash === '' || location.hash === '#'))) {
    if (!sameRoute(S.route, r)) {
      switchTo(settle(r));
      scrollTop();
    }
    S.navOpen = false;
    render();
    return;
  }
  location.hash = hash; // -> hashchange -> onHashChange
}

export function setNavOpen(open: boolean): void {
  S.navOpen = open;
  render();
}

// ---------------------------------------------------------------- loading

/** Loads everything the pages need. Any single failure goes to the fatal page. */
export async function loadAll(): Promise<void> {
  const [info, bank, banks, progress, records] = await Promise.all([
    fetchInfo(), fetchBank(), fetchBanks(), fetchProgress(), fetchRecords(), loadSettings(),
  ]);
  S.info = info;
  S.progress = progress;
  setBankData(bank, banks);
  S.records = records.records;
  S.recordsRetention = records.retention;
}

export async function boot(): Promise<void> {
  window.addEventListener('hashchange', onHashChange);
  S.phase = 'loading';
  S.route = parseHash(location.hash);
  render();
  try {
    await loadAll();
    S.phase = 'ready';
    void migrateBrowserQuestionLang().then(render);
  } catch (e) {
    S.phase = 'fatal';
    S.fatal = e;
  }
  const next = settle(S.route);
  if (!sameRoute(next, S.route)) history.replaceState(null, '', routeHash(next));
  S.route = next;
  render();
}

/** Re-reads the questions and the bank list (after an import). */
export async function refreshLibrary(): Promise<void> {
  const [bank, banks] = await Promise.all([fetchBank(), fetchBanks()]);
  setBankData(bank, banks);
  // Exam detail shows the questions as stored now; an import may have changed them
  S.recordCache.clear();
}

export async function refreshRecords(): Promise<void> {
  const r = await fetchRecords();
  S.records = r.records;
  S.recordsRetention = r.retention;
}

/** Exam detail, cached per record */
export async function loadRecord(id: number): Promise<RecordDetail> {
  const hit = S.recordCache.get(id);
  if (hit) return hit;
  const d = await fetchRecord(id);
  S.recordCache.set(id, d);
  return d;
}

/** After POST /api/banks/delete */
export function applyBankDeleted(d: BankDeleted): void {
  setBankData(d.bank, d.banks);
  S.progress = d.progress;
  S.recordCache.clear();
  // Records may have been removed or now point at a deleted bank
  refreshRecords().then(render, () => {});
}

/** After POST /api/records/delete */
export function applyRecordsDeleted(d: RecordsDeleted): void {
  S.records = d.records;
  S.recordsRetention = d.retention;
  S.progress = d.progress;
  const keep = new Set(d.records.map((r) => r.id));
  for (const id of [...S.recordCache.keys()]) if (!keep.has(id)) S.recordCache.delete(id);
}

// ---------------------------------------------------------------- exams

export function startSession(qs: Question[], mode: Mode): void {
  S.session = {
    qs, answers: {}, flags: new Set(), idx: 0, base: 0, t0: Date.now(), mode,
    sheets: [...new Set(qs.map((q) => q.sheet))],
  };
  persistSession();
  navigate({ name: 'quiz' });
}

/** Starts an exam; if an unsubmitted exam is saved, asks before replacing it. Used by home, result and records pages. */
export async function startExam(qs: Question[], mode: Mode): Promise<void> {
  if (!qs.length) return;
  if (loadSaved(S.bank) && !(await ask({
    title: t('act.overwrite.title'), body: t('act.overwrite.body'), ok: t('act.overwrite.ok'), cancel: t('act.overwrite.cancel'), danger: true,
  }))) return;
  startSession(qs, mode);
}

export function resumeSession(): void {
  const sv = loadSaved(S.bank);
  if (!sv) {
    render();
    return;
  }
  S.session = { qs: sv.qs, answers: sv.answers, flags: sv.flags, idx: sv.idx, base: sv.elapsed, t0: Date.now(), mode: sv.mode, sheets: sv.sheets };
  if (sv.dropped) toast(tn('act.resume.dropped', sv.dropped));
  navigate({ name: 'quiz' });
}

/** "Save and exit" on the quiz page */
export function leaveQuiz(): void {
  navigate({ name: 'home' });
}

export function go(i: number): void {
  const se = S.session;
  if (!se || i < 0 || i >= se.qs.length) return;
  cancelAdvance();
  se.idx = i;
  persistSession();
  render();
  scrollTop();
}

/** The language the question text is shown in (saved on the server like the other settings). */
export function setQuestionLang(code: QuestionLang): void {
  if (S.settings.questionLang === code) return;
  const y = window.scrollY;
  const rerender = (): void => {
    render(); // only swaps the text; answers, flags and the timer live in S.session and are unaffected
    window.scrollTo(0, y);
  };
  updateSettings({ questionLang: code }, rerender).catch((e: unknown) => toast(t('act.settings.saveFailed', { reason: errorText(e) }), 6000));
}

function syncQuiz(): void {
  runRefreshers();
  persistSession();
}

// Auto-advance (setting): single choice after a pick, hotspot when its last open item gets a choice; never multi-choice, never on deselect.
let advanceTimer: number | undefined;
const ADVANCE_DELAY_MS = 350;

function cancelAdvance(): void {
  if (advanceTimer !== undefined) window.clearTimeout(advanceTimer);
  advanceTimer = undefined;
}

function scheduleAdvance(): void {
  const se = S.session;
  if (!se || !S.settings.autoAdvance || se.idx >= se.qs.length - 1) return;
  const idx = se.idx;
  cancelAdvance();
  advanceTimer = window.setTimeout(() => {
    advanceTimer = undefined;
    if (S.session === se && se.idx === idx && S.route.name === 'quiz' && !document.querySelector('dialog[open]')) go(idx + 1);
  }, ADVANCE_DELAY_MS);
}

export function pick(q: Question, key: string): void {
  const se = S.session;
  if (!se) return;
  let a = (se.answers[q.id] as string[] | undefined) ?? [];
  const adding = !a.includes(key);
  if (q.kind === 'single') a = adding ? [key] : [];
  else a = adding ? [...a, key].sort() : a.filter((k) => k !== key);
  if (a.length) se.answers[q.id] = a;
  else delete se.answers[q.id];
  syncQuiz();
  if (q.kind === 'single' && adding) scheduleAdvance();
  else cancelAdvance();
}

export function pickItem(q: Question, i: number, j: number): void {
  const se = S.session;
  if (!se) return;
  const cur = se.answers[q.id];
  const a: Array<number | null> = Array.isArray(cur) ? (cur.slice() as Array<number | null>) : Array<number | null>(q.items.length).fill(null);
  const adding = a[i] !== j;
  a[i] = adding ? j : null;
  if (a.every((x) => x == null)) delete se.answers[q.id];
  else se.answers[q.id] = a;
  syncQuiz();
  // Only the pick that fills the last open item advances; correcting an item of a complete question stays on it
  if (adding && completes(q, cur, se.answers[q.id])) scheduleAdvance();
  else cancelAdvance();
}

export function toggleFlag(q: Question): void {
  const se = S.session;
  if (!se) return;
  if (se.flags.has(q.id)) se.flags.delete(q.id);
  else se.flags.add(q.id);
  syncQuiz();
}

let submitting = false;

/** Submit the exam: confirm first, then save the result; the saved exam is cleared only after saving succeeded (on failure the answers are still there, so it can be submitted again). */
export async function submit(): Promise<void> {
  const se = S.session;
  if (!se || submitting) return;
  cancelAdvance();
  const open = se.qs.filter((q) => stateOf(q, se.answers[q.id] as Answer) < 2).length;
  const go_ = await ask(open
    ? { title: tn('act.submit.openTitle', open), body: t('act.submit.openBody'), ok: t('act.submit.ok'), cancel: t('act.submit.back') }
    : { title: t('act.submit.confirmTitle'), body: t('act.submit.confirmBody'), ok: t('act.submit.ok'), cancel: t('act.submit.check') });
  if (!go_ || S.session !== se) return;

  submitting = true;
  try {
    const results = se.qs.map((q) => {
      const ans = (Array.isArray(se.answers[q.id]) ? se.answers[q.id] : null) as Answer;
      return { q, ans, correct: grade(q, ans) };
    });
    const seconds = Math.floor(elapsed());
    let prev = null;
    let saved = false;
    try {
      S.progress = await submitAttempt({
        results: results.map((r) => ({ id: r.q.id, correct: r.correct, answer: Array.isArray(r.ans) ? r.ans : null })),
        sheets: se.sheets, mode: se.mode, seconds,
      });
      saved = true;
      clearSaved();
      const second = S.progress.history[1];
      if (second && second.mode === se.mode) prev = second;
    } catch (e) {
      toast(`${t('act.submit.saveFailed')} (${errorText(e)})`, 9000);
    }
    S.session = null;
    S.result = { results, seconds, mode: se.mode, prev, recordId: null };
    S.rvFilter = 'wrong';
    navigate({ name: 'result' });
    if (saved) {
      // The newest record is this exam (single user); remember its id so the result page can link to it
      const good = results.filter((r) => r.correct).length;
      const mine = S.result;
      refreshRecords().then(() => {
        const top = S.records?.[0];
        if (mine && S.result === mine && top && top.total === results.length && top.correct === good) mine.recordId = top.id;
        render();
      }, () => {});
    }
  } finally {
    submitting = false;
  }
}

export const resultPercent = (good: number, total: number): number => percentOf(good, total);
