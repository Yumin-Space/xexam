// v2.6: the state of the Home AI study assessment card: the saved text (read once per visit, no browser storage) and a
// generation that streams into the card. The page draws from assessView(); the pure rules are domain/dash-assess.ts.
// A generation goes on when the person leaves Home (the server saves it); coming back shows the saved text again.
import { askAssess, fetchAssessLatest, stopAnswer } from '../api/ai.ts';
import { isAbortError } from '../api/client.ts';
import { buildAssessAsk } from '../domain/ai-plus.ts';
import { asAskError } from '../domain/ai-run.ts';
import { refreshesStatus } from '../domain/ai-study.ts';
import { IDLE, STARTING, afterRun, runReduce, type AssessLatest, type AssessRun } from '../domain/dash-assess.ts';
import { uiLang } from '../i18n/index.ts';
import { onRouteChange, render } from './actions.ts';
import { aiStatus, noteCallBlock, refreshAiStatus } from './ai.ts';
import { onClearUserData, S } from './store.ts';

let latest: AssessLatest | null = null;
let run: AssessRun = IDLE;
let ctrl: AbortController | null = null;
let chatId: number | null = null;
let inflight = false;
const POLL_MS = 5000;
let pollTimer: ReturnType<typeof setTimeout> | null = null;
/** The current visit already read the saved text (a re-render must not read again) */
let asked = false;
/** Bumped on sign-out: an answer for the previous user is dropped */
let epoch = 0;

export interface AssessView {
  latest: AssessLatest | null;
  run: AssessRun;
}

export const assessView = (): AssessView => ({ latest, run });

const paint = (): void => {
  if (S.route.name === 'home') render();
};

function readLatest(): void {
  if (inflight) return;
  inflight = true;
  asked = true;
  const mine = epoch;
  void fetchAssessLatest()
    .then((r) => {
      if (mine === epoch) latest = r;
    })
    // The card is an extra: a failed read leaves what is shown (or no card), never an error block on Home
    .catch(() => {})
    .finally(() => {
      inflight = false;
      if (mine !== epoch) return;
      paint();
      // A run that goes on elsewhere (a reload, a second tab): read again until its text is saved
      if (latest?.running) {
        if (pollTimer) clearTimeout(pollTimer);
        pollTimer = setTimeout(() => {
          pollTimer = null;
          if (mine === epoch && S.route.name === 'home' && run.phase !== 'streaming') readLatest();
        }, POLL_MS);
      }
    });
}

/** Called by every paint of Home: one read per visit; the status of the AI service is not needed to read the saved text */
export function ensureAssess(): void {
  if (!asked && run.phase !== 'streaming' && aiStatus().state !== 'unavailable') readLatest();
}

/** Starts a generation that streams into the card; the old text stays until the new one is saved */
export function generateAssess(): void {
  if (run.phase === 'streaming') return;
  const mine = epoch;
  const ac = new AbortController();
  ctrl = ac;
  chatId = null;
  run = STARTING;
  paint();
  const finish = (next: AssessRun): void => {
    if (mine !== epoch || ctrl !== ac) return;
    run = next;
    paint();
  };
  void askAssess(buildAssessAsk(uiLang()), {
    signal: ac.signal,
    onEvent: (ev) => {
      if (mine !== epoch || ctrl !== ac) return;
      if (ev.event === 'start') {
        const id = (ev.data as { chatId?: unknown } | null)?.chatId;
        if (typeof id === 'number') chatId = id;
      }
      run = runReduce(run, ev);
      paint();
    },
  }).then(() => {
    if (mine !== epoch || ctrl !== ac) return;
    // The stream ended without an end event (a lost connection): the server may still save it; read what is saved
    if (run.phase === 'streaming') run = IDLE;
    onEnded();
  }).catch((e: unknown) => {
    if (isAbortError(e)) return finish(IDLE);
    const error = asAskError(e);
    if (error.quota) noteCallBlock(error.quota);
    if (refreshesStatus(error)) void refreshAiStatus({ force: true, mustChangePassword: false }).then((changed) => { if (changed) paint(); });
    // Today's runs are used up (another tab, or a stale card): the button follows without another read
    // (the disabled button and its note say it, so no second error line)
    if (error.code === 'ai.assessDaily' && latest) { latest = { ...latest, remaining: 0 }; return finish(IDLE); }
    finish({ phase: 'error', error });
  });
}

/** A finished run: the saved copy replaces the old text; until it is read, the new text stands (and today's count goes down) */
function onEnded(): void {
  const mine = epoch;
  const text = run.phase === 'done' ? run.text.trim() : '';
  if (text && latest) {
    const day = latest.today;
    latest = { ...latest, remaining: afterRun(latest.remaining), assess: { text, day, createdAt: Math.floor(Date.now() / 1000), latestAt: null, records: null, answered: null, model: latest.assess?.model ?? null } };
  }
  paint();
  void fetchAssessLatest()
    .then((r) => { if (mine === epoch) latest = r; })
    .catch(() => {})
    .finally(() => {
      if (mine !== epoch) return;
      if (run.phase === 'done') run = IDLE;
      paint();
    });
}

/** Stop: abort the stream and stop the chat's answer (the old text stays) */
export function stopAssess(): void {
  const ac = ctrl;
  if (!ac || run.phase !== 'streaming') return;
  ac.abort();
  if (chatId != null) void stopAnswer(chatId).catch(() => {});
}

onRouteChange((from) => {
  if (from.name === 'home') {
    asked = false;
    // An error is a note of that visit
    if (run.phase === 'error') run = IDLE;
  }
});
onClearUserData(() => {
  epoch++;
  ctrl?.abort();
  ctrl = null;
  latest = null;
  run = IDLE;
  chatId = null;
  inflight = false;
  asked = false;
  if (pollTimer) clearTimeout(pollTimer);
  pollTimer = null;
});
