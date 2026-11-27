// Actions that change application state and trigger a re-render. Page files only call these and never import each other,
// which avoids circular dependencies. The renderer is registered via setRenderer() (done in main.ts).
//
// Sign-in (v0.6): boot asks GET /api/auth/state first. Nobody signed in -> the sign-in page; a default or initial password
// -> the "choose a new password" page; otherwise everything is loaded as before. Whenever a request answers 401 the sign-in
// page comes back (the address keeps the route, which opens again after signing in); a 403 "choose a new password first"
// brings that page. A different user signing in on a page that still holds someone's data reloads the page, so nothing of
// the previous user (data, page state, unfinished exam) survives in memory.
import { changePassword, fetchAuthState, loginSession, logout, setDisplayName } from '../api/auth.ts';
import { ApiError, isSignInError, setAuthHandler, type AuthProblem } from '../api/client.ts';
import { submitAttempt } from '../api/progress.ts';
import { fetchBank, fetchBanks, fetchInfo } from '../api/bank.ts';
import { fetchProgress, fetchRecord, fetchRecords } from '../api/progress.ts';
import { hardSoonMinutes } from '../domain/activity.ts';
import { countedResults, kindOf } from '../domain/attempt.ts';
import { aiServiceOf } from '../domain/ai-settings.ts';
import { setupRequired, setupView } from '../domain/user-name.ts';
import {
  attemptTail, defaultFilter, examAnswer, isEmptyAnswer, isTimeUp, markUnanswered, noticeDue, recordSeconds, remaining, resultCounts, srDue, submitConfirm,
  type EndKind, type TimedFields,
} from '../domain/exam-timer.ts';
import { completes, percentOf } from '../domain/grading.ts';
import { finalResults, isLocked, listNumbers, openCount, openNumbers } from '../domain/practice.ts';
import { t, tn } from '../i18n/index.ts';
import { errorText } from '../i18n/errors.ts';
import { parseHash, routeHash, sameRoute, type Route } from '../router.ts';
import type { AttemptKind, AttemptRequest, AttemptResponse, AuthUser, BankDeleted, Mode, Question, QuestionLang, RecordDetail, RecordsDeleted, UiLang } from '../types/api.ts';
import { releaseAnswerFx } from '../components/answer-fx.ts';
import { dropAiSummary } from '../components/ai-summary.ts';
import { ask, closeAllDialogs } from '../utils/dialog.ts';
import { toast } from '../utils/toast.ts';
import { refreshAiStatus } from './ai.ts';
import { loadSettings, migrateBrowserQuestionLang, showCachedSettings, updateSettings } from './preferences.ts';
import { adoptLegacySession, clearSaved, loadSaved, newSessionKey, saveSession as persist, type SavedSession } from './session-storage.ts';
import { DEFAULT_SETTINGS, cachedSettingsOf, carryLanguage, rememberUser } from './settings-cache.ts';
import { paths, setStorageOwner } from './storage-keys.ts';
import { nextUnsaved } from './unsaved.ts';
import { visitToReport } from './visits.ts';
import { reportVisit } from '../api/activity.ts';
import { S, clearUserData, setBankData, type ResultRow, type Session } from './store.ts';

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
    kind: se.kind, checked: se.checked, key: se.key, startedAt: se.startedAt, timed: timedOf(se), lang: se.lang,
  });
}

/** The time-limit fields of a session (TL-8 `TimedFields`) */
export const timedOf = (se: Pick<Session, 'limit' | 'warned' | 'timeUp'>): TimedFields => ({ limit: se.limit, warned: se.warned, timeUp: se.timeUp });

/** The session to continue from what was saved in this browser (the timer goes on from the saved time) */
function sessionOf(sv: SavedSession): Session {
  return {
    qs: sv.qs, answers: sv.answers, flags: sv.flags, idx: sv.idx, base: sv.elapsed, t0: Date.now(), mode: sv.mode, sheets: sv.sheets,
    kind: sv.kind, checked: sv.checked, key: sv.key, startedAt: sv.startedAt,
    limit: sv.timed.limit, warned: sv.timed.warned, timeUp: sv.timed.timeUp,
    // A save from before v2.6 has no language: it starts from the default
    lang: sv.lang ?? S.settings.questionLang,
  };
}

