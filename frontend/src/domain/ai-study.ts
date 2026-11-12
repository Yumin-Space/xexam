// The state of one AI study answer (v0.9): a hint, an explanation or a record summary, as the panel and the overlay show
// it. Pure: no DOM, no i18n (it runs under `node --test`). One `Entry` per cache key holds
//   · `shown`   the newest FINISHED answer (or nothing yet), the one the window shows;
//   · `attempt` the newest attempt: a running, stopped, failed or withheld answer, or the finished one.
// A regenerate that fails, is stopped or is withheld never hides the previous good answer (spec 2.3, 7.8): the view then
// shows `shown` under a note saying what happened. `reduce` is the only way an entry changes; `entryView` says what to paint.
import type { ReplyLang, RunningInfo, Sample, StudyKind, StudySource } from './ai-plus.ts';

export type AnswerStatus = 'streaming' | 'done' | 'stopped' | 'error';

/** The code the server gives a hint that gave the answer away; its text is never shown */
export const HINT_LEAK = 'ai.hintLeak';

export interface Answer {
  messageId: number | null;
  status: AnswerStatus;
  /** An error code (`ai.hintLeak` = withheld) */
  error: string | null;
  truncated: boolean;
  content: string;
  lang: ReplyLang | null;
  createdAt: number | null;
  finishedAt: number | null;
  /** Summaries only */
  sample: Sample | null;
  /** A reasoning model is thinking: the `thinking` event came and no text yet (live only, never saved) */
  thinking?: boolean;
}

/** The answer of GET /api/ai/study/saved and /api/ai/summary/saved, normalised */
export interface SavedPayload {
  chatId: number | null;
  /** The chat's real title (the server's; null = unknown) */
  chatTitle?: string | null;
  latest: Answer | null;
  lastGood: Answer | null;
}

/** A request that failed before (or instead of) a stream: an HTTP refusal or a network problem */
export interface AskError {
  /** 0 = no HTTP answer (offline, lost connection) */
  status: number;
  code: string | null;
  /** `running` of a 409 ai.busy */
  running?: RunningInfo | null;
  /** The number a reached limit (ai.chatLimit / ai.messageLimit) came with */
  limit?: number;
  /** The per-user call cap behind an `ai.callLimit` refusal (spec 3.6) */
  quota?: CallBlock;
}

/** A reached per-user AI call cap (spec 3.6; structurally the same as `AiCallBlock` of types/api.ts, kept here so the domain stays free of it) */
export interface CallBlock {
  cap: 'minute' | 'hour' | 'day';
  limit: number;
  /** unix seconds when the block ends */
  retryAt: number;
}

/** The code of a refusal by the per-user call caps */
export const CALL_LIMIT = 'ai.callLimit';

/** A `CallBlock` out of any value (a thrown error's `quota`); null when a field is missing or wrong */
export function callBlockOf(v: unknown): CallBlock | null {
  const o = obj(v);
  const cap = o.cap;
  const limit = numOrNull(o.limit);
  const retryAt = numOrNull(o.retryAt);
  if ((cap !== 'minute' && cap !== 'hour' && cap !== 'day') || limit == null || retryAt == null) return null;
  return { cap, limit, retryAt };
}

export interface Entry {
  chatId: number | null;
  /** The real title of that chat, as the server says it (shown in the "saved in AI Chat" tip); null = not known */
  chatTitle: string | null;
  shown: Answer | null;
  attempt: Answer | null;
  /** The saved answer is being read */
  loading: boolean;
  /** An ask was sent and its stream has not started yet */
  requesting: boolean;
  /** The server is still writing the attempt and this page has no stream for it: poll the saved route every 2 s */
  polling: boolean;
  /** The last request failed before a stream (shown as a note; cleared by the next ask) */
  failure: AskError | null;
}

export const emptyEntry = (): Entry => ({ chatId: null, chatTitle: null, shown: null, attempt: null, loading: false, requesting: false, polling: false, failure: null });

// ---------------------------------------------------------------- keys

export const studyKey = (i: { mode: 'practice' | 'exam'; sessionKey: string; source: StudySource; kind: StudyKind; questionId: string }): string =>
  `${i.mode}|${i.sessionKey}|${i.source}|${i.kind}|${i.questionId}`;

/**
 * The session part of a cache key (answer A7): `k<key>` for a session key, `r<recordId>` for a record whose key is unknown.
 * A record that has a key uses `k<key>`, so answers saved on its result page show on its detail page.
 */
export const sessionPart = (i: { key: string | null; recordId: number | null }): string | null =>
  i.key != null ? `k${i.key}` : i.recordId != null ? `r${i.recordId}` : null;

export const summaryKey = (recordId: number, atDigitsText: string | null): string => `${recordId}@${atDigitsText ?? ''}`;

