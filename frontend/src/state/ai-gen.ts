// AI bank generation and file import: the module state of the progress cards, the client-only upload, the polling of a
// running job and the moments that follow a job's end (v1.1, GB-7, IM-5, IM-7, P-18). Nothing is stored in the browser.
// The pure rules (what a card shows, how fast to poll) live in domain/ai-gen.ts and domain/ai-import.ts.
import { cancelGen, dismissGen, fetchGenJob, jobOfError, startGen, type StartBody } from '../api/ai-gen.ts';
import { confirmImport as confirmImportRequest, uploadImport } from '../api/ai-import.ts';
import { activeJob, finishedBetween, pollInterval } from '../domain/ai-gen.ts';
import { awaitsConfirm, type UploadState } from '../domain/ai-import.ts';
import { errorText } from '../i18n/errors.ts';
import { t, uiLang } from '../i18n/index.ts';
import type { GenJob, ImportFileType } from '../types/api.ts';
import { toast } from '../utils/toast.ts';
import { onRouteChange, refreshAi, refreshLibrary, render } from './actions.ts';
import { aiGenStatus, aiStatus, onAiStatusChange, refreshAiStatus, setAiGenJob } from './ai.ts';
import { onClearUserData, S } from './store.ts';

// ---------------------------------------------------------------- subscriptions of the cards and buttons

const subs = new Set<() => void>();

/**
 * `fn` runs whenever the job state or the upload changes, for as long as `el` is in the page; the first notification that
 * finds `el` detached drops the subscription (pages redraw their elements, so nothing leaks).
 */
export function subscribeGen(el: Element, fn: () => void): void {
  const run = (): void => {
    if (!el.isConnected) {
      subs.delete(run);
      return;
    }
    fn();
  };
  subs.add(run);
}

function notify(): void {
  for (const fn of [...subs]) fn();
}

// ---------------------------------------------------------------- the client-only upload (IM-5)

export interface UploadView extends UploadState {
  abort: () => void;
}

let upload: UploadView | null = null;

export const currentUpload = (): UploadView | null => upload;

/** Starts the upload of a checked file; resolves with `null` on success (the job is in the status), 'aborted', or the refusal's text */
export async function startUpload(file: File, type: ImportFileType): Promise<null | 'aborted' | string> {
  if (upload) return null;
  const ctrl = new AbortController();
  const view: UploadView = { name: file.name, size: file.size, type, loaded: 0, total: file.size, abort: () => ctrl.abort() };
  upload = view;
  notify();
  try {
    const a = await uploadImport(file, uiLang(), (loaded, total) => {
      view.loaded = loaded;
      view.total = total;
      notify();
    }, ctrl.signal);
    upload = null;
    setAiGenJob(a.job, a.gen);
    toast(t('aigen.imp.toast.uploaded'), 3500);
    void refreshAi(true);
    scrollToCard();
    return null;
  } catch (e) {
    upload = null;
    notify();
    if (e instanceof Error && e.name === 'AbortError') return 'aborted';
    syncJobFromError(e);
    return errorText(e);
  }
}

// ---------------------------------------------------------------- actions

/** A 409 carries the job that is in the way (SEC-7: it may be null): keep the status in line with it */
function syncJobFromError(e: unknown): void {
  const j = jobOfError(e);
  if (j) setAiGenJob(j);
}

/** Starts a generation (API-U2); resolves with an error text, or null when it started */
export async function startGeneration(body: StartBody): Promise<string | null> {
  try {
    const a = await startGen(body);
    setAiGenJob(a.job, a.gen);
    toast(t('aigen.toast.started'), 3500);
    void refreshAi(true);
    scrollToCard();
    return null;
  } catch (e) {
    syncJobFromError(e);
    return errorText(e);
  }
}

/** Cancels (or declines) a job; the toast says which. Resolves with an error text, or null */
export async function cancelJob(job: GenJob, decline = false, keep = false): Promise<string | null> {
  try {
    const j = await cancelGen(job.id, keep);
    setAiGenJob(j);
    toast(decline ? t('aigen.imp.toast.declined') : keep && j.phase === 'saving' ? t('aigen.toast.keepSaving') : t(job.source === 'file' ? 'aigen.imp.toast.cancelled' : 'aigen.toast.cancelled'), 3000);
    void refreshAi(true);
    return null;
  } catch (e) {
    syncJobFromError(e);
    return errorText(e);
  }
}

/** Confirms an import waiting in phase `confirm` (API-U7) */
export async function confirmJob(job: GenJob, name: string): Promise<string | null> {
  try {
    const a = await confirmImportRequest(job.id, name);
    setAiGenJob(a.job, a.gen);
    toast(t('aigen.toast.started'), 3500);
    return null;
  } catch (e) {
    syncJobFromError(e);
    return errorText(e);
  }
}

