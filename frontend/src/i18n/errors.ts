// Turns any error into text in the current UI language.
//   · ApiError from the backend: the backend's (Chinese) message is translated by localizeServerMessage(); a 429 that says how
//     long to wait (retryAfter) becomes "Try again in N minutes / hours / 1 day";
//   · the proxy's rate limit ('tooMany', v0.7.5): "too many requests, wait a few seconds", without the sign-in hint;
//   · network / unexpected / bare status errors: generic texts from the catalog;
//   · LocalError: an error raised by the page itself (catalog key + params), translated lazily so a language switch also switches it.
//   · v0.8: an error of the AI routes carries a stable code: `ai.<x>` is the catalog text `ai.err.<x>` (provider problems
//     `ai.up.*` are `ai.err.up.*`), with the number of a reached limit as {n}; `auth.guard` (Nginx) is `ai.err.guard`;
//     `auth.signin` / `auth.password` carry the backend's own texts and are translated like any backend message.
//   · v1.0: `ai.callLimit` (a per-user AI request cap) with its `quota` (cap, limit, retryAt) becomes callLimitText(): the
//     cap named, when it frees, and for administrators where to change it. Without a usable quota it is the plain sentence.
//   · v1.1: the AI bank codes (`ai.gen.*`, `ai.import.*`, `ai.genCount`, I18N-4 / I18N-7) become their own texts, with the
//     numbers of the answer (`need`, `have`, `limit`, `maxMb`, `ext`) or of a job (`errorArg`): see genCodeText().
import { ApiError } from '../api/client.ts';
import { DEFAULT_LIMITS } from '../domain/ai-settings.ts';
import type { AiCallBlock } from '../types/api.ts';
import { hasKey, localeTag, t, uiLang, type MsgKey, type Params } from './index.ts';
import { localizeServerMessage } from './server-messages.ts';
import { clockText, waitText } from './wait.ts';

let isAdminForTexts: () => boolean = () => false;

/** main.ts registers who is signed in, so cap texts can point administrators to AI settings (i18n does not import the store) */
export function setAdminForTexts(fn: () => boolean): void {
  isAdminForTexts = fn;
}

/**
 * The text of a reached AI request cap: "You have used up your AI requests (20 per hour) ...", then when it frees.
 * Day cap: always "after 0:00 tomorrow" (it resets at the server's midnight); minute / hour: "less than a minute" below
 * 60 s, else "about {wait} ({time})". `admin` swaps the advice for the place of the settings. `nowSec`: unix seconds now.
 */
export function callLimitText(q: AiCallBlock, admin: boolean, nowSec: number): string {
  const cap = t(q.cap === 'minute' ? 'ai.cap.minute' : q.cap === 'hour' ? 'ai.cap.hour' : 'ai.cap.day', { n: q.limit.toLocaleString(localeTag()) });
  const left = Math.max(1, Math.ceil(q.retryAt - nowSec));
  const when = q.cap === 'day'
    ? t('ai.cap.whenDay')
    : left < 60 ? t('ai.cap.whenSoon') : t('ai.cap.when', { wait: waitText(left), time: clockText(q.retryAt, nowSec) });
  return t(admin ? 'ai.cap.msgAdmin' : 'ai.cap.msg', { cap, when });
}

const nowSeconds = (): number => Date.now() / 1000;

/** The fixed cap of one user message in characters (v1.0: no longer an admin setting, spec 3.10 C-8b) */
const MESSAGE_CHARS = 16000;

/** The number an AI limit text states when the answer did not give it: the defaults of the safety limits */
const LIMIT_OF: Record<string, number> = {
  'ai.tooLong': MESSAGE_CHARS,
  'ai.chatFull': DEFAULT_LIMITS.messagesPerChat,
  'ai.messageLimit': DEFAULT_LIMITS.messagesPerChat,
  'ai.chatLimit': DEFAULT_LIMITS.chatsPerUser,
  'ai.memoryFull': DEFAULT_LIMITS.memoriesPerUser,
  'ai.memoryLimit': DEFAULT_LIMITS.memoriesPerUser,
  'ai.memoryTooLong': DEFAULT_LIMITS.memoryTokens,
};