/** The cache bound of the panel (and the overlay): the oldest entries are dropped */
export const MAX_ENTRIES = 200;

/** Sets `key` (making it the newest) and drops the oldest entries beyond `max` */
export function boundedSet<V>(map: Map<string, V>, key: string, value: V, max = MAX_ENTRIES): void {
  map.delete(key);
  map.set(key, value);
  while (map.size > max) {
    const oldest = map.keys().next();
    if (oldest.done) break;
    map.delete(oldest.value);
  }
}

// ---------------------------------------------------------------- parsing (the wire shapes of spec 5.4)

const obj = (v: unknown): Record<string, unknown> => (typeof v === 'object' && v !== null ? (v as Record<string, unknown>) : {});
const numOrNull = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const strOrNull = (v: unknown): string | null => (typeof v === 'string' && v !== '' ? v : null);
const LANGS = new Set(['en', 'zh-CN', 'ja']);
const STATUSES = new Set(['streaming', 'done', 'stopped', 'error']);

function sampleOf(v: unknown): Sample | null {
  const o = obj(v);
  const used = numOrNull(o.used);
  const usable = numOrNull(o.usable);
  const wrong = numOrNull(o.wrong);
  return used == null || usable == null || wrong == null ? null : { used, usable, wrong };
}

/** One StudyAnswer / SummaryAnswer; null when it does not have the shape */
export function answerOf(v: unknown): Answer | null {
  const o = obj(v);
  if (typeof o.status !== 'string' || !STATUSES.has(o.status)) return null;
  return {
    messageId: numOrNull(o.messageId),
    status: o.status as AnswerStatus,
    error: typeof o.error === 'string' && o.error ? o.error : null,
    truncated: o.truncated === true,
    content: typeof o.content === 'string' ? o.content : '',
    lang: typeof o.lang === 'string' && LANGS.has(o.lang) ? (o.lang as ReplyLang) : null,
    createdAt: numOrNull(o.createdAt),
    finishedAt: numOrNull(o.finishedAt),
    sample: sampleOf(o.sample),
  };
}

/** The saved answers of a saved route: `{chatId, latest, lastGood}` */
export function parseSaved(json: unknown): SavedPayload {
  const o = obj(json);
  return { chatId: numOrNull(o.chatId), chatTitle: strOrNull(o.chatTitle), latest: answerOf(o.latest), lastGood: answerOf(o.lastGood) };
}

/** The `running` object of a 409 ai.busy answer, or null */
export function runningOf(v: unknown): RunningInfo | null {
  const o = obj(v);
  const chatId = numOrNull(o.chatId);
  const messageId = numOrNull(o.messageId);
  if (chatId == null || messageId == null) return null;
  const kinds = ['chat', 'hint', 'explain', 'summary'];
  const chatKinds = ['chat', 'practice', 'exam', 'summary'];
  const sources = ['practice', 'result'];
  return {
    chatId, messageId,
    chatKind: (typeof o.chatKind === 'string' && chatKinds.includes(o.chatKind) ? o.chatKind : 'chat') as RunningInfo['chatKind'],
    kind: (typeof o.kind === 'string' && kinds.includes(o.kind) ? o.kind : 'chat') as RunningInfo['kind'],
    questionId: typeof o.questionId === 'string' ? o.questionId : null,
    questionNo: numOrNull(o.questionNo),
    source: (typeof o.source === 'string' && sources.includes(o.source) ? o.source : null) as StudySource | null,
    recordId: numOrNull(o.recordId),
  };
}

// ---------------------------------------------------------------- the stream events (spec 5.6)

export interface StreamEvent {
  event: string;
  data: unknown;
}

export const isTerminalEvent = (event: string): boolean => event === 'done' || event === 'stopped' || event === 'error';

// ---------------------------------------------------------------- the reducer

export type Action =
  | { type: 'loading' }
  | { type: 'saved'; saved: SavedPayload }
  | { type: 'ask' }
  | { type: 'event'; ev: StreamEvent; now?: number }
  | { type: 'askFailed'; error: AskError }
  | { type: 'localStop' }
  | { type: 'lost' }
  | { type: 'loadFailed'; error: AskError };

const blank = (messageId: number | null, now: number | null): Answer => ({
  messageId, status: 'streaming', error: null, truncated: false, content: '', lang: null, createdAt: now, finishedAt: null, sample: null,
});

const isGood = (a: Answer | null): a is Answer => a != null && a.status === 'done' && a.content !== '';