/** Dismisses a finished job's card */
export async function dismissJob(job: GenJob): Promise<string | null> {
  try {
    await dismissGen(job.id);
    const g = aiGenStatus();
    if (g) {
      // drop it from the kept status at once; the next read agrees
      setAiGenJob(null, { ...g, unseen: g.unseen.filter((j) => j.id !== job.id) });
    }
    void refreshAi(true);
    return null;
  } catch (e) {
    return errorText(e);
  }
}

/** Scrolls the progress card into view and focuses it (the busy / "AI 导入中…" links) */
export function scrollToCard(): void {
  const go = (): void => {
    const el = document.querySelector<HTMLElement>('[data-gen-card]');
    if (!el) return;
    el.scrollIntoView({ block: 'start', behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'auto' : 'smooth' });
    el.focus({ preventScroll: true });
  };
  // after the page has drawn the card
  setTimeout(go, 50);
}

// ---------------------------------------------------------------- polling and the moments after a job

const confirmDialogShown = new Set<number>();
const recognisedToastShown = new Set<number>();
const confirmHooks: Array<(job: GenJob) => void> = [];
let timer: ReturnType<typeof setTimeout> | null = null;
let lastActive: GenJob | null = null;
let libraryPending = false;

/** The confirm dialog (components/ai-import.ts) registers here; it runs once per job per tab, on Home or Bank management only */
export function onConfirmDue(fn: (job: GenJob) => void): void {
  confirmHooks.push(fn);
}

const route = (): string => S.route.name;

/** Reloads the banks (the AI status is refreshed by the poll), then redraws the page (Home and Bank management show the new bank without a reload) */
function redrawLibrary(): void {
  refreshLibrary().then(render, () => {});
}

function finish(job: GenJob): void {
  const file = job.file?.name ?? '';
  const isFile = job.source === 'file';
  if (job.state === 'done') {
    libraryPending = true;
    if (route() !== 'quiz') {
      toast(isFile ? t('aigen.imp.toast.done', { file }) : t('aigen.toast.done', { name: job.name }), 3500);
      libraryPending = false;
      redrawLibrary();
    }
  } else if (route() !== 'quiz') {
    toast(isFile ? t('aigen.imp.toast.failed', { file }) : t('aigen.toast.failed', { name: job.name }), 3500);
  }
}

function checkConfirm(): void {
  const job = aiGenStatus()?.job ?? null;
  if (!awaitsConfirm(job)) return;
  const r = route();
  if ((r === 'home' || r === 'banks') && !confirmDialogShown.has(job.id)) {
    confirmDialogShown.add(job.id);
    for (const fn of confirmHooks) fn(job);
  } else if (r !== 'quiz' && r !== 'home' && r !== 'banks' && !recognisedToastShown.has(job.id)) {
    recognisedToastShown.add(job.id);
    toast(t('aigen.imp.toast.recognised', { file: job.file?.name ?? '' }), 5000);
  }
}

function schedule(): void {
  if (timer != null) {
    clearTimeout(timer);
    timer = null;
  }
  const ms = pollInterval(route(), !document.hidden, activeJob(aiGenStatus()));
  if (ms != null) timer = setTimeout(() => void tick(), ms);
}

async function tick(): Promise<void> {
  timer = null;
  const job = activeJob(aiGenStatus());
  if (!job) return;
  try {
    const j = await fetchGenJob(job.id);
    const finished = j.state === 'done' || j.state === 'failed';
    setAiGenJob(j);
    // the exact list of unseen jobs and the quota come from the status
    if (finished || j.state === 'cancelled') await refreshAiStatus({ force: true, mustChangePassword: S.user?.mustChangePassword ?? true });
  } catch {
    // a failed poll is tried again at the next interval
  }
  schedule();
}

function onStatus(): void {
  const gen = aiGenStatus();
  const done = finishedBetween(lastActive, gen);
  lastActive = activeJob(gen);
  if (done) finish(done);
  checkConfirm();
  schedule();
  notify();
}

let installed = false;

/** Called once by main.ts: follows the AI status and the route */
export function installAiGen(): void {
  if (installed) return;
  installed = true;
  onAiStatusChange(onStatus);
  onRouteChange((from, to) => {
    if (from.name === 'quiz' && to.name !== 'quiz' && libraryPending) {
      libraryPending = false;
      redrawLibrary();
    }
    checkConfirm();
    schedule();
  });
  document.addEventListener('visibilitychange', () => schedule());
  onClearUserData(() => {
    upload?.abort();
    upload = null;
    lastActive = null;
    libraryPending = false;
    confirmDialogShown.clear();
    recognisedToastShown.clear();
    if (timer != null) clearTimeout(timer);
    timer = null;
    notify();
  });
  // a status that was read before the install
  void aiStatus();
  onStatus();
}
