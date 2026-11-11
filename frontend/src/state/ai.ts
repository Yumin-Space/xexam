// What the signed-in user may do with AI (v0.8): the answer of GET /api/ai/status, kept here for the menu, the header
// badge and the AI Chat page. No DOM (it runs under `node --test`). Round 2: `enabled` is false also while ai-service has not
// reached the main service for 60 s, and `suspended` says that an administrator suspended the user's AI+ function.
//   · 'loading' after sign-in and on a reload, until the first answer (so AI Chat never flashes "Not allowed");
//   · 'unavailable' when the AI service cannot be reached (Nginx's 502, a 503, no JSON, offline), silently: the menu
//     then has no AI Chat and nothing else changes; also while a new password must be chosen (it is not asked then);
//   · 'ready' with the answer.
// It is read after signing in, again at most every 60 s when the session is checked (on page changes, a tab coming back),
// and at once after an administrator changes AI settings or someone's AI+.
import { fetchAiStatus } from '../api/ai.ts';
import type { AiStatusJson } from '../types/api.ts';

export type AiStatus =
  | { state: 'loading' }
  | { state: 'unavailable' }
  | ({ state: 'ready' } & AiStatusJson);

/** A ready status is read again after this long (when the session is checked) */
export const AI_STATUS_MAX_AGE_MS = 60_000;

let status: AiStatus = { state: 'loading' };
let readAt = 0;
/** Changes when the person at the page may change (sign-out): an answer for the previous user is dropped */
let generation = 0;
let inflight: Promise<boolean> | null = null;

export const aiStatus = (): AiStatus => status;

/** Signing out (or another user): back to 'loading' for the next user */
export function resetAiStatus(): void {
  status = { state: 'loading' };
  readAt = 0;
  generation += 1;
  inflight = null;
}

/** For the tests: sets the status directly */
export function setAiStatusForTest(s: AiStatus, at = 0): void {
  status = s;
  readAt = at;
}

const bool = (v: unknown): boolean => v === true;
const count = (v: unknown, fallback: number): number => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : fallback);

/** The status for an answer of GET /api/ai/status: 'ready' when it has the expected shape, else 'unavailable' */
export function statusOf(json: unknown): AiStatus {
  if (typeof json !== 'object' || json === null) return { state: 'unavailable' };
  const j = json as Record<string, unknown>;
  if (typeof j.enabled !== 'boolean' || typeof j.allowed !== 'boolean') return { state: 'unavailable' };
  const r = typeof j.running === 'object' && j.running !== null ? (j.running as Record<string, unknown>) : null;
  const running = r && typeof r.chatId === 'number' && typeof r.messageId === 'number' ? { chatId: r.chatId, messageId: r.messageId } : null;
  const messageChars = count(j.messageChars ?? j.maxMessageChars, 16000);
  const memoryChars = count(j.memoryChars ?? j.memoryMaxChars, 300);
  return {
    state: 'ready',
    enabled: j.enabled,
    allowed: j.allowed,
    suspended: bool(j.suspended),
    hasData: bool(j.hasData),
    running,
    retentionDays: count(j.retentionDays, 90),
    memoryLimit: count(j.memoryLimit, 50),
    memoryMaxChars: memoryChars,
    maxMessageChars: messageChars,
    messageChars,
    memoryChars,
    keyProblem: bool(j.keyProblem),
    configured: bool(j.configured),
    freeMode: bool(j.freeMode),
  };
}

/** Whether the page must be drawn again for a new status: the state, enabled, allowed, suspended or hasData changed */
export function aiNeedsRender(prev: AiStatus, next: AiStatus): boolean {
  if (prev.state !== next.state) return true;
  if (prev.state !== 'ready' || next.state !== 'ready') return false;
  return prev.enabled !== next.enabled || prev.allowed !== next.allowed || prev.suspended !== next.suspended || prev.hasData !== next.hasData;
}

/** The header badge (and the account areas): the user can use AI right now */
export const aiBadgeOn = (s: AiStatus = status): boolean => s.state === 'ready' && s.allowed;

/** AI Chat in the menu: usable now, or the user has chats or memories to read and delete */
export const aiChatVisible = (s: AiStatus = status): boolean => s.state === 'ready' && (s.allowed || s.hasData);

/** The administrator's free mode is on (always false for others) */
export const aiFreeMode = (s: AiStatus = status): boolean => s.state === 'ready' && s.freeMode;

/** The free mode was switched (the answer of POST /api/ai/free): the kept status follows at once */
export function setAiFreeMode(on: boolean): void {
  if (status.state === 'ready') status = { ...status, freeMode: on };
}

/** Settings > AI chat mode: administrators only, and only while AI can be used (the service is on) */
export const aiModeShown = (admin: boolean, s: AiStatus = status): boolean => admin && s.state === 'ready' && s.enabled;

/** AI is enabled (the badges of other users are shown in colour) */
export const aiEnabled = (s: AiStatus = status): boolean => s.state === 'ready' && s.enabled;

/** v0.8 round 2: an administrator suspended the user's AI+ function (the badge is dark; AI Chat is read only) */
export const aiSuspended = (s: AiStatus = status): boolean => s.state === 'ready' && s.suspended;

export interface RefreshOptions {
  /** Read now even when the last answer is recent */
  force?: boolean;
  /** The user must choose a new password first: the status is not asked (it stays 'unavailable') */
  mustChangePassword: boolean;
  /** For the tests */
  now?: number;
  fetcher?: () => Promise<unknown>;
}

/**
 * Reads the status (at most every 60 s unless forced; one request at a time). Resolves true when the page must be drawn
 * again (aiNeedsRender). Never rejects: any failure is 'unavailable'.
 */
export function refreshAiStatus(o: RefreshOptions): Promise<boolean> {
  const now = o.now ?? Date.now();
  if (o.mustChangePassword) {
    const prev = status;
    status = { state: 'unavailable' };
    readAt = now;
    return Promise.resolve(aiNeedsRender(prev, status));
  }
  if (inflight) {
    // A forced read after a change must not take the answer of a request that started before it
    if (!o.force) return inflight;
    const before = inflight;
    return before.then((a) => refreshAiStatus({ ...o, now: undefined }).then((b) => a || b));
  }
  if (!o.force && status.state !== 'loading' && now - readAt < AI_STATUS_MAX_AGE_MS) return Promise.resolve(false);
  const mine = generation;
  const fetcher = o.fetcher ?? fetchAiStatus;
  const run = (async (): Promise<boolean> => {
    let next: AiStatus;
    try {
      next = statusOf(await fetcher());
    } catch {
      next = { state: 'unavailable' };
    }
    if (mine !== generation) return false;
    const prev = status;
    status = next;
    readAt = o.now ?? Date.now();
    return aiNeedsRender(prev, next);
  })();
  inflight = run;
  void run.finally(() => {
    if (inflight === run) inflight = null;
  });
  return run;
}