// ---------------------------------------------------------------- routing

/** Brings S.route in line with the state: no exam -> no quiz page, no result -> no result page. */
function settle(r: Route): Route {
  if (r.name === 'quiz' && !S.session) {
    // A reload on the quiz page: continue the saved exam if there is one
    const sv = S.phase === 'ready' ? loadSaved(S.bank) : null;
    if (sv) {
      S.session = sessionOf(sv);
      afterOpen();
      return r;
    }
    return { name: 'study' };
  }
  if (r.name === 'result' && !S.result) return { name: 'records' };
  return r;
}

/** v1.3: whether the server wants page visits of this user (`logVisits` of the auth state); set after sign-in and at every check */
let logVisits = false;

/** v1.3: the end of an administrator's sign-in (a hard cut, no extension); one toast when it is under 10 minutes away */
let hardEndsAt: string | null = null;
let hardWarned = false;
let hardTimer: number | undefined;

function warnHardEnd(): void {
  if (hardWarned || !S.user || S.authView) return;
  const n = hardSoonMinutes(hardEndsAt, Date.now());
  if (n == null) return;
  hardWarned = true;
  toast(t('auth.hardSoon', { n }), 8000);
}

/** Remembers the session rule of the last answer (login or auth state); a new end time allows a new warning */
function noteSession(info: { hardEndsAt: string | null } | null | undefined): void {
  const end = info?.hardEndsAt ?? null;
  if (end !== hardEndsAt) hardWarned = false;
  hardEndsAt = end;
  window.clearInterval(hardTimer);
  if (end) {
    hardTimer = window.setInterval(warnHardEnd, 60_000);
    warnHardEnd();
  }
}

/** Reports a page visit when the server asked for it (a route-name change); never disturbs the page */
function noteVisit(from: string | null, to: string): void {
  if (S.authView || !S.user) return;
  const page = visitToReport(logVisits, from, to);
  if (page) void reportVisit(page);
}

