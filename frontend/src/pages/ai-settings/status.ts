// AI settings page: the link to ai-service (the status panel with its cause, rechecked every 10 s while it shows) and the four
// status cards at the top of every tab (AI+ switch, connections, default model, features). Each card is a button that jumps
// to its section. A default model that cannot be used is a warning on its card and its row; nothing is switched on its own.
import { fetchAiStatus } from '../../api/ai.ts';
import { fetchAuthState } from '../../api/auth.ts';
import { ApiError, isSignInError } from '../../api/client.ts';
import { miniIcon, type MiniIcon } from '../../components/mini-icons.ts';
import { offLabelKeys } from '../../domain/ai-features.ts';
import { aiServiceOf, serviceCause, statusCards, type StatusCard, type SvcCause, type SvcProbe } from '../../domain/ai-settings.ts';
import { aiCodeText } from '../../i18n/errors.ts';
import { t, uiLang, type MsgKey } from '../../i18n/index.ts';
import type { AiService, AiSettings } from '../../types/api.ts';
import { h } from '../../utils/dom.ts';
import { A, hooks, listSep, onPage, redraw, RECHECK_MS, when } from './state.ts';

/** The text of each cause of the status panel (contracts 2.9 / 2.10) */
const CAUSE_TEXT: Record<Exclude<SvcCause, 'connected'>, MsgKey> = {
  backendDown: 'ai.err.backendDown', linkOff: 'ai.set.svc.linkOff', notRunning: 'ai.set.svc.notRunning', stopped: 'ai.set.svc.stopped',
  notConnected: 'ai.set.svc.notConnected', waiting: 'ai.set.svc.waiting',
};

/** The AI service is not there (Nginx's 502, no JSON, offline) */
function isUnreachable(e: unknown): boolean {
  if (!(e instanceof ApiError)) return false;
  if (e.code === 'ai.unavailable') return true;
  return e.kind === 'network' || (e.kind === 'status' && e.status >= 500);
}

export function stopRecheck(): void {
  if (A.svcTimer != null) window.clearTimeout(A.svcTimer);
  A.svcTimer = null;
}

function scheduleRecheck(): void {
  stopRecheck();
  if (A.phase === 'panel' && onPage()) A.svcTimer = window.setTimeout(() => void checkService(), RECHECK_MS);
}

/** The one probe of ai-service itself while the main service does not see it: answers, refuses (not connected), or is gone */
async function probeService(): Promise<SvcProbe> {
  try {
    await fetchAiStatus();
    return 'ok';
  } catch (e) {
    if (e instanceof ApiError && e.code === 'ai.backendDown') return 'refused';
    if (isUnreachable(e)) return 'down';
    // Any other answer of its own (JSON with a code): it runs
    return 'ok';
  }
}

function showPanel(c: Exclude<SvcCause, 'connected'>): void {
  A.phase = 'panel';
  A.cause = c;
  if (!A.panelSince) A.panelSince = Date.now();
}

/** What the page shows of the connection: a check that changes none of it does not redraw the page */
const svcKey = (): string => [A.phase, A.cause, A.svc?.connected, A.svc?.lastSeen, A.svc?.version, A.svc?.stopped].join('|');

/**
 * Reads the state of ai-service from the main service and decides what the page shows: the settings (loaded now) while it
 * is connected, else the status panel with its cause (checked again every 10 s while the page is open). `manual`: Retry.
 */
export async function checkService(manual = false): Promise<void> {
  if (A.svcBusy) return;
  A.svcBusy = true;
  A.wanted = false;
  stopRecheck();
  const before = svcKey();
  let a: AiService | null = null;
  let failed = false;
  try {
    const st = await fetchAuthState();
    // Signed out meanwhile: the session check shows the sign-in page
    if (!st.user) {
      A.svcBusy = false;
      return;
    }
    a = aiServiceOf(st);
  } catch (e) {
    if (isSignInError(e)) {
      A.svcBusy = false;
      return;
    }
    failed = true;
  }
  A.svc = failed ? A.svc : a;
  let next: SvcCause;
  if (failed) {
    next = 'backendDown';
  } else if (!a) {
    // An answer without aiService (a main service older than this page): ai-service itself tells
    const probe = await probeService();
    next = probe === 'ok' ? 'connected' : probe === 'refused' ? 'notConnected' : 'notRunning';
  } else if (a.connected && A.failedAs) {
    // The main service still counts the last report (up to 60 s), but a call of this page failed: ask ai-service itself
    const probe = await probeService();
    next = probe === 'ok' ? 'connected' : probe === 'refused' ? 'notConnected' : 'notRunning';
  } else {
    const probe = !a.connected && a.link ? await probeService() : null;
    next = serviceCause({ state: { aiService: a }, probe, waitedMs: A.panelSince ? Date.now() - A.panelSince : 0 });
  }
  A.svcBusy = false;
  if (next === 'connected') {
    A.failedAs = null;
    A.phase = 'settings';
    A.cause = null;
    A.panelSince = 0;
    if (!A.loading) void hooks.load();
  } else {
    showPanel(next);
    scheduleRecheck();
  }
  if (onPage() && (manual || next === 'connected' || svcKey() !== before)) redraw();
}