/** The numbers a text of an AI bank code may need (from the error body, or from a job's `errorArg`) */
export interface GenCodeArgs {
  need?: number;
  have?: number;
  what?: string;
  limit?: number;
  max?: number;
  maxMb?: number;
  ext?: string;
  arg?: number | null;
}

const num = (v: unknown): number | undefined => (typeof v === 'number' && Number.isFinite(v) ? v : undefined);

/** The numbers of an error body (an ApiError's `body`) for genCodeText() */
export function genArgsOf(body: Record<string, unknown> | undefined): GenCodeArgs {
  if (!body) return {};
  const out: GenCodeArgs = {};
  const set = <K extends keyof GenCodeArgs>(k: K, v: GenCodeArgs[K] | undefined): void => {
    if (v !== undefined) out[k] = v;
  };
  set('need', num(body['need']));
  set('have', num(body['have']));
  set('limit', num(body['limit']));
  set('max', num(body['max']));
  set('maxMb', num(body['maxMb']));
  if (typeof body['what'] === 'string') out.what = body['what'];
  if (typeof body['ext'] === 'string') out.ext = body['ext'];
  return out;
}

/**
 * v1.1 (I18N-4, I18N-7): the text of an AI bank code (a refusal of the generation / import routes or the `error` of a job),
 * or null when the code is not one of them. Numbers come from `args`.
 */
export function genCodeText(code: string, args: GenCodeArgs = {}): string | null {
  const grouped = (n: number): string => n.toLocaleString(localeTag());
  switch (code) {
    case 'ai.genCount': return t('ai.set.gen.totalBad');
    case 'ai.gen.badName': return t('aigen.dlg.nameBad');
    case 'ai.import.nameTaken': return t('aigen.imp.cfm.nameTaken');
    case 'ai.import.nameEmpty': return t('aigen.imp.cfm.nameNeed');
    case 'ai.gen.busy': return t('aigen.why.busy');
    case 'ai.gen.quota': return args.limit === 0 ? t('aigen.why.quotaZero') : t('aigen.why.quota', { limit: args.limit ?? 0 });
    case 'ai.gen.tooFew':
      return t(args.what === 'wrong' ? 'aigen.why.fewWrong' : 'aigen.why.fewSource', { need: args.need ?? 0, have: args.have ?? 0 });
    case 'ai.gen.forbidden': return t('aigen.err.forbidden');
    case 'ai.gen.aiSource': return t('aigen.why.aiSource');
    case 'ai.gen.notFound': return t('aigen.err.notFound');
    case 'ai.gen.tooLate': return t('aigen.err.tooLate');
    case 'ai.gen.quality': return t('aigen.err.quality');
    case 'ai.gen.stopped': return t('aigen.err.stopped');
    case 'ai.gen.timeout': return t('aigen.err.timeout');
    case 'ai.gen.interrupted': return t('aigen.err.interrupted');
    case 'ai.gen.save': return t('aigen.err.save');
    case 'ai.import.type': return t('aigen.imp.err.type', { ext: args.ext ? `.${args.ext.replace(/^\./, '')}` : '', types: t('aigen.imp.types') });
    case 'ai.import.legacy': return t('aigen.imp.err.legacy');
    case 'ai.import.image': return t('aigen.imp.err.image');
    case 'ai.import.empty': return t('aigen.imp.err.empty');
    case 'ai.import.tooLarge': return t('aigen.imp.err.tooLarge', { mb: args.maxMb ?? 20 });
    case 'ai.import.notWaiting': return t('aigen.imp.err.notWaiting');
    case 'ai.import.noText': return t('aigen.imp.err.noText');
    case 'ai.import.encoding': return t('aigen.imp.err.encoding');
    case 'ai.import.protected': return t('aigen.imp.err.protected');
    case 'ai.import.corrupt': return t('aigen.imp.err.corrupt');
    case 'ai.import.tooComplex': return t('aigen.imp.err.tooComplex');
    case 'ai.import.tooMuchText': return t('aigen.imp.err.tooMuchText', { max: grouped(args.arg ?? args.max ?? 400000) });
    case 'ai.import.readTimeout': return t('aigen.imp.err.readTimeout');
    case 'ai.import.noContent': return t('aigen.imp.err.noContent');
    case 'ai.import.unclear': return t('aigen.imp.err.unclear');
    case 'ai.import.tooMany': return t('aigen.imp.err.tooMany', { max: grouped(args.arg ?? args.max ?? 50) });
    case 'ai.import.tooLongQuestion': return t('aigen.imp.err.tooLongQuestion', { no: args.arg ?? 0, max: grouped(3000) });
    case 'ai.import.callLimit': return t('aigen.imp.err.callLimit');
    case 'ai.import.expired': return t('aigen.imp.card.expired');
    default: return null;
  }
}