export function reduce(e: Entry, a: Action): Entry {
  switch (a.type) {
    case 'loading':
      return { ...e, loading: true, failure: null };
    case 'saved': {
      const { chatId, latest, lastGood } = a.saved;
      const chatTitle = chatId == null ? null : a.saved.chatTitle || (chatId === e.chatId ? e.chatTitle : null);
      // Nothing saved: the chat may exist (other questions) but this answer does not
      // (a chat that is gone too: a deleted chat must not keep its link)
      if (!latest) return { ...e, chatId, chatTitle, loading: false, polling: false, attempt: null, shown: null };
      const shown = isGood(latest) ? latest : lastGood;
      return {
        ...e, chatId: chatId ?? e.chatId, chatTitle, loading: false, requesting: false, failure: null, attempt: latest, shown,
        polling: latest.status === 'streaming',
      };
    }
    case 'ask':
      return { ...e, requesting: true, polling: false, failure: null };
    case 'event':
      return onEvent(e, a.ev, a.now ?? null);
    case 'askFailed':
      return { ...e, requesting: false, loading: false, failure: a.error };
    case 'loadFailed':
      return { ...e, loading: false, polling: false, failure: a.error };
    case 'localStop': {
      const at = e.attempt;
      const stopped = at && at.status === 'streaming' ? { ...at, status: 'stopped' as const } : at;
      return { ...e, requesting: false, polling: false, attempt: stopped };
    }
    case 'lost':
      // The stream broke while the server may still be writing: keep the text, ask the saved route
      return e.attempt && e.attempt.status === 'streaming' ? { ...e, polling: true } : { ...e, requesting: false };
  }
}

function onEvent(e: Entry, ev: StreamEvent, now: number | null): Entry {
  const d = obj(ev.data);
  switch (ev.event) {
    case 'start': {
      const chatId = numOrNull(d.chatId);
      const lang = typeof d.lang === 'string' && LANGS.has(d.lang) ? (d.lang as ReplyLang) : null;
      const sample = sampleOf(d.sample);
      const at: Answer = { ...blank(numOrNull(d.assistantMessageId), now), lang, sample };
      // Without a title in the event, the known one stays only while the chat is the same one
      const chatTitle = strOrNull(d.chatTitle) ?? (chatId == null || chatId === e.chatId ? e.chatTitle : null);
      return { ...e, chatId: chatId ?? e.chatId, chatTitle, attempt: at, requesting: false, polling: false, failure: null };
    }
    case 'delta': {
      const text = typeof d.text === 'string' ? d.text : '';
      const at = e.attempt;
      if (!text || !at || at.status !== 'streaming') return e;
      return { ...e, attempt: { ...at, content: at.content + text, thinking: false } };
    }
    case 'thinking': {
      const at = e.attempt;
      if (!at || at.status !== 'streaming' || at.content !== '') return e;
      return { ...e, attempt: { ...at, thinking: true } };
    }
    case 'done': {
      const at = e.attempt;
      if (!at) return e;
      const done: Answer = { ...at, status: 'done', truncated: d.truncated === true, finishedAt: now };
      return { ...e, attempt: done, shown: isGood(done) ? done : e.shown, requesting: false, polling: false };
    }
    case 'stopped': {
      const at = e.attempt;
      return at ? { ...e, attempt: { ...at, status: 'stopped', finishedAt: now }, requesting: false, polling: false } : e;
    }
    case 'error': {
      const at = e.attempt ?? blank(null, now);
      const code = typeof d.code === 'string' && d.code ? d.code : 'ai.internal';
      // `withdraw`: the withheld text is cleared at once, whatever was streamed
      const content = d.withdraw === true || code === HINT_LEAK ? '' : at.content;
      return { ...e, attempt: { ...at, status: 'error', error: code, content, finishedAt: now }, requesting: false, polling: false };
    }
    default:
      return e;
  }
}

// ---------------------------------------------------------------- the view (what to paint)

export type Phase = 'loading' | 'empty' | 'requesting' | 'streaming' | 'done' | 'stopped' | 'error' | 'withheld';
export type ReasonKind = 'stopped' | 'error';

export type Note =
  | { kind: 'busy'; running: RunningInfo | null }
  | { kind: 'error'; code: string | null; status: number; limit?: number; quota?: CallBlock }
  | { kind: 'withheld' }
  | { kind: 'withheldPrev' }
  | { kind: 'failedPrev'; reason: ReasonKind; code: string | null }
  | { kind: 'otherLang'; lang: ReplyLang };

export interface EntryView {
  text: string;
  phase: Phase;
  note: Note | null;
  canRegenerate: boolean;
  canStop: boolean;
  /** unix seconds of the shown answer's end, for "Saved 20:44" */
  savedAt: number | null;
  sample: Sample | null;
  truncated: boolean;
  /** Streaming, but still thinking (a reasoning model, no text yet) */
  thinking?: boolean;
}

/**
 * What the window paints for an entry. `settingsLang` = the current Settings language (a shown answer in another one gets
 * the otherLang note). One note at most; busy and request errors beat failed-regenerate notes beat the language note.
 */