function switchTo(next: Route): void {
  const from = S.route;
  if (from.name === 'quiz' && next.name !== 'quiz') releaseAnswerFx();
  if (from.name === 'quiz' && next.name !== 'quiz' && S.session) {
    // Leaving the quiz in any way (back button, link): keep the answers
    persistSession();
    S.session = null;
  }
  cancelAdvance();
  S.route = next;
  S.navOpen = false;
  for (const fn of routeHooks) fn(from, next);
  noteVisit(from.name, next.name);
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

/**
 * v0.8: reads what the signed-in user may do with AI (state/ai.ts) and draws the page again when that changed (the menu
 * entry, the badge, AI Chat). `force`: now (after sign-in, or after an administrator changed AI settings or someone's AI+);
 * otherwise at most every 60 s. Never fails: an AI service that cannot be reached only hides AI Chat.
 */
export function refreshAi(force = false): Promise<void> {
  return refreshAiStatus({ force, mustChangePassword: S.user ? setupRequired(S.user) : true }).then((changed) => {
    if (changed && S.user && !S.authView) render();
  });
}

/**
 * v1.1 (BT-2): keeps `S.aiService` in line with an auth-state answer (administrators get `aiService`, everyone else none).
 * Returns whether it changed, so a caller that can re-render does.
 */
function noteAiService(st: unknown): boolean {
  const next = aiServiceOf(st);
  const changed = JSON.stringify(next) !== JSON.stringify(S.aiService);
  S.aiService = next;
  return changed;
}

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
  noteSession(null);
  if (problem === 'signin' || problem === 'expired') {
    if (S.authView === 'signin') return;
    // Keep the answers of an exam in progress: after signing in again it can be continued
    if (S.route.name === 'quiz' && S.session) persistSession();
    // v1.3: a 401 with code auth.expired says so even before the user was known (a reload with an ended session)
    S.authNote = S.user || problem === 'expired' ? 'expired' : null;
    S.authView = 'signin';
    epoch += 1;
    refreshSignInHint();
  } else {
    if (S.authView) return;
    // v2.6: 'name' = only the display name is missing (the password is fine)
    if (S.user) S.user = problem === 'name' ? { ...S.user, needsName: true } : { ...S.user, mustChangePassword: true };
    S.authView = problem === 'name' ? 'name' : 'password';
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
  // v0.8: what the user may do with AI, read alongside the rest (it never fails the loading; AI Chat waits for it)
  void refreshAi(true);
  try {
    // The stamp first: a bank imported while the rest loads makes the next check read the banks once more, never less
    const st = await fetchAuthState();
    libraryStamp = st.library ?? null;
    logVisits = st.logVisits === true;
    noteSession(st.session);
    noteAiService(st);
    await loadAll();
    await keepFirstVisitLang();
    dataOwner = S.user?.id ?? null;
    for (const fn of signedInHooks) fn();
    S.phase = 'ready';
    noteVisit(null, S.route.name);
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
  // TL-5 step 7: a timed exam whose time ran out while signed out (or before a reload) is submitted now
  afterOpen();
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
    logVisits = st.logVisits === true;
    noteSession(st.session);
    if (!st.user) onAuthProblem('signin');
    else if (st.user.id !== me.id) window.location.reload();
    else if (st.user.mustChangePassword) onAuthProblem('password');
    else if (st.user.needsName) onAuthProblem('name');
    else {
      // A new name, or (v0.6.5) the bank-manager permission given or taken away: it applies without signing out, so the
      // menu and the Question banks page follow at the next check. v0.8: the same for AI+ (the AI status is read again
      // at once then; otherwise at most every 60 s)
      const aiChanged = st.user.aiAllowed !== me.aiAllowed;
      const svcChanged = noteAiService(st);
      if (st.user.username !== me.username || (st.user.displayName ?? null) !== (me.displayName ?? null) || st.user.canManageBanks !== me.canManageBanks || (st.user.canDownloadBanks === true) !== (me.canDownloadBanks === true) || aiChanged || svcChanged) {
        S.user = st.user;
        render();
      }
      void refreshAi(aiChanged);
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
      if (S.authView === 'password' || S.authView === 'name') onAuthProblem('signin');
      else render();
      return;
    }
    // Still the same person on the same page: nothing to do
    if (S.authView === 'password' && S.user?.id === u.id && u.mustChangePassword) return;
    if (S.authView === 'name' && S.user?.id === u.id && !u.mustChangePassword && u.needsName) return;
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
  installSessionGuards();
  watchOtherTabs();
  S.phase = 'loading';
  S.route = parseHash(location.hash);
  render();
  await start();
}

/** The start-up proper (also the fatal page's retry): who is signed in, then the app, the sign-in page or the new-password page */
async function start(): Promise<void> {
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
  if (setupRequired(user)) {
    S.authView = setupView(user);
    render();
    return;
  }
  await enterApp();
}

let retryingStart = false;

/** Whether the fatal page's retry is running (its button says 正在重新连接… and waits) */
export const isRetryingStart = (): boolean => retryingStart;

/**
 * v1.3 (A6): the fatal page's 重试 button: runs the start-up again without a full reload (the script is loaded). A signed-in
 * user loads everything again (the banks come back as a 304 when nothing changed); otherwise the whole start-up runs. A
 * second press while it runs does nothing.
 */
export async function retryStartup(): Promise<void> {
  if (retryingStart) return;
  retryingStart = true;
  render();
  try {
    if (S.user) await enterApp();
    else {
      S.phase = 'loading';
      await start();
    }
  } finally {
    retryingStart = false;
    if (S.phase === 'fatal') render();
  }
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
  if (setupRequired(user)) {
    S.authView = setupView(user);
    S.authNote = null;
    render();
    return;
  }
  await enterApp();
}

/** The sign-in page: throws the ApiError of a failed sign-in (wrong name or password, too many attempts, ...). */
export async function signIn(username: string, password: string, remember = true): Promise<void> {
  authBusy = true;
  try {
    const a = await loginSession(username, password, remember);
    noteSession(a.session);
    await afterSignIn(a.user);
  } finally {
    authBusy = false;
  }
}

/**
 * The "choose a new password" page: sets the new password (the session proves who it is), then opens the app. The toast
 * comes after the user's own settings are applied, so it is in the same language as the page under it.
 */
export async function saveNewPassword(password: string, displayName?: string): Promise<void> {
  authBusy = true;
  let user: AuthUser;
  try {
    user = await changePassword(password, undefined, displayName);
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
  if (!S.authView) toast(t(displayName !== undefined ? 'auth.setup.saved' : 'auth.password.saved'));
}

/**
 * v2.6: the "set a username" page (an existing user without a display name; the password is fine): saves the name, then opens
 * the app. Throws the ApiError (a taken or refused name) for the page to show.
 */
export async function saveFirstName(displayName: string): Promise<void> {
  authBusy = true;
  try {
    const user = await setDisplayName(displayName);
    adoptUser(user);
    await enterApp();
  } finally {
    authBusy = false;
  }
  if (!S.authView) toast(t('auth.name.saved'));
}

/** v2.6: Profile > Username: changes the signed-in user's display name (throws the ApiError for the page to show) */
export async function changeOwnName(displayName: string): Promise<void> {
  S.user = await setDisplayName(displayName);
}

/** Profile > Change password: changes the signed-in user's password; their other sessions are signed out. */
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
  noteSession(null);
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
  if (st?.user) noteAiService(st);
  // The bank-manager permission may have changed with the banks (a refused bank change is often the first sign of it)
  if (st?.user && S.user && st.user.id === S.user.id && (st.user.canManageBanks !== S.user.canManageBanks || (st.user.canDownloadBanks === true) !== (S.user.canDownloadBanks === true))) S.user = st.user;
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
  // v0.9: the saved AI summaries of the records that are gone are forgotten (an open window stays open)
  const left = new Set(d.records.map((r) => r.id));
  for (const r of S.records ?? []) if (!left.has(r.id)) dropAiSummary(r.id, r.at);
  S.records = d.records;
  S.recordsRetention = d.retention;
  S.progress = d.progress;
  const keep = new Set(d.records.map((r) => r.id));
  for (const id of [...S.recordCache.keys()]) if (!keep.has(id)) S.recordCache.delete(id);
}

// ---------------------------------------------------------------- exams

/** Options of a new session: `limit` = the time limit of an exam in seconds (TL-2; ignored for a practice) */
export interface StartOptions {
  limit?: number | null;
}

export function startSession(qs: Question[], mode: Mode, kind: AttemptKind = 'exam', opts: StartOptions = {}): void {
  S.session = {
    qs, answers: {}, flags: new Set(), idx: 0, base: 0, t0: Date.now(), mode,
    sheets: [...new Set(qs.map((q) => q.sheet))],
    kind, checked: new Set(), key: newSessionKey(), startedAt: Math.floor(Date.now() / 1000),
    limit: kind === 'exam' && opts.limit != null ? opts.limit : null, warned: false, timeUp: false,
    // Every new session starts from the default question language (Settings); the quiz switch changes only the session
    lang: S.settings.questionLang,
  };
  persistSession();
  navigate({ name: 'quiz' });
}

/**
 * Starts an exam or (v0.6.5) a practice; if an unfinished exam or practice is saved, asks before replacing it (the texts
 * name what is saved and what starts). Used by the home, result and records pages.
 */
export async function startExam(qs: Question[], mode: Mode, kind: AttemptKind = 'exam', opts: StartOptions = {}): Promise<void> {
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
  startSession(qs, mode, kind, opts);
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
  afterOpen();
}

/** Home's resume card of a session whose time ran out (TL-6): resumes it and submits at once (answering stays impossible) */
export function resumeAndSubmit(): void {
  resumeSession();
}

/** "Save and exit" on the quiz page */
export function leaveQuiz(): void {
  navigate({ name: 'study' });
}

export function go(i: number): void {
  const se = S.session;
  if (!se || i < 0 || i >= se.qs.length || timeIsUp(se)) return;
  cancelAdvance();
  se.idx = i;
  persistSession();
  render();
  scrollTop();
}

/** The language the questions of this session are shown in: it changes only the session (saved with it), never the settings. */
export function setSessionLang(code: QuestionLang): void {
  const se = S.session;
  if (!se || se.lang === code) return;
  const y = window.scrollY;
  se.lang = code;
  persistSession();
  render(); // only swaps the text; answers, flags and the timer live in S.session and are unaffected
  window.scrollTo(0, y);
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
  // v2.7: no auto-advance while the phone answer sheet (drawer) is open: the page under it must not be rebuilt
  if (document.documentElement.classList.contains('qz-sheet-open')) return;
  const idx = se.idx;
  cancelAdvance();
  advanceTimer = window.setTimeout(() => {
    advanceTimer = undefined;
    if (S.session === se && se.idx === idx && S.route.name === 'quiz' && !document.querySelector('dialog[open]') && !document.documentElement.classList.contains('qz-sheet-open')) go(idx + 1);
  }, ADVANCE_DELAY_MS);
}

export function pick(q: Question, key: string): void {
  const se = S.session;
  if (!se || timeIsUp(se) || isLocked(se.kind, se.checked, q.id)) return;
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
  if (!se || timeIsUp(se) || isLocked(se.kind, se.checked, q.id)) return;
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
  if (!se || timeIsUp(se) || se.kind !== 'practice' || se.checked.has(q.id) || !Array.isArray(se.answers[q.id])) return;
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
  if (!se || timeIsUp(se)) return;
  if (se.flags.has(q.id)) se.flags.delete(q.id);
  else se.flags.add(q.id);
  syncQuiz();
}

// ---------------------------------------------------------------- the time limit of an exam (v1.1, TL-3 ... TL-6)

/**
 * TL-5: whether the time of a timed exam is up, read from the clock every time (a background tab ticks a second or a minute
 * late), never only from the flag the tick sets.
 */
export function checkTimeUp(se: Session): boolean {
  return se.limit != null && (se.timeUp || isTimeUp(se.limit, elapsed()));
}

/** Input guard: when the time is up, the input is ignored and the time-up steps start (true) */
function timeIsUp(se: Session): boolean {
  if (!checkTimeUp(se)) return false;
  startTimeUp();
  return true;
}

let timeUpRunning = false;
let prevLeft: { se: Session; left: number } | null = null;

/**
 * TL-5 steps 1 - 4: marks the session, saves it at once, closes every dialog, locks the page (the overlay is drawn by the quiz
 * page from `se.timeUp`) and submits without a confirm. One run at a time; nothing while the sign-in page is showing (the
 * run starts again after signing in, `afterOpen`).
 */
export function startTimeUp(): void {
  const se = S.session;
  if (!se || se.limit == null || timeUpRunning || submitting || S.authView) return;
  timeUpRunning = true;
  se.timeUp = true;
  persistSession();
  closeAllDialogs();
  cancelAdvance();
  render();
  void submit({ end: 'timeUp' }).finally(() => {
    timeUpRunning = false;
  });
}

/**
 * The 1-second tick of a timed exam (the quiz page's interval): the time-up check, the 30-minute notice (TL-4) and the
 * screen-reader lines at 5 minutes and 1 minute. Returns the seconds left (null for an untimed session).
 */
export function examTick(): number | null {
  const se = S.session;
  if (!se || se.limit == null || S.route.name !== 'quiz') return null;
  if (checkTimeUp(se)) {
    startTimeUp();
    return 0;
  }
  const left = remaining(se.limit, elapsed());
  if (noticeDue(se.limit, left, se.warned)) {
    se.warned = true;
    persistSession();
    const text = t('quiz.notice30');
    toast(text, 6000);
    const live = document.getElementById('quiz-sr-alert');
    if (live) live.textContent = text;
  }
  const before = prevLeft && prevLeft.se === se ? prevLeft.left : left;
  const due = srDue(se.limit, before, left);
  if (due) {
    const live = document.getElementById('quiz-sr');
    if (live) live.textContent = t(due === 'm5' ? 'quiz.sr5min' : 'quiz.sr1min');
  }
  prevLeft = { se, left };
  return left;
}

/** After a session is opened or resumed (a reload, a resume, the sign-in page leaving): a timed exam whose time is up ends now (TL-5) */
function afterOpen(): void {
  queueMicrotask(() => {
    // No route check: a resume navigates by hash change, which lands after this microtask
    const se = S.session;
    if (se && checkTimeUp(se)) startTimeUp();
  });
}

/**
 * TL-6: the countdown pauses at the moment the page goes away. On `pagehide` and when the page becomes hidden, an exam or
 * practice on the quiz page is saved; when the page is visible (or restored from the cache) again the time limit is checked
 * before anything else. Called once from `boot()`.
 */
export function installSessionGuards(): void {
  const save = (): void => {
    if (S.route.name === 'quiz' && S.session) persistSession();
  };
  const check = (): void => {
    const se = S.session;
    if (se && S.route.name === 'quiz' && checkTimeUp(se)) startTimeUp();
  };
  window.addEventListener('pagehide', save);
  window.addEventListener('pageshow', check);
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'hidden') save();
    else check();
  });
}

