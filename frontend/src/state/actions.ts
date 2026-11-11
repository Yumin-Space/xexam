// Actions that change application state and trigger a re-render. Page files only call these and never import each other,
// which avoids circular dependencies. The renderer is registered via setRenderer() (done in main.ts).
//
// Sign-in (v0.6): boot asks GET /api/auth/state first. Nobody signed in -> the sign-in page; a default or initial password
// -> the "choose a new password" page; otherwise everything is loaded as before. Whenever a request answers 401 the sign-in
// page comes back (the address keeps the route, which opens again after signing in); a 403 "choose a new password first"
// brings that page. A different user signing in on a page that still holds someone's data reloads the page, so nothing of
// the previous user (data, page state, unfinished exam) survives in memory.
import { changePassword, fetchAuthState, login, logout } from '../api/auth.ts';
import { ApiError, isSignInError, setAuthHandler, type AuthProblem } from '../api/client.ts';
import { submitAttempt } from '../api/progress.ts';
import { fetchBank, fetchBanks, fetchInfo } from '../api/bank.ts';
import { fetchProgress, fetchRecord, fetchRecords } from '../api/progress.ts';
import { countedResults, kindOf } from '../domain/attempt.ts';
import { completes, percentOf } from '../domain/grading.ts';
import { checksOnPick, finalResults, isLocked, openCount } from '../domain/practice.ts';
import { t, tn } from '../i18n/index.ts';
import { errorText } from '../i18n/errors.ts';
import { adminOnly, parseHash, routeHash, sameRoute, type Route } from '../router.ts';
import type { AttemptKind, AuthUser, BankDeleted, Mode, Question, QuestionLang, RecordDetail, RecordsDeleted, UiLang } from '../types/api.ts';
import { ask } from '../utils/dialog.ts';
import { toast } from '../utils/toast.ts';
import { loadSettings, migrateBrowserQuestionLang, showCachedSettings, updateSettings } from './preferences.ts';
import { adoptLegacySession, clearSaved, loadSaved, saveSession as persist, type SavedSession } from './session-storage.ts';
import { DEFAULT_SETTINGS, cachedSettingsOf, carryLanguage, rememberUser } from './settings-cache.ts';
import { paths, setStorageOwner } from './storage-keys.ts';
import { S, clearUserData, isAdmin, setBankData, type Session } from './store.ts';

let renderer: () => void = () => {};
let refreshers: Array<() => void> = [];
const routeHooks: Array<(from: Route, to: Route) => void> = [];
const signedInHooks: Array<() => void> = [];

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

/**
 * Called after a sign-in that keeps the page (the same user signed in again, the route is unchanged): pages forget what failed
 * while the session was gone (a list that answered 401, an error of a form), so they load afresh instead of showing it.
 */
export const onSignedIn = (fn: () => void): void => {
  signedInHooks.push(fn);
};

/**
 * Changes whenever the person at this page may have changed (signing in, signing out, a lost session). The sign-in pages
 * forget what was typed into their forms when it changes, so nothing typed by one person is shown to the next.
 */
let epoch = 0;
export const authEpoch = (): number => epoch;

export const elapsed = (): number => {
  const se = S.session;
  return se ? se.base + (Date.now() - se.t0) / 1000 : 0;
};

const scrollTop = (): void => window.scrollTo(0, 0);

export function persistSession(): void {
  const se = S.session;
  if (!se) return;
  persist({
    qs: se.qs, answers: se.answers, flags: se.flags, idx: se.idx, elapsedSeconds: elapsed(), mode: se.mode, sheets: se.sheets,
    kind: se.kind, checked: se.checked,
  });
}

/** The session to continue from what was saved in this browser (the timer goes on from the saved time) */
function sessionOf(sv: SavedSession): Session {
  return {
    qs: sv.qs, answers: sv.answers, flags: sv.flags, idx: sv.idx, base: sv.elapsed, t0: Date.now(), mode: sv.mode, sheets: sv.sheets,
    kind: sv.kind, checked: sv.checked,
  };
}