/**
 * A call of this page failed. ai-service gone (502, no JSON, offline) or not connected (503 `ai.backendDown`): the status
 * panel takes over and the page checks why; 403 `ai.adminOnly`: "Not allowed". True when handled here; anything else is shown
 * where it happened.
 */
export function aiDown(e: unknown): boolean {
  if (e instanceof ApiError && e.status === 403 && e.code === 'ai.adminOnly') {
    A.forbidden = true;
    redraw();
    return true;
  }
  const why = e instanceof ApiError && e.code === 'ai.backendDown' ? 'notConnected' : isUnreachable(e) ? 'notRunning' : null;
  if (!why) return false;
  A.failedAs = why;
  if (A.phase !== 'panel') showPanel(why);
  // At most one immediate check per 10 s (several calls fail at once; a flapping service must not loop)
  const soon = Date.now() - A.lastDownAt < RECHECK_MS;
  A.lastDownAt = Date.now();
  if (soon) scheduleRecheck();
  else void checkService();
  redraw();
  return true;
}

/** ai-service is not connected (or the main service cannot be reached): why, the last report, and Retry */
export function servicePanel(): HTMLElement {
  const c = A.cause ?? 'notConnected';
  const a = c === 'backendDown' ? null : A.svc;
  return h('section', { class: 'panel ais-sec ais-svc', 'aria-labelledby': 'ais-svc-h' },
    h('h2', { id: 'ais-svc-h' }, t('ai.set.svc.title')),
    h('p', { class: 'callout warn ais-svc-cause', role: 'status' }, t(CAUSE_TEXT[c])),
    a ? h('p', { class: 'ais-note' }, a.lastSeen != null ? t('ai.set.svc.lastSeen', { time: when(a.lastSeen) }) : t('ai.set.svc.never')) : null,
    a?.version ? h('p', { class: 'ais-note' }, t('ai.set.svc.version', { v: a.version })) : null,
    h('p', { class: 'ais-note' }, t('ai.set.svc.recheck')),
    h('div', { class: 'acts' },
      h('button', { class: 'btn', id: 'ais-svc-retry', type: 'button', 'aria-busy': A.svcBusy ? 'true' : null, onclick: () => void checkService(true) },
        t('common.retry'))));
}

// ---------------------------------------------------------------- the four status cards

const CARD_ICON: Record<StatusCard['id'], MiniIcon> = { main: 'aiGen', conn: 'plug', model: 'chart', features: 'shield' };
const CARD_TITLE: Record<StatusCard['id'], MsgKey> = {
  main: 'ai.set.card.main', conn: 'ai.set.card.conn', model: 'ai.set.card.model', features: 'ai.set.card.features',
};
/** The section each card jumps to (all of them are on the first tab) */
const CARD_TARGET: Record<StatusCard['id'], string> = { main: 'ais-enable-h', conn: 'ais-conn-h', model: 'ais-models-h', features: 'ais-switches-h' };

function jump(id: StatusCard['id']): void {
  hooks.selectTab('feature');
  window.setTimeout(() => {
    const el = document.getElementById(CARD_TARGET[id]);
    if (!el) return;
    el.scrollIntoView({ block: 'start' });
    el.focus({ preventScroll: true });
  }, 0);
}

/** The text of one card's value line (the features card names the features that are off) */
export function cardText(c: StatusCard): string {
  if (c.id === 'features' && c.offFeatures.length) {
    const list = offLabelKeys(c.offFeatures).map((k) => t(k as MsgKey)).join(listSep(uiLang()));
    return t('ai.set.card.features.someOff', { list });
  }
  return t(c.key as MsgKey, c.params);
}

export function statusCardsRow(s: AiSettings): HTMLElement {
  return h('div', { class: 'ais-cards4', role: 'group', 'aria-label': t('ai.set.tab.conn') },
    statusCards(s).map((c) => h('button', {
      type: 'button', class: `ais-scard ${c.tone}`, id: `ais-scard-${c.id}`, onclick: () => jump(c.id),
    },
    h('span', { class: 'ais-scard-ic', 'aria-hidden': 'true' }, miniIcon(CARD_ICON[c.id], 'ais-mi')),
    h('span', { class: 'ais-scard-t' }, t(CARD_TITLE[c.id])),
    h('span', { class: 'ais-scard-v' }, cardText(c)))),
  );
}

/** The red callout when the keys of the whole AI service cannot be read (a master-key problem; every connection is affected) */
export function keyProblemLine(s: AiSettings): HTMLElement | null {
  if (!s.keyProblem) return null;
  return h('p', { class: 'callout bad ais-problem', role: 'alert' }, s.keyProblem === 'invalidEnv' ? t('ai.set.keyEnv') : s.keyProblem === 'invalidFile' ? t('ai.set.keyFile') : t('ai.set.keyProblem'));
}

/** The line under the cards: the connection to ai-service */
export function serviceLine(): HTMLElement | null {
  return A.svc?.connected ? h('p', { class: 'ais-svc-ok' }, t('ai.set.svc.connected', { v: A.svc.version ?? '–', time: when(A.svc.lastSeen) })) : null;
}

/** Why a connection is marked broken, in words */
export const reasonText = (code: string | null): string => aiCodeText(code ?? 'ai.keyProblem');