let submitting = false;

export interface SubmitOptions {
  /** 'timeUp': the time ran out - no confirm, the result page shows the time-up banner (TL-5) */
  end?: EndKind;
}

/**
 * Submits the exam / finishes the practice: confirm first, then save the result; the saved session is cleared only after
 * saving succeeded (on failure the answers are still there, so it can be submitted again). An exam always asks (TL-5a: the
 * wording says what the not-completed questions mean); a practice asks only when some questions were not checked (they
 * count as not answered). At time-up (`end: 'timeUp'`) nothing is asked.
 */
export async function submit(opts: SubmitOptions = {}): Promise<void> {
  const se = S.session;
  if (!se || submitting) return;
  // The time ran out while the button was pressed: it becomes the time-up submit, without a confirm
  if (opts.end !== 'timeUp' && checkTimeUp(se)) {
    startTimeUp();
    return;
  }
  const end: EndKind = opts.end === 'timeUp' ? 'timeUp' : 'submit';
  const exam = se.kind === 'exam';
  cancelAdvance();
  if (end === 'submit') {
    const open = openCount(se.kind, se.qs, se.answers, se.checked);
    // QZ-9: the dialog names the open and the marked questions (up to 10 numbers each)
    const numsText = (nums: number[]): string => {
      const { shown, more } = listNumbers(nums);
      const list = shown.join(t('quiz.dlg.sep'));
      return more > 0 ? t('quiz.dlg.numsMore', { list, n: more }) : t('quiz.dlg.nums', { list });
    };
    const openNums = openNumbers(se.kind, se.qs, se.answers, se.checked);
    const flagNums = se.qs.flatMap((q, i) => (se.flags.has(q.id) ? [i + 1] : []));
    const lists = [openNums.length ? t('quiz.dlg.open', { nums: numsText(openNums) }) : '', flagNums.length ? t('quiz.dlg.flag', { nums: numsText(flagNums) }) : '']
      .filter(Boolean).join(' ');
    const withLists = (body: string): string => (lists ? `${body} ${lists}` : body);
    let go_: boolean;
    if (!exam) {
      go_ = !open || await ask({
        title: tn('quiz.practice.finish.openTitle', open), body: withLists(t('quiz.practice.finish.openBody')), ok: t('quiz.practice.finish'), cancel: t('act.submit.back'),
      });
    } else {
      // TL-5a, every exam: `empty` = questions with no answer (not completed); `open` also counts partly answered hotspots
      const empty = se.qs.filter((q) => isEmptyAnswer(se.answers[q.id])).length;
      const c = submitConfirm(open, empty);
      const dlg = c.kind === 'empty'
        ? {
          title: tn('act.submit.emptyTitle', empty),
          body: withLists(c.partial > 0 ? `${t('act.submit.emptyBody')} ${tn('act.submit.partialMore', c.partial)}` : t('act.submit.emptyBody')),
          ok: t('act.submit.ok'), cancel: t('act.submit.back'),
        }
        : c.kind === 'partial'
          ? { title: tn('act.submit.partialTitle', open), body: withLists(t('act.submit.partialBody')), ok: t('act.submit.ok'), cancel: t('act.submit.back') }
          : { title: t('act.submit.confirmTitle'), body: t('act.submit.confirmBody'), ok: t('act.submit.ok'), cancel: t('act.submit.check') };
      go_ = await ask(dlg);
    }
    if (!go_ || S.session !== se) {
      // 'Keep answering' jumps to the first open question (else the first marked one); not when the time ran out meanwhile
      const target = openNums[0] ?? flagNums[0];
      if (!go_ && S.session === se && !se.timeUp && target != null) go(target - 1);
      return;
    }
    // The time may have run out while the confirm was open: the time-up submit takes over (nothing more is asked)
    if (checkTimeUp(se)) {
      startTimeUp();
      return;
    }
  }

  submitting = true;
  try {
    // A practice counts its unchecked questions as not answered (see domain/practice.ts); an exam marks its not-completed
    // questions (TL-7) and sends no answer for them
    const graded = finalResults(se.kind, se.qs, se.answers, se.checked);
    const results: ResultRow[] = exam ? markUnanswered(graded, 'exam') : graded;
    const timed = timedOf(se);
    const seconds = exam ? recordSeconds(se.limit, elapsed()) : Math.floor(elapsed());
    let prev = null;
    let saved = false;
    // The results that count: all of them, unless the server left out questions that no longer exist (v0.6.5)
    let counted = results;
    let notCounted = 0;
    let recordId: number | null = null;
    let unsavedBody: AttemptRequest | null = null;
    try {
      const body: AttemptRequest = {
        results: results.map((r) => {
          const a = exam ? examAnswer(r.ans, r.correct) : r.ans;
          return { id: r.q.id, correct: r.correct, answer: Array.isArray(a) ? a : null };
        }),
        sheets: se.sheets, mode: se.mode, seconds, kind: se.kind, key: se.key, startedAt: se.startedAt, ...attemptTail(se.kind, timed, end),
      };
      unsavedBody = body;
      // the client retries a lost connection or a 502 / 503 / 504 by itself (api/client.ts retryPlan): the save is idempotent
      const { notCounted: gone, recordId: savedId, ...progress } = await submitAttempt(body);
      unsavedBody = null;
      recordId = savedId ?? null;
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
        navigate({ name: 'study' });
        refreshLibrary().then(render, () => {});
        return;
      }
      // v1.3 (A1): no toast; the result page carries a banner with a retry button (the answers stay on this device)
    }
    S.session = null;
    S.result = {
      results: counted, seconds, mode: se.mode, prev, recordId, notCounted, kind: se.kind, key: se.key, startedAt: se.startedAt,
      limit: exam ? se.limit : null, ended: exam ? end : null,
      ...(unsavedBody ? { unsaved: { body: unsavedBody, state: 'unsaved' as const } } : {}),
    };
    S.rvFilter = defaultFilter(resultCounts(counted));
    navigate({ name: 'result' });
    // Some questions were deleted meanwhile: the banks in memory are out of date too
    if (notCounted) refreshLibrary().then(render, () => {});
    // The save answered with the record's id (v0.9); the list of records is refreshed for the pages that show it
    if (saved) refreshRecords().then(render, () => {});
  } finally {
    submitting = false;
  }
}