// ---------------------------------------------------------------- routing

/** Brings S.route in line with the state: no exam -> no quiz page, no result -> no result page, admin pages -> admins only. */
function settle(r: Route): Route {
  if (adminOnly(r) && !isAdmin()) return { name: 'home' };
  if (r.name === 'quiz' && !S.session) {
    // A reload on the quiz page: continue the saved exam if there is one
    const sv = S.phase === 'ready' ? loadSaved(S.bank) : null;
    if (sv) {
      S.session = sessionOf(sv);
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
  checkSession();
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

// ---------------------------------------------------------------- signing in and out

/** The user whose data (banks, progress, records, page state) is in memory; null before the first load */
let dataOwner: number | null = null;
/** Set after signing out: the next sign-in starts from a fresh page, so no page keeps anything of the previous user */
let freshStart = false;
/**
 * The language the sign-in pages were shown in, kept for a user who signs in on this browser for the first time: when their
 * server settings are still all defaults (a new account), that language becomes theirs instead of the default English.
 */
let firstVisitLang: UiLang | null = null;

/** Makes `u` the signed-in user: their browser data (unfinished exam, settings cache) and, if cached, their theme and language. */
function adoptUser(u: AuthUser): void {
  if (S.user?.id !== u.id) epoch += 1;
  S.user = u;
  setStorageOwner(u.id);
  rememberUser(u.id);
  adoptLegacySession(u.id, u.role === 'admin');
  const cached = cachedSettingsOf(u.id);
  firstVisitLang = cached ? null : S.settings.uiLang;
  if (cached) showCachedSettings(cached);
}

/** A new account signing in for the first time keeps the language of the sign-in page (see firstVisitLang). */
async function keepFirstVisitLang(): Promise<void> {
  const lang = firstVisitLang;
  firstVisitLang = null;
  const s = S.settings;
  const untouched = s.theme === DEFAULT_SETTINGS.theme && s.uiLang === DEFAULT_SETTINGS.uiLang
    && s.autoAdvance === DEFAULT_SETTINGS.autoAdvance && s.questionLang === DEFAULT_SETTINGS.questionLang;
  if (!lang || lang === s.uiLang || !untouched) return;
  try {
    await updateSettings({ uiLang: lang }, () => {});
  } catch {
    /* the default stays; it can be changed in Settings */
  }
}

function closeDialogs(): void {
  document.querySelectorAll<HTMLDialogElement>('dialog[open]').forEach((d) => d.close('cancel'));
}

/** The sign-in page's first-start hint: asks the server again (an administrator may have replaced the default password). */
function refreshSignInHint(): void {
  fetchAuthState().then((st) => {
    const next = st.user ? null : st.defaultAdmin ?? null;
    if (next !== S.defaultAdmin) {
      S.defaultAdmin = next;
      if (S.authView === 'signin') render();
    }
  }, () => {});
}

/** A request showed that the user must sign in again (401) or choose a new password first (403). */
function onAuthProblem(problem: AuthProblem): void {
  if (problem === 'signin') {
    if (S.authView === 'signin') return;
    // Keep the answers of an exam in progress: after signing in again it can be continued
    if (S.route.name === 'quiz' && S.session) persistSession();
    S.authNote = S.user ? 'expired' : null;
    S.authView = 'signin';
    epoch += 1;
    refreshSignInHint();
  } else {
    if (S.authView) return;
    if (S.user) S.user = { ...S.user, mustChangePassword: true };
    S.authView = 'password';
  }
  cancelAdvance();
  S.navOpen = false;
  closeDialogs();
  render();
}

/** The library stamp the banks in memory were read at (GET /api/auth/state); null = not known yet */
let libraryStamp: string | null = null;

/** Loads everything for the signed-in user and shows the page of the current route. */
async function enterApp(): Promise<void> {
  S.authView = null;
  S.authNote = null;
  S.defaultAdmin = null;
  S.phase = 'loading';
  render();
  try {
    // The stamp first: a bank imported while the rest loads makes the next check read the banks once more, never less
    const st = await fetchAuthState();
    libraryStamp = st.library ?? null;
    await loadAll();
    await keepFirstVisitLang();
    dataOwner = S.user?.id ?? null;
    for (const fn of signedInHooks) fn();
    S.phase = 'ready';
    void migrateBrowserQuestionLang().then(render);
  } catch (e) {
    // A sign-in problem shows its own page (set by onAuthProblem); anything else is the fatal page
    if (S.authView) return;
    S.phase = 'fatal';
    S.fatal = e;
  }
  const next = settle(S.route);
  if (!sameRoute(next, S.route)) history.replaceState(null, '', routeHash(next));
  S.route = next;
  render();
}

let checking = false;
/** A sign-in, a new password or a sign-out of this page is running (a check must not act at the same time) */
let authBusy = false;

/**
 * Asks the server who is signed in (on every page change and when the tab comes back into view), so a session that ended
 * meanwhile (signed out elsewhere, account disabled, deleted or given a new password) shows the sign-in page at the next
 * click, not only at the next request that needs data. It also notices when the shared question banks changed (an
 * administrator imported or deleted one) and reads them again.
 */
function checkSession(): void {
  if (checking || authBusy) return;
  if (S.authView) {
    void recheckAuthPage();
    return;
  }
  if (!S.user || S.phase !== 'ready') return;
  checking = true;
  fetchAuthState().then((st) => {
    const me = S.user;
    if (!me || S.authView) return;
    if (!st.user) onAuthProblem('signin');
    else if (st.user.id !== me.id) window.location.reload();
    else if (st.user.mustChangePassword) onAuthProblem('password');
    else {
      // A new name, or (v0.6.5) the bank-manager permission given or taken away: it applies without signing out, so the
      // menu and the Question banks page follow at the next check
      if (st.user.username !== me.username || st.user.canManageBanks !== me.canManageBanks) {
        S.user = st.user;
        render();
      }
      if (st.library && st.library !== libraryStamp) {
        refreshLibrary().then(render, () => {
          /* the next check tries again */
        });
      }
    }
  }, () => {
    /* offline for a moment: the next request tells */
  }).finally(() => {
    checking = false;
  });
}

/**
 * A sign-in page that comes back into view: another tab of this browser (same cookie) may have signed in or chosen the new
 * password meanwhile, or the session behind the "choose a new password" page may have ended.
 */
async function recheckAuthPage(): Promise<void> {
  checking = true;
  try {
    const st = await fetchAuthState();
    if (!S.authView || authBusy) return;
    const u = st.user;
    if (!u) {
      S.defaultAdmin = st.defaultAdmin ?? null;
      if (S.authView === 'password') onAuthProblem('signin');
      else render();
      return;
    }
    // Still the same person on the same page: nothing to do
    if (S.authView === 'password' && S.user?.id === u.id && u.mustChangePassword) return;
    await afterSignIn(u);
  } catch {
    /* offline for a moment: the page stays as it is */
  } finally {
    checking = false;
  }
}

/** Another tab signed in as someone else: this page (its data and its cookie) now belongs to that user, so start afresh. */
function watchOtherTabs(): void {
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') checkSession();
  });
  window.addEventListener('storage', (e) => {
    if (e.key === paths.lastUser && S.user && e.newValue != null && e.newValue !== String(S.user.id)) window.location.reload();
  });
}

export async function boot(): Promise<void> {
  window.addEventListener('hashchange', onHashChange);
  setAuthHandler(onAuthProblem);
  watchOtherTabs();
  S.phase = 'loading';
  S.route = parseHash(location.hash);
  render();
  let user: AuthUser | null;
  let state: Awaited<ReturnType<typeof fetchAuthState>>;
  try {
    // /api/info is public too: the sign-in page shows the version
    const [st, info] = await Promise.all([fetchAuthState(), fetchInfo().catch(() => null)]);
    state = st;
    if (info) S.info = info;
    user = state.user;
  } catch (e) {
    S.phase = 'fatal';
    S.fatal = e;
    render();
    return;
  }
  if (!user) {
    S.authView = 'signin';
    S.defaultAdmin = state.defaultAdmin ?? null;
    render();
    return;
  }
  adoptUser(user);
  if (user.mustChangePassword) {
    S.authView = 'password';
    render();
    return;
  }
  await enterApp();
}

/** `user` is signed in now (this page, or another tab with the same cookie): show their password page or the app. */
async function afterSignIn(user: AuthUser): Promise<void> {
  if (freshStart || (dataOwner != null && dataOwner !== user.id)) {
    // Someone else's data and page state are still in memory: start from a fresh page (the address keeps the route). The new
    // user's cache paints it, in the language picked on the sign-in page.
    rememberUser(user.id);
    carryLanguage(S.settings.uiLang);
    window.location.reload();
    return;
  }
  adoptUser(user);
  if (user.mustChangePassword) {
    S.authView = 'password';
    S.authNote = null;
    render();
    return;
  }
  await enterApp();
}

/** The sign-in page: throws the ApiError of a failed sign-in (wrong name or password, too many attempts, ...). */
export async function signIn(username: string, password: string): Promise<void> {
  authBusy = true;
  try {
    await afterSignIn(await login(username, password));
  } finally {
    authBusy = false;
  }
}

/**
 * The "choose a new password" page: sets the new password (the session proves who it is), then opens the app. The toast
 * comes after the user's own settings are applied, so it is in the same language as the page under it.
 */
export async function saveNewPassword(password: string): Promise<void> {
  authBusy = true;
  let user: AuthUser;
  try {
    user = await changePassword(password);
  } catch (e) {
    // 400: maybe another tab of this browser chose the new password meanwhile (then the server asks for the current one)
    const st = e instanceof ApiError && e.status === 400 ? await fetchAuthState().catch(() => null) : null;
    const me = S.user;
    if (st?.user && me && st.user.id === me.id && !st.user.mustChangePassword) {
      await afterSignIn(st.user).finally(() => {
        authBusy = false;
      });
      toast(t('auth.password.elsewhere'), 6000);
      return;
    }
    authBusy = false;
    throw e;
  }
  try {
    adoptUser(user);
    await enterApp();
  } finally {
    authBusy = false;
  }
  if (!S.authView) toast(t('auth.password.saved'));
}

/** Settings > Account: changes the signed-in user's password; their other sessions are signed out. */
export async function changeOwnPassword(current: string, password: string): Promise<void> {
  S.user = await changePassword(password, current);
}

let signingOut = false;

/** Signs out: the session ends, everything of this user leaves memory, and the sign-in page opens on the home route. */
export async function signOut(): Promise<void> {
  if (signingOut) return;
  signingOut = true;
  authBusy = true;
  try {
    await logout();
  } catch (e) {
    // Already signed out (401: the session was gone) is fine; anything else keeps the user signed in
    if (!isSignInError(e)) {
      toast(t('auth.signOut.failed', { reason: errorText(e) }), 6000);
      return;
    }
  } finally {
    signingOut = false;
    authBusy = false;
  }
  if (S.route.name === 'quiz' && S.session) persistSession();
  cancelAdvance();
  closeDialogs();
  epoch += 1;
  freshStart = freshStart || dataOwner != null;
  clearUserData();
  setStorageOwner(null);
  S.authView = 'signin';
  S.authNote = 'signedOut';
  if (location.hash && location.hash !== '#/') history.replaceState(null, '', '#/');
  S.route = { name: 'home' };
  render();
  scrollTop();
  refreshSignInHint();
}

/** Re-reads the questions and the bank list (after an import, or when the shared banks changed elsewhere). */
export async function refreshLibrary(): Promise<void> {
  // The stamp first (see enterApp)
  const st = await fetchAuthState().catch(() => null);
  if (st?.library) libraryStamp = st.library;
  // The bank-manager permission may have changed with the banks (a refused bank change is often the first sign of it)
  if (st?.user && S.user && st.user.id === S.user.id && st.user.canManageBanks !== S.user.canManageBanks) S.user = st.user;
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

export function startSession(qs: Question[], mode: Mode, kind: AttemptKind = 'exam'): void {
  S.session = {
    qs, answers: {}, flags: new Set(), idx: 0, base: 0, t0: Date.now(), mode,
    sheets: [...new Set(qs.map((q) => q.sheet))],
    kind, checked: new Set(),
  };
  persistSession();
  navigate({ name: 'quiz' });
}

/**
 * Starts an exam or (v0.6.5) a practice; if an unfinished exam or practice is saved, asks before replacing it (the texts
 * name what is saved and what starts). Used by the home, result and records pages.
 */
export async function startExam(qs: Question[], mode: Mode, kind: AttemptKind = 'exam'): Promise<void> {
  if (!qs.length) return;
  const saved = loadSaved(S.bank);
  if (saved) {
    const savedPractice = saved.kind === 'practice';
    const ok = await ask({
      title: savedPractice ? t('quiz.overwrite.practiceTitle') : t('act.overwrite.title'),
      body: savedPractice ? t('quiz.overwrite.practiceBody') : kind === 'practice' ? t('quiz.overwrite.examBody') : t('act.overwrite.body'),
      ok: kind === 'practice' ? t('quiz.overwrite.practiceOk') : t('act.overwrite.ok'),
      cancel: t('act.overwrite.cancel'),
      danger: true,
    });
    if (!ok) return;
  }
  startSession(qs, mode, kind);
}

export function resumeSession(): void {
  const sv = loadSaved(S.bank);
  if (!sv) {
    render();
    return;
  }
  S.session = sessionOf(sv);
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
  // Practice: the feedback on the answer is the point, so the page never moves on by itself
  if (!se || se.kind === 'practice' || !S.settings.autoAdvance || se.idx >= se.qs.length - 1) return;
  const idx = se.idx;
  cancelAdvance();
  advanceTimer = window.setTimeout(() => {
    advanceTimer = undefined;
    if (S.session === se && se.idx === idx && S.route.name === 'quiz' && !document.querySelector('dialog[open]')) go(idx + 1);
  }, ADVANCE_DELAY_MS);
}

export function pick(q: Question, key: string): void {
  const se = S.session;
  if (!se || isLocked(se.kind, se.checked, q.id)) return;
  let a = (se.answers[q.id] as string[] | undefined) ?? [];
  const adding = !a.includes(key);
  if (q.kind === 'single') a = adding ? [key] : [];
  else a = adding ? [...a, key].sort() : a.filter((k) => k !== key);
  if (a.length) se.answers[q.id] = a;
  else delete se.answers[q.id];
  // Practice: a single-choice question is checked by the pick itself
  if (se.kind === 'practice' && adding && checksOnPick(q)) {
    checkAnswer(q);
    return;
  }
  syncQuiz();
  if (q.kind === 'single' && adding) scheduleAdvance();
  else cancelAdvance();
}

export function pickItem(q: Question, i: number, j: number): void {
  const se = S.session;
  if (!se || isLocked(se.kind, se.checked, q.id)) return;
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

/**
 * Practice: checks the answer of a question (something must be chosen). The question is locked from then on, and the page is
 * drawn again to show right / wrong, the correct answer and the explanation.
 */
export function checkAnswer(q: Question): void {
  const se = S.session;
  if (!se || se.kind !== 'practice' || se.checked.has(q.id) || !Array.isArray(se.answers[q.id])) return;
  se.checked.add(q.id);
  cancelAdvance();
  persistSession();
  // Keep the page where it is: the feedback opens under the options, where the eyes already are
  const y = window.scrollY;
  render();
  window.scrollTo(0, y);
}

export function toggleFlag(q: Question): void {
  const se = S.session;
  if (!se) return;
  if (se.flags.has(q.id)) se.flags.delete(q.id);
  else se.flags.add(q.id);
  syncQuiz();
}

let submitting = false;

/**
 * Submits the exam / finishes the practice: confirm first, then save the result; the saved session is cleared only after
 * saving succeeded (on failure the answers are still there, so it can be submitted again). An exam always asks; a practice
 * asks only when some questions were not checked (they count as not answered).
 */
export async function submit(): Promise<void> {
  const se = S.session;
  if (!se || submitting) return;
  cancelAdvance();
  const open = openCount(se.kind, se.qs, se.answers, se.checked);
  const go_ = se.kind === 'practice'
    ? !open || await ask({
      title: tn('quiz.practice.finish.openTitle', open), body: t('quiz.practice.finish.openBody'), ok: t('quiz.practice.finish'), cancel: t('act.submit.back'),
    })
    : await ask(open
      ? { title: tn('act.submit.openTitle', open), body: t('act.submit.openBody'), ok: t('act.submit.ok'), cancel: t('act.submit.back') }
      : { title: t('act.submit.confirmTitle'), body: t('act.submit.confirmBody'), ok: t('act.submit.ok'), cancel: t('act.submit.check') });
  if (!go_ || S.session !== se) return;

  submitting = true;
  try {
    // A practice counts its unchecked questions as not answered (see domain/practice.ts)
    const results = finalResults(se.kind, se.qs, se.answers, se.checked);
    const seconds = Math.floor(elapsed());
    let prev = null;
    let saved = false;
    // The results that count: all of them, unless the server left out questions that no longer exist (v0.6.5)
    let counted = results;
    let notCounted = 0;
    try {
      const { notCounted: gone, ...progress } = await submitAttempt({
        results: results.map((r) => ({ id: r.q.id, correct: r.correct, answer: Array.isArray(r.ans) ? r.ans : null })),
        sheets: se.sheets, mode: se.mode, seconds, kind: se.kind,
      });
      S.progress = progress;
      saved = true;
      clearSaved();
      ({ counted, left: notCounted } = countedResults(results, gone));
      const second = S.progress.history[1];
      // "Last time": the previous result of the same kind and mode
      if (second && second.mode === se.mode && kindOf(second) === se.kind) prev = second;
    } catch (e) {
      // Signed out meanwhile (the sign-in page is showing now): stay on the exam, so it can be submitted after signing in
      if (S.authView) return;
      if (e instanceof ApiError && e.status === 409) {
        // Every question of this exam was deleted while it was open (its bank is gone): it can never be saved, so drop it
        // instead of keeping it for another try, say why, and show the banks as they are now
        clearSaved();
        S.session = null;
        toast(errorText(e), 9000);
        navigate({ name: 'home' });
        refreshLibrary().then(render, () => {});
        return;
      }
      toast(`${t('act.submit.saveFailed')} (${errorText(e)})`, 9000);
    }
    S.session = null;
    S.result = { results: counted, seconds, mode: se.mode, prev, recordId: null, notCounted, kind: se.kind };
    S.rvFilter = 'wrong';
    navigate({ name: 'result' });
    // Some questions were deleted meanwhile: the banks in memory are out of date too
    if (notCounted) refreshLibrary().then(render, () => {});
    if (saved) {
      // The newest record of this user is this exam; remember its id so the result page can link to it
      const good = counted.filter((r) => r.correct).length;
      const mine = S.result;
      refreshRecords().then(() => {
        const top = S.records?.[0];
        if (mine && S.result === mine && top && top.total === counted.length && top.correct === good && kindOf(top) === mine.kind) mine.recordId = top.id;
        render();
      }, () => {});
    }
  } finally {
    submitting = false;
  }
}

export const resultPercent = (good: number, total: number): number => percentOf(good, total);