export function entryView(entry: Entry, settingsLang: ReplyLang, nowSec?: number): EntryView {
  // A call-cap refusal expires by itself (spec 3.6): the saved answer or the "not asked" prompt shows again
  const e = nowSec != null && entry.failure?.code === CALL_LIMIT && entry.failure.quota != null && entry.failure.quota.retryAt <= nowSec
    ? { ...entry, failure: null }
    : entry;
  const base = { sample: null as Sample | null, truncated: false, savedAt: null as number | null };
  if (e.loading && !e.attempt && !e.shown) {
    return { ...base, text: '', phase: 'loading', note: null, canRegenerate: false, canStop: false };
  }
  const at = e.attempt;
  const shown = e.shown;

  // A request error (busy, refused, offline) before any stream: the previous good answer stays
  if (e.failure && !e.requesting) {
    const note: Note = e.failure.code === 'ai.busy'
      ? { kind: 'busy', running: e.failure.running ?? null }
      : {
        kind: 'error', code: e.failure.code, status: e.failure.status,
        ...(e.failure.limit != null ? { limit: e.failure.limit } : {}), ...(e.failure.quota ? { quota: e.failure.quota } : {}),
      };
    return {
      text: shown?.content ?? '', phase: shown ? 'done' : 'error', note, canRegenerate: !(e.failure.code === 'ai.busy' && e.failure.running != null), canStop: false,
      savedAt: shown?.finishedAt ?? null, sample: shown?.sample ?? null, truncated: shown?.truncated ?? false,
    };
  }
  if (e.requesting && !(at && at.status === 'streaming')) {
    return {
      text: shown?.content ?? '', phase: 'requesting', note: null, canRegenerate: false, canStop: true, savedAt: null,
      sample: shown?.sample ?? null, truncated: false,
    };
  }
  if (!at) {
    return { ...base, text: shown?.content ?? '', phase: shown ? 'done' : 'empty', note: null, canRegenerate: true, canStop: false };
  }
  if (at.status === 'streaming') {
    return {
      text: at.content, phase: 'streaming', note: null, canRegenerate: false, canStop: e.chatId != null || !e.polling,
      savedAt: null, sample: at.sample, truncated: false, thinking: at.thinking === true && at.content === '',
    };
  }
  if (at.status === 'done') {
    const other = at.lang != null && at.lang !== settingsLang;
    return {
      text: at.content, phase: 'done', note: other ? { kind: 'otherLang', lang: at.lang as ReplyLang } : null, canRegenerate: true,
      canStop: false, savedAt: at.finishedAt, sample: at.sample, truncated: at.truncated,
    };
  }
  // stopped or error
  const withheld = at.status === 'error' && at.error === HINT_LEAK;
  if (isGood(shown)) {
    const note: Note = withheld ? { kind: 'withheldPrev' } : { kind: 'failedPrev', reason: at.status === 'stopped' ? 'stopped' : 'error', code: at.error };
    return {
      text: shown.content, phase: 'done', note, canRegenerate: true, canStop: false, savedAt: shown.finishedAt, sample: shown.sample,
      truncated: shown.truncated,
    };
  }
  if (withheld) return { ...base, text: '', phase: 'withheld', note: { kind: 'withheld' }, canRegenerate: true, canStop: false };
  if (at.status === 'stopped') {
    return { ...base, text: at.content, phase: 'stopped', note: null, canRegenerate: true, canStop: false, sample: at.sample };
  }
  return {
    ...base, text: at.content, phase: 'error', note: { kind: 'error', code: at.error, status: 0 }, canRegenerate: true, canStop: false, sample: at.sample,
  };
}

// ---------------------------------------------------------------- small rules

/** A saved answer written in another language than the current Settings language (note: Regenerate to get it in yours) */
export const isOtherLang = (a: Pick<Answer, 'lang'> | null, settingsLang: ReplyLang): boolean => a != null && a.lang != null && a.lang !== settingsLang;

const REFRESH_CODES = new Set(['ai.disabled', 'ai.notAllowed', 'ai.suspended', 'ai.unavailable']);

/** A refusal that means the AI status the page holds is stale: refresh it (forced) and redraw the controls */
export const refreshesStatus = (e: AskError): boolean =>
  (e.code != null && REFRESH_CODES.has(e.code)) || (e.status === 503 && e.code === 'ai.backendDown');

/** Refusals of a summary that are shown in the body (nothing to summarise) */
export const isNothingToSummarise = (code: string | null): boolean => code === 'ai.noWrong' || code === 'ai.noUsable' || code === 'ai.noDetail';

export function isAskError(v: unknown): v is AskError {
  const o = obj(v);
  return typeof o.status === 'number' && (o.code === null || typeof o.code === 'string');
}