/**
 * v1.3 (A1): the banner's retry. Sends the kept request again (the server answers a repeat with the first answer plus
 * `duplicate`), then fills the result page like a normal save. A failure leaves the banner for another try; a 409 (the
 * questions are gone) drops the kept result like a normal save does.
 */
export async function retryUnsavedResult(): Promise<void> {
  const R = S.result;
  const u = R?.unsaved;
  if (!R || !u || u.state !== 'unsaved') return;
  u.state = nextUnsaved(u.state, 'retry');
  render();
  try {
    const answer: AttemptResponse = await submitAttempt(u.body);
    if (S.result !== R) return;
    const { notCounted: gone, recordId, ...progress } = answer;
    S.progress = progress;
    clearSaved();
    const { counted, left } = countedResults(R.results, gone);
    R.results = counted;
    R.notCounted = left;
    R.recordId = recordId ?? null;
    const second = S.progress.history[1];
    if (second && second.mode === R.mode && kindOf(second) === R.kind) R.prev = second;
    u.state = nextUnsaved(u.state, 'ok');
    S.rvFilter = defaultFilter(resultCounts(counted));
    render();
    setTimeout(() => {
      if (S.result === R && R.unsaved === u) {
        delete R.unsaved;
        render();
      }
    }, 3500);
    if (left) refreshLibrary().then(render, () => {});
    refreshRecords().then(render, () => {});
  } catch (e) {
    if (S.authView || S.result !== R) return;
    if (e instanceof ApiError && e.status === 409) {
      clearSaved();
      delete R.unsaved;
      toast(errorText(e), 9000);
      refreshLibrary().then(render, () => {});
    } else {
      u.state = nextUnsaved(u.state, 'fail');
      toast(errorText(e), 6000);
    }
    render();
  }
}

export const resultPercent = (good: number, total: number): number => percentOf(good, total);