/** The catalog key of an AI error code, or null when the catalog has no text for it */
function aiKey(code: string): MsgKey | null {
  if (code === 'auth.guard') return 'ai.err.guard';
  if (!code.startsWith('ai.')) return null;
  const key = `ai.err.${code.slice(3)}`;
  return hasKey('en', key) ? (key as MsgKey) : null;
}

/** Whether an AI error code has a text of its own */
export const knownAiCode = (code: string): boolean => aiKey(code) != null || genCodeText(code) != null;

/**
 * The text of an AI error code (a stream's error event, a failed answer, a test result). `limit`: the number the answer
 * gave for a limit that was reached. An unknown code falls back to the generic internal error of AI.
 */
export function aiCodeText(code: string, limit?: number): string {
  const g = genCodeText(code, limit == null ? {} : { limit });
  if (g != null) return g;
  const key = aiKey(code) ?? 'ai.err.internal';
  const n = limit ?? LIMIT_OF[code];
  return t(key, n == null ? undefined : { n: n.toLocaleString(localeTag()) });
}

/** The reason text of a failed job (`error` code and `errorArg` of the Job object, IM-9) */
export function jobReasonText(job: { error: string | null; errorArg: number | null }): string {
  if (!job.error) return t('ai.err.internal');
  return genCodeText(job.error, { arg: job.errorArg }) ?? aiCodeText(job.error);
}

export class LocalError extends Error {
  readonly key: MsgKey;
  readonly params: Params | undefined;

  constructor(key: MsgKey, params?: Params) {
    super(key);
    this.name = 'LocalError';
    this.key = key;
    this.params = params;
  }
}

export function errorText(e: unknown, fallback: MsgKey = 'err.generic'): string {
  if (e instanceof LocalError) return t(e.key, e.params);
  if (e instanceof ApiError) {
    // before the sign-in lock's 429 branch below: the cap refusal is a 429 with retryAfter too
    if (e.code === 'ai.callLimit' && e.quota) return callLimitText(e.quota, isAdminForTexts(), nowSeconds());
    if (e.code) {
      const g = genCodeText(e.code, genArgsOf(e.body));
      if (g != null) return g;
    }
    if (e.code && aiKey(e.code)) return aiCodeText(e.code, e.limit);
    switch (e.kind) {
      case 'http':
        // Too many failed sign-ins / password checks: say how long the lock still runs
        if (e.status === 429 && e.retryAfter != null) return t('auth.err.tooMany', { wait: waitText(e.retryAfter) });
        return localizeServerMessage(e.serverMessage, uiLang());
      case 'tooMany':
        // The proxy's rate limit, still refusing after the automatic retries: not a lost connection or sign-in
        return t('err.tooMany');
      case 'network':
        return t('err.network');
      case 'unexpected':
        return t('err.unexpected');
      case 'status':
        return t('err.status', { status: e.status });
    }
  }
  return t(fallback);
}

/**
 * The text of a failed study ask (the AI popover / summary window): the code's own text (with the limit it came with); Nginx's
 * own 429 has no code and means "too many requests", not a lost sign-in; no HTTP answer = the network; else the status.
 */
export function studyErrorText(e: { status: number; code: string | null; limit?: number; quota?: AiCallBlock | undefined }): string {
  if (e.code === 'ai.callLimit' && e.quota) return callLimitText(e.quota, isAdminForTexts(), nowSeconds());
  if (e.code && knownAiCode(e.code)) return aiCodeText(e.code, e.limit);
  if (e.status === 429) return t('err.tooMany');
  if (e.status === 0) return t('err.network');
  return e.status > 0 ? t('err.status', { status: e.status }) : t('err.generic');
}

/** A backend text that is not an error (for example the reason a row of an uploaded workbook was skipped) */
export function serverText(msg: string): string {
  return localizeServerMessage(msg, uiLang());
}
