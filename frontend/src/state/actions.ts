// Actions that change application state and trigger a re-render. Page files only call these and never import each other, which avoids circular dependencies.
//
// The renderer is registered via setRenderer() (done in main.ts), so nothing here depends on any page.
import { submitAttempt, resetProgress } from '../api/progress.ts';
import { fetchBank, fetchInfo } from '../api/bank.ts';
import { fetchProgress } from '../api/progress.ts';
import { cancelImport, fetchImports } from '../api/import.ts';
import { grade, percentOf, stateOf, type Answer } from '../domain/grading.ts';
import type { Mode, Question } from '../types/api.ts';
import { ask } from '../utils/dialog.ts';
import { toast } from '../utils/toast.ts';
import { ApiError } from '../api/client.ts';
import { clearSaved, loadSaved, saveSession as persist } from './session-storage.ts';
import { S, emptyAdmin, setBank, type Lang } from './store.ts';
import { paths } from './storage-keys.ts';

let renderer: () => void = () => {};
let refreshers: Array<() => void> = [];

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

/** First page load / refresh: fetch the version, question bank and study records. Any single failure goes to the fatal page. */
export async function loadAll(): Promise<void> {
  const [info, bank, progress] = await Promise.all([fetchInfo(), fetchBank(), fetchProgress()]);
  S.info = info;
  S.progress = progress;
  setBank(bank);
}

export async function boot(): Promise<void> {
  S.view = 'loading';
  render();
  try {
    await loadAll();
    S.view = 'home';
  } catch (e) {
    S.view = 'fatal';
    S.admin.error = e instanceof ApiError ? e.message : '连不上服务。请确认程序（或 Docker 容器）还在运行，然后刷新这个页面。';
  }
  render();
}

export function startSession(qs: Question[], mode: Mode): void {
  S.session = {
    qs, answers: {}, flags: new Set(), idx: 0, base: 0, t0: Date.now(), mode,
    sheets: [...new Set(qs.map((q) => q.sheet))],
  };
  S.view = 'quiz';
  persistSession();
  render();
  scrollTop();
}

export function resumeSession(): void {
  const sv = loadSaved(S.bank);
  if (!sv) {
    render();
    return;
  }
  S.session = { qs: sv.qs, answers: sv.answers, flags: sv.flags, idx: sv.idx, base: sv.elapsed, t0: Date.now(), mode: sv.mode, sheets: sv.sheets };
  S.view = 'quiz';
  if (sv.dropped) toast(`有 ${sv.dropped} 道题已经不在题库里，已跳过`);
  render();
  scrollTop();
}

export function leaveQuiz(): void {
  persistSession();
  S.session = null;
  S.view = 'home';
  render();
  scrollTop();
}

export function go(i: number): void {
  const se = S.session;
  if (!se || i < 0 || i >= se.qs.length) return;
  se.idx = i;
  persistSession();
  render();
  scrollTop();
}

export function setLang(code: Lang): void {
  if (S.lang === code) return;
  S.lang = code;
  try {
    localStorage.setItem(paths.lang, code);
  } catch {
    /* If it cannot be saved, it only applies for this session */
  }
  const y = window.scrollY;
  render(); // Only swaps the text; selected answers, flags and the timer all live in S.session and are unaffected
  window.scrollTo(0, y);
}

function syncQuiz(): void {
  runRefreshers();
  persistSession();
}

export function pick(q: Question, key: string): void {
  const se = S.session;
  if (!se) return;
  let a = (se.answers[q.id] as string[] | undefined) ?? [];
  if (q.kind === 'single') a = a.includes(key) ? [] : [key];
  else a = a.includes(key) ? a.filter((k) => k !== key) : [...a, key].sort();
  if (a.length) se.answers[q.id] = a;
  else delete se.answers[q.id];
  syncQuiz();
}

export function pickItem(q: Question, i: number, j: number): void {
  const se = S.session;
  if (!se) return;
  const cur = se.answers[q.id];
  const a: Array<number | null> = Array.isArray(cur) ? (cur.slice() as Array<number | null>) : Array<number | null>(q.items.length).fill(null);
  a[i] = a[i] === j ? null : j;
  if (a.every((x) => x == null)) delete se.answers[q.id];
  else se.answers[q.id] = a;
  syncQuiz();
}

export function toggleFlag(q: Question): void {
  const se = S.session;
  if (!se) return;
  if (se.flags.has(q.id)) se.flags.delete(q.id);
  else se.flags.add(q.id);
  syncQuiz();
}

let submitting = false;

/** Submit the exam: confirm first, then submit the score; the saved exam is cleared only after the score is submitted successfully (on failure the answers are still there, so it can be submitted again). */
export async function submit(): Promise<void> {
  const se = S.session;
  if (!se || submitting) return;
  const open = se.qs.filter((q) => stateOf(q, se.answers[q.id] as Answer) < 2).length;
  const go_ = await ask(open
    ? { title: `还有 ${open} 道题没答完`, body: '没答完的题按答错计分。现在交卷吗？', ok: '交卷', cancel: '回去继续' }
    : { title: '确认交卷？', body: '交卷后才会计分，并显示答案和解析。', ok: '交卷', cancel: '再检查一下' });
  if (!go_ || S.session !== se) return;

  submitting = true;
  try {
    const results = se.qs.map((q) => {
      const ans = (Array.isArray(se.answers[q.id]) ? se.answers[q.id] : null) as Answer;
      return { q, ans, correct: grade(q, ans) };
    });
    const seconds = Math.floor(elapsed());
    let prev = null;
    try {
      S.progress = await submitAttempt({
        results: results.map((r) => ({ id: r.q.id, correct: r.correct })), sheets: se.sheets, mode: se.mode, seconds,
      });
      clearSaved();
      const hist = S.progress.history;
      const second = hist[1];
      if (second && second.mode === se.mode) prev = second;
    } catch (e) {
      const why = e instanceof ApiError && e.kind !== 'http' ? '（服务没连上，或登录已过期）' : '';
      toast(`成绩没能保存到学习记录${why}。这一场的答案还留在浏览器里：刷新页面重新登录后，在首页「继续作答」里再交一次卷`, 7000);
    }
    S.session = null;
    S.result = { results, seconds, mode: se.mode, prev };
    S.rvFilter = 'wrong';
    S.view = 'result';
    render();
    scrollTop();
  } finally {
    submitting = false;
  }
}

export const resultPercent = (good: number, total: number): number => percentOf(good, total);

export async function openAdmin(): Promise<void> {
  S.admin = emptyAdmin();
  S.view = 'admin';
  render();
  scrollTop();
  try {
    S.admin.imports = await fetchImports();
  } catch {
    /* Fine if there is none */
  }
  if (S.view === 'admin') render();
}

export function closeAdmin(): void {
  if (S.admin.preview) cancelImport().catch(() => {});
  S.admin = emptyAdmin();
  S.view = 'home';
  render();
  scrollTop();
}

export async function refreshBank(): Promise<void> {
  setBank(await fetchBank());
}

export async function clearProgress(): Promise<void> {
  const ok = await ask({ title: '清空学习记录？', body: '成绩历史和错题本都会清掉，不能恢复。', ok: '清空', cancel: '不清空', danger: true });
  if (!ok) return;
  try {
    S.progress = await resetProgress();
    render();
    toast('学习记录已清空');
  } catch (e) {
    toast(e instanceof ApiError && e.kind === 'http' ? e.message : '没能清空：服务没连上，或登录已过期');
  }
}
