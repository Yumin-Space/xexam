// AI settings (administrators): the rules of the AI settings page as pure functions the page and the tests share. The AI
// service enforces everything; this only decides what the page shows and sends. v1.3:
//   · connections: the form of one connection (name, provider, address, keys), its save body (key fields: absent = keep,
//     null = remove, string = set), the host a key reuse must be confirmed for, the result line of a save or a test;
//   · the status cards, the seven model slots (the model each uses, the "unusable" warning: nothing is switched on its own),
//     the merged model list grouped by connection;
//   · the safety limits form and the retention days; why ai-service is not connected (the status panel);
//   · which listed models are probably not chat models; the category tabs; the per-user request counts.
// v1.9: a connection can be switched off (`enabled`), the slot `study` is split into `hint` and `explain`, every model row has its
//   own feature switch, one global pacing setting (`pace*`), and the safety limits are shown in seven sub-tabs.
import type {
  AiConnection, AiConnectionSave, AiFeatureKey, AiFormat, AiGatewayAuth, AiKeySlot, AiModelList, AiPoolModel, AiProvider, AiService,
  AiSettings, AiSlot, AiTestAnswer, ModelRef,
} from '../types/api.ts';

export const PROVIDERS: readonly AiProvider[] = ['openai', 'anthropic', 'gateway'];
export const FORMATS: readonly AiFormat[] = ['openai', 'anthropic', 'responses'];
export const GATEWAY_AUTHS: readonly AiGatewayAuth[] = ['bearer', 'x-api-key', 'cf-aig-authorization'];
export const KEY_SLOTS: readonly AiKeySlot[] = ['apiKey', 'gatewayKey'];
/** The example address of a Cloudflare AI Gateway (a placeholder only; never a real address) */
export const GATEWAY_PLACEHOLDER = 'https://ai-gateway.example.com/openai';

export interface ProviderFields {
  /** The gateway's API format and auth scheme */
  gateway: boolean;
  /** The key fields, in the order shown */
  slots: AiKeySlot[];
  /** Keys that listing the models and the test need */
  required: AiKeySlot[];
}

/**
 * OpenAI-compatible: one key, optional (a local server may need none); Anthropic: one key, required; Cloudflare AI Gateway:
 * the gateway key (required) and, only for gateways that need it, the provider key.
 */
export function fieldsOf(p: AiProvider): ProviderFields {
  switch (p) {
    case 'anthropic':
      return { gateway: false, slots: ['apiKey'], required: ['apiKey'] };
    case 'gateway':
      return { gateway: true, slots: ['gatewayKey', 'apiKey'], required: ['gatewayKey'] };
    default:
      return { gateway: false, slots: ['apiKey'], required: [] };
  }
}

/** Trimmed, without trailing slashes (how the service stores an address) */
export const normalizeUrl = (u: string): string => u.trim().replace(/\/+$/, '');

export function defaultUrl(p: AiProvider, defaults: AiSettings['defaults']): string {
  return p === 'openai' ? defaults.openai : p === 'anthropic' ? defaults.anthropic : '';
}

/** Switching the provider puts in the new default address only when the field is empty or still the old default */
export function urlAfterSwitch(url: string, from: AiProvider, to: AiProvider, defaults: AiSettings['defaults']): string {
  const now = normalizeUrl(url);
  if (!now || now === normalizeUrl(defaultUrl(from, defaults))) return defaultUrl(to, defaults);
  return url;
}

/** The API format a gateway address suggests: ".../anthropic" is Anthropic, anything else OpenAI (Responses is only ever picked by hand) */
export function inferFormat(url: string): AiFormat {
  let path = normalizeUrl(url);
  try {
    path = new URL(path).pathname;
  } catch {
    /* not a full address yet: look at the text */
  }
  const last = path.split('/').filter(Boolean).pop() ?? '';
  return last.toLowerCase() === 'anthropic' ? 'anthropic' : 'openai';
}

/** Scheme, lower-case host and port (the scheme's default filled in) of an http(s) address; null when it is not one */
export function originOf(url: string): string | null {
  let u: URL;
  try {
    u = new URL(normalizeUrl(url));
  } catch {
    return null;
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
  if (!u.hostname) return null;
  const port = u.port || (u.protocol === 'https:' ? '443' : '80');
  return `${u.protocol.slice(0, -1)}://${u.hostname.toLowerCase()}:${port}`;
}

/**
 * A gateway's key and the provider key would go in the same header: the bearer scheme with the OpenAI or Responses format, or x-api-key
 * with the Anthropic format, while a provider key is set (typed or saved). Only cf-aig-authorization leaves both free.
 */
export function headerClash(form: Pick<ConnForm, 'provider' | 'format' | 'gatewayAuth'>, providerKeySet: boolean): boolean {
  if (form.provider !== 'gateway' || !providerKeySet) return false;
  return (form.gatewayAuth === 'bearer' && (form.format === 'openai' || form.format === 'responses')) || (form.gatewayAuth === 'x-api-key' && form.format === 'anthropic');
}

/**
 * Where the keyboard focus goes after the administrator confirmed a delete in the viewer of a user's AI data and the lists
 * were drawn again (the button that had it is gone; a modal dialog must not lose it to the page): the first chat that is
 * left, else the tab of the list that was cleared.
 */
export function focusAfterClear(what: 'chat' | 'chats' | 'memories', chatIds: readonly number[]): string {
  if (what !== 'memories') return chatIds.length ? `vw-c-${chatIds[0]}` : 'vw-tab-chats';
  return 'vw-tab-memories';
}

/** An http:// address sends the key unencrypted, except to this computer or a single-label name (a container nearby) */
export function httpWarning(url: string): boolean {
  let u: URL;
  try {
    u = new URL(normalizeUrl(url));
  } catch {
    return false;
  }
  if (u.protocol !== 'http:') return false;
  const host = u.hostname.toLowerCase();
  if (host === 'localhost' || host === '127.0.0.1' || host === '[::1]') return false;
  return host.includes('.') || host.includes(':');
}

/** Codes that mean "the AI is not set up correctly": a user who is not an administrator sees that instead of the detail */
export const SETUP_CODES: readonly string[] = ['ai.up.auth', 'ai.up.quota', 'ai.up.notFound', 'ai.up.badRequest', 'ai.up.protocol', 'ai.keyProblem'];

/** The code to show to the signed-in user */
export function shownCode(code: string, admin: boolean): string {
  return !admin && SETUP_CODES.includes(code) ? 'ai.setup' : code;
}

/** Provider problems that pass by themselves: a failed test with one of them leaves AI on */
export const TRANSIENT_CODES: readonly string[] = ['ai.up.rateLimit', 'ai.up.overloaded', 'ai.up.server', 'ai.up.timeout', 'ai.up.connect'];

/** The hint under a failed test: check the key, the model and address, or the connection */
export function testHint(code: string | null): 'auth' | 'model' | 'connect' | 'slow' | null {
  if (code === 'ai.up.auth') return 'auth';
  if (code === 'ai.up.notFound' || code === 'ai.up.badRequest') return 'model';
  if (code === 'ai.up.connect') return 'connect';
  if (code === 'ai.up.timeout') return 'slow';
  return null;
}

/** The models offered by the model picker: every model containing the typed text (any case); all of them when nothing
 *  is typed. Gateways can list many models (dozens to hundreds), so the picker shows this in a scrolling list. */
export function filterModels(models: readonly string[], query: string): string[] {
  const q = query.trim().toLowerCase();
  return q ? models.filter((m) => m.toLowerCase().includes(q)) : [...models];
}

// ---------------------------------------------------------------- models that are probably not chat models (round 2)

/** What a listed model probably is when it is not a chat model (a gateway lists embeddings, classifiers, …) */
export type ModelKind = 'rerank' | 'embedding' | 'guard' | 'image' | 'audio' | 'parse' | 'vision';

/** A whole segment of a model id: delimited by the start / end or one of / _ . - @ : */
const seg = (words: string): string => `(?:^|[-/_.@:])(?:${words})(?=$|[-/_.@:])`;

/** The rules in the order they are tried (the first match wins): rerank before embedding, since "bge-reranker-…" is a reranker */
const KIND_RULES: ReadonlyArray<[ModelKind, RegExp]> = [
  ['rerank', /rerank/i],
  ['embedding', new RegExp(`embed|${seg('bge|gte|e5|bce|uae')}`, 'i')],
  ['guard', /guard|moderation|safety|shield|classif/i],
  ['image', new RegExp(`stable-diffusion|sdxl|${seg('flux')}|dall-?e|imagen|text-to-image|image-gen`, 'i')],
  ['audio', new RegExp(`whisper|${seg('tts')}|text-to-speech|speech-to-text|transcri`, 'i')],
  ['parse', new RegExp(`parse|${seg('ocr')}`, 'i')],
];
const VISION = new RegExp(`${seg('resnet|detr|vit|yolo|clip|siglip|dinov2')}|image-classification|object-detection|image-to-text`, 'i');

/**
 * The kind of a model id that is probably not a chat model, or null (probably a chat model). Only a hint: the picker still
 * offers every model, and any model can be saved and tested. A vision model counts as "vision only" unless its id also says
 * "instruct" or "chat" (vision-language chat models such as "…-vision-instruct").
 */
export function modelKind(id: string): ModelKind | null {
  for (const [kind, re] of KIND_RULES) if (re.test(id)) return kind;
  if (VISION.test(id) && !/instruct|chat/i.test(id)) return 'vision';
  return null;
}

/** The picker's order: likely chat models first, then the marked ones; each group keeps the order of the list */
export function orderModels(models: readonly string[]): string[] {
  const chat: string[] = [];
  const other: string[] = [];
  for (const m of models) (modelKind(m) == null ? chat : other).push(m);
  return [...chat, ...other];
}

// ---------------------------------------------------------------- the connection to ai-service (round 2)

/** Why the AI settings show the status panel instead of the settings ('connected': they load) */
export type SvcCause = 'connected' | 'backendDown' | 'linkOff' | 'notRunning' | 'stopped' | 'notConnected' | 'waiting';

/** The answer of the one probe of `GET /api/ai/status` while not connected: 200, 503 `ai.backendDown`, or no ai-service */
export type SvcProbe = 'ok' | 'refused' | 'down';

/** A report is expected within this long after the main service started (ai-service reports every 25 s) */
export const SVC_WAIT_MS = 60_000;

/**
 * The cause the status panel names. `state`: `GET /api/auth/state` failed (the main service), or its `aiService`; `probe`:
 * the probe of ai-service (null: not asked, e.g. the AI link is off); `waitedMs`: since the panel was first shown.
 *   1 the state failed → backendDown; 2 connected → connected; 3 no internal token → linkOff;
 *   4 probe down → stopped (its last report said so) or notRunning; 5 probe refused → notConnected;
 *   6 probe ok → waiting while no report came yet and less than a minute passed, else notConnected.
 */
export function serviceCause(i: { state: 'failed' | { aiService: AiService }; probe: SvcProbe | null; waitedMs: number }): SvcCause {
  if (i.state === 'failed') return 'backendDown';
  const a = i.state.aiService;
  if (a.connected) return 'connected';
  if (!a.link) return 'linkOff';
  if (i.probe === 'down') return a.stopped ? 'stopped' : 'notRunning';
  if (i.probe === 'refused') return 'notConnected';
  if (i.probe === 'ok') return a.lastSeen == null && i.waitedMs < SVC_WAIT_MS ? 'waiting' : 'notConnected';
  // Not probed (yet): name what the main service knows
  return a.stopped ? 'stopped' : 'notConnected';
}

/** The `aiService` of a `GET /api/auth/state` answer, or null when it has none (not an administrator, an older backend) */
export function aiServiceOf(state: unknown): AiService | null {
  if (typeof state !== 'object' || state === null) return null;
  const a = (state as { aiService?: unknown }).aiService;
  if (typeof a !== 'object' || a === null) return null;
  const o = a as Record<string, unknown>;
  if (typeof o.connected !== 'boolean') return null;
  return {
    connected: o.connected,
    stopped: o.stopped === true,
    lastSeen: typeof o.lastSeen === 'number' && Number.isFinite(o.lastSeen) ? o.lastSeen : null,
    version: typeof o.version === 'string' && o.version ? o.version : null,
    enabled: o.enabled === true,
    link: o.link === true,
  };
}

/** The form field a refused Save is about (the key named by the answer, the address, the model, the retention days) */
export type SettingsField = AiKeySlot | 'baseUrl' | 'name' | 'model' | 'retention';

export function fieldOfError(e: { code?: string | undefined; field?: string | undefined }): SettingsField | null {
  if (e.field === 'apiKey' || e.field === 'gatewayKey') return e.field;
  switch (e.code) {
    case 'ai.badUrl':
      return 'baseUrl';
    case 'ai.badName':
    case 'ai.nameTaken':
      return 'name';
    case 'ai.modelMissing':
      return 'model';
    case 'ai.badRetention':
      return 'retention';
    default:
      return null;
  }
}

// ---------------------------------------------------------------- chat retention

export const RETENTION_MIN = 1;
export const RETENTION_MAX = 3650;

/** The typed number of days, or null when it is not a whole number from 1 to 3650 */
export function retentionOf(text: string): number | null {
  const s = text.trim();
  if (!/^\d{1,4}$/.test(s)) return null;
  const n = Number(s);
  return n >= RETENTION_MIN && n <= RETENTION_MAX ? n : null;
}

// ---------------------------------------------------------------- safety limits (v0.8 spec 14.1, v1.0 spec 3.2 / 3.10)

/** The limits of the AI service (v1.3: 6 more than v1.2) */
export type LimitKey =
  | 'callsPerMinute' | 'callsPerHour' | 'callsPerDay' | 'callsAdminsExempt' | 'replyMaxTokens' | 'memoryTokens'
  | 'perUserRunning' | 'globalRunning' | 'chatsPerUser' | 'messagesPerChat' | 'memoriesPerUser'
  | 'genPerDay' | 'genAdminsExempt' | 'genMinSource' | 'genMinWrong' | 'genBankSingle' | 'genBankMulti' | 'genWrongSingle'
  | 'genWrongMulti' | 'genCallTokens' | 'genRunning' | 'importMaxMb' | 'importMaxQuestions'
  | 'genMaxCalls' | 'importMaxCalls' | 'recognizeMaxCalls' | 'genUnlimited' | 'importUnlimited'
  // v1.3: sharing of hints / explanations, the chunks of a big bank, the summary size, the site-wide daily budget
  | 'studyShare' | 'studyShareDays' | 'genMaxChunks' | 'summaryMaxItems' | 'dailyTokenBudget'
  // v1.9: one pacing setting for AI generation and import calls (the provider's own rate limit)
  | 'paceOn' | 'paceRpm' | 'paceTpm';
/** The values of the limits; a key the service did not send falls back to its default where it is read */
export type LimitValues = Partial<Record<LimitKey, number>>;
export type LimitRanges = Partial<Record<LimitKey, [number, number]>>;

/** The per-user caps on AI model calls, shown first, in this order */
export const CAP_KEYS: readonly LimitKey[] = ['callsPerMinute', 'callsPerHour', 'callsPerDay'];
/** The group "AI requests": the three caps and the switch for administrators */
export const CALLS_KEYS: readonly LimitKey[] = [...CAP_KEYS, 'callsAdminsExempt'];
/** The group "Chat" */
export const CHAT_KEYS: readonly LimitKey[] = ['chatsPerUser', 'messagesPerChat', 'memoriesPerUser', 'memoryTokens'];
/** The one answer length of chat, hint, explanation and summary */
export const LENGTH_KEYS: readonly LimitKey[] = ['replyMaxTokens'];
/** The group "Shared hints and explanations" and the site budget */
export const SHARE_KEYS: readonly LimitKey[] = ['studyShare', 'studyShareDays'];
export const SUMMARY_KEYS: readonly LimitKey[] = ['summaryMaxItems'];
/** The group "AI banks": generation and import */
export const GEN_KEYS: readonly LimitKey[] = ['genPerDay', 'genAdminsExempt', 'genCallTokens', 'genMaxCalls', 'genUnlimited', 'genMaxChunks'];
export const IMPORT_KEYS: readonly LimitKey[] = ['importMaxMb', 'importMaxQuestions', 'importMaxCalls', 'recognizeMaxCalls', 'importUnlimited'];
/** The group "Server" */
export const SERVER_KEYS: readonly LimitKey[] = ['perUserRunning', 'globalRunning', 'genRunning'];
/** v1.9: the provider rate limit (shown in the Server sub-tab) */
export const PACE_KEYS: readonly LimitKey[] = ['paceOn', 'paceRpm', 'paceTpm'];
/** The "AI banks" tab keeps the content rules only */
export const CONTENT_KEYS: readonly LimitKey[] = ['genMinSource', 'genMinWrong', 'genBankSingle', 'genBankMulti', 'genWrongSingle', 'genWrongMulti'];
/** What the safety tab shows (its Reset fills these defaults) */
export const AS_KEYS: readonly LimitKey[] = [
  ...CALLS_KEYS, ...CHAT_KEYS, ...LENGTH_KEYS, ...SHARE_KEYS, ...SUMMARY_KEYS, 'dailyTokenBudget', ...GEN_KEYS, ...IMPORT_KEYS, ...SERVER_KEYS,
  ...PACE_KEYS,
];
/** Every key of the limits row */
export const LIMIT_KEYS: readonly LimitKey[] = [...AS_KEYS, ...CONTENT_KEYS];
/** The AI banks tab saves and resets these */
export const BANK_KEYS: readonly LimitKey[] = CONTENT_KEYS;

/** The defaults and ranges of the spec (the service sends its own; these are used only until it answers) */
export const DEFAULT_LIMITS: Record<LimitKey, number> = {
  callsPerMinute: 5, callsPerHour: 20, callsPerDay: 100, callsAdminsExempt: 1, replyMaxTokens: 3000, memoryTokens: 200,
  perUserRunning: 1, globalRunning: 16, chatsPerUser: 1000, messagesPerChat: 400, memoriesPerUser: 30,
  genPerDay: 3, genAdminsExempt: 1, genMinSource: 30, genMinWrong: 10, genBankSingle: 14, genBankMulti: 6, genWrongSingle: 14,
  genWrongMulti: 6, genCallTokens: 20000, genRunning: 2, importMaxMb: 20, importMaxQuestions: 100, genMaxCalls: 15, importMaxCalls: 25, recognizeMaxCalls: 20,
  genUnlimited: 1, importUnlimited: 1,
  studyShare: 1, studyShareDays: 180, genMaxChunks: 8, summaryMaxItems: 40, dailyTokenBudget: 0,
  paceOn: 1, paceRpm: 13, paceTpm: 240000,
};
export const LIMIT_RANGES: Record<LimitKey, [number, number]> = {
  callsPerMinute: [1, 100], callsPerHour: [1, 2000], callsPerDay: [1, 20000], callsAdminsExempt: [0, 1], replyMaxTokens: [1, 250000], memoryTokens: [50, 250000],
  perUserRunning: [1, 4], globalRunning: [1, 64], chatsPerUser: [10, 100000], messagesPerChat: [20, 5000], memoriesPerUser: [1, 1000],
  genPerDay: [0, 100], genAdminsExempt: [0, 1], genMinSource: [5, 2000], genMinWrong: [1, 500], genBankSingle: [0, 100],
  genBankMulti: [0, 100], genWrongSingle: [0, 100], genWrongMulti: [0, 100], genCallTokens: [1000, 250000], genRunning: [1, 8],
  importMaxMb: [1, 20], importMaxQuestions: [1, 300], genMaxCalls: [8, 40], importMaxCalls: [8, 40], recognizeMaxCalls: [8, 40],
  genUnlimited: [0, 1], importUnlimited: [0, 1],
  studyShare: [0, 1], studyShareDays: [7, 3650], genMaxChunks: [1, 50], summaryMaxItems: [10, 70], dailyTokenBudget: [0, 100000000],
  paceOn: [0, 1], paceRpm: [1, 10000], paceTpm: [1000, 100000000],
};

export type LimitsForm = Record<LimitKey, string>;

export function limitsFormOf(l: LimitValues): LimitsForm {
  const f = {} as LimitsForm;
  for (const k of LIMIT_KEYS) f[k] = String(l[k] ?? DEFAULT_LIMITS[k]);
  return f;
}

/** The range of a limit: the service's own when it sent one, else the spec's */
export const rangeOf = (ranges: LimitRanges | undefined, k: LimitKey): [number, number] => ranges?.[k] ?? LIMIT_RANGES[k];

/** What is wrong with the form: a value outside its range, or the caps out of order (hour below minute, day below hour) */
export interface LimitIssue {
  key: LimitKey;
  /** `comp` (v1.1, A0): single + multiple of a mode is not 1 .. 100; `key` is the multi key of the mode that fails */
  kind: 'range' | 'order' | 'comp';
}

const whole = (s: string): number | null => (/^\d{1,9}$/.test(s.trim()) ? Number(s.trim()) : null);

/** The first problem of the form, or null: every field in its range first (in field order), then the order of the caps */
export function limitIssue(form: LimitsForm, ranges?: LimitRanges): LimitIssue | null {
  for (const k of LIMIT_KEYS) {
    const n = whole(form[k]);
    const [min, max] = rangeOf(ranges, k);
    if (n == null || n < min || n > max) return { key: k, kind: 'range' };
  }
  // All numbers are valid here
  if (Number(form.callsPerHour) < Number(form.callsPerMinute)) return { key: 'callsPerHour', kind: 'order' };
  if (Number(form.callsPerDay) < Number(form.callsPerHour)) return { key: 'callsPerDay', kind: 'order' };
  // The composition rule (A0), the bank mode first
  if (!compOk(form.genBankSingle, form.genBankMulti)) return { key: 'genBankMulti', kind: 'comp' };
  if (!compOk(form.genWrongSingle, form.genWrongMulti)) return { key: 'genWrongMulti', kind: 'comp' };
  return null;
}

/** single + multiple of a generation mode: 1 .. 100 in total (each is a valid number by now) */
function compOk(single: string, multi: string): boolean {
  const n = Number(single) + Number(multi);
  return Number.isFinite(n) && n >= 1 && n <= 100;
}

/** The live total of a composition row (`NaN` while a field is not a number) */
export function compTotal(single: string, multi: string): number {
  const a = whole(single);
  const b = whole(multi);
  return a == null || b == null ? Number.NaN : a + b;
}

/** Whether a typed composition is in range (the live total turns red otherwise) */
export const compBad = (single: string, multi: string): boolean => {
  const n = compTotal(single, multi);
  return Number.isNaN(n) || n < 1 || n > 100;
};

/** Estimated output tokens of one generated question (twin of `EST_TOKENS_PER_QUESTION` in ai-service) */
export const EST_TOKENS_PER_QUESTION = 2400;

/** GP-1 `per_call`: questions per model call: `clamp(min(callTokens, ceiling * 2 / 3) / 2400, 1, 5)` */
export function perCall(callTokens: number, tokenCeiling: number | null): number {
  const room = tokenCeiling == null ? callTokens : Math.min(callTokens, Math.floor((tokenCeiling * 2) / 3));
  return Math.min(5, Math.max(1, Math.floor(room / EST_TOKENS_PER_QUESTION)));
}

/** The live line under the length field of the AI banks tab (GS-2): questions per call, or null for a bad number */
export function genCallLive(text: string, ranges?: LimitRanges): { q: number } | null {
  const n = whole(text);
  const [min, max] = rangeOf(ranges, 'genCallTokens');
  if (n == null || n < min || n > max) return null;
  return { q: perCall(n, null) };
}

/** The first field with a problem (range or order), or null */
export function limitProblem(form: LimitsForm, ranges?: LimitRanges): LimitKey | null {
  return limitIssue(form, ranges)?.key ?? null;
}

/** The changed limits (only valid numbers; check limitProblem first) */
export function limitsUpdate(saved: LimitValues, form: LimitsForm): LimitValues {
  const u: LimitValues = {};
  for (const k of LIMIT_KEYS) {
    const s = form[k].trim();
    if (/^\d{1,9}$/.test(s) && Number(s) !== (saved[k] ?? DEFAULT_LIMITS[k])) u[k] = Number(s);
  }
  return u;
}

/** The typed provider limits as entered (no hidden margin) and the even gap between two requests in seconds (60 / rpm) */
export function paceEffective(rpm: number, tpm: number): { rpm: number; tpm: number; sec: number } {
  return { rpm, tpm, sec: Math.round(600 / Math.max(1, rpm)) / 10 };
}

// ---------------------------------------------------------------- the sub-tabs of the safety tab (v1.9)

/** The seven sub-tabs of AI safety, in the order of the row */
export const SAFETY_SUBS = ['caps', 'chat', 'study', 'summary', 'gen', 'import', 'server'] as const;
export type SafetySub = (typeof SAFETY_SUBS)[number];

/** The limits each sub-tab shows (together exactly AS_KEYS, each once) */
export const SUB_KEYS: Record<SafetySub, readonly LimitKey[]> = {
  caps: [...CALLS_KEYS, 'dailyTokenBudget', ...LENGTH_KEYS],
  chat: CHAT_KEYS,
  study: SHARE_KEYS,
  summary: SUMMARY_KEYS,
  gen: GEN_KEYS,
  import: IMPORT_KEYS,
  server: [...SERVER_KEYS, ...PACE_KEYS],
};

/** The sub-tab that shows a limit, or null for a key of no sub-tab (the AI banks tab keeps the content rules) */
export function subOfKey(k: LimitKey): SafetySub | null {
  return SAFETY_SUBS.find((sub) => SUB_KEYS[sub].includes(k)) ?? null;
}

/** The sub-tab a location hash names (`#/ai-settings?tab=safety&sub=server`); a missing or unknown value is null */
export function subOfHash(hash: string): SafetySub | null {
  const q = hash.indexOf('?');
  if (q < 0) return null;
  const v = new URLSearchParams(hash.slice(q + 1)).get('sub');
  return (SAFETY_SUBS as readonly string[]).includes(v ?? '') ? (v as SafetySub) : null;
}

/** The hash of a safety sub-tab */
export const hashOfSafety = (sub: SafetySub): string => `#/ai-settings?tab=safety&sub=${sub}`;

// ---------------------------------------------------------------- the category tabs (FB-6, AS-9)

/** The five categories of AI settings, in the order of the tab row (v1.1: the fifth, `gen`, is "AI 题库" / AI banks) */
export const AI_TABS = ['feature', 'users', 'usage', 'safety', 'gen'] as const;
/* the id 'feature' stays (old links); its label is "Connections and models" */
export type AiTab = (typeof AI_TABS)[number];

/** The tab a location hash names (`#/ai-settings?tab=users`); a missing or unknown value is the first tab */
export function tabOfHash(hash: string): AiTab {
  const q = hash.indexOf('?');
  if (q < 0) return 'feature';
  const v = new URLSearchParams(hash.slice(q + 1)).get('tab');
  return (AI_TABS as readonly string[]).includes(v ?? '') ? (v as AiTab) : 'feature';
}

/** The hash of a tab: always `#/ai-settings?tab=<key>` */
export const hashOfTab = (tab: AiTab): string => `#/ai-settings?tab=${tab}`;

// ---------------------------------------------------------------- per-user AI request counts (C-7, AS-2)

/** The counts of one user as the admin list sends them (`calls` of a row; absent on an older service) */
export interface UserCalls {
  used: { minute: number; hour: number; day: number };
  blocked: { cap: 'minute' | 'hour' | 'day'; limit: number; retryAt: number } | null;
}

/**
 * The tag a blocked user gets: minute / hour cap "Used up · back at <time>", the day cap "Used up today"; null when nothing
 * blocks. The page names the time from `retryAt`.
 */
export function blockTag(c: Pick<UserCalls, 'blocked'> | undefined): { kind: 'timed' | 'today'; retryAt: number } | null {
  const b = c?.blocked;
  if (!b) return null;
  return { kind: b.cap === 'day' ? 'today' : 'timed', retryAt: b.retryAt };
}

/** The figures of a user's line "1 min 2/5 · 1 h 7/20 · today 37/100": "used/limit" for each window */
export function callsFigures(c: UserCalls | undefined, limits: LimitValues | undefined): { minute: string; hour: string; day: string } | null {
  if (!c) return null;
  const cap = (k: 'callsPerMinute' | 'callsPerHour' | 'callsPerDay'): number => limits?.[k] ?? DEFAULT_LIMITS[k];
  return {
    minute: `${c.used.minute}/${cap('callsPerMinute')}`,
    hour: `${c.used.hour}/${cap('callsPerHour')}`,
    day: `${c.used.day}/${cap('callsPerDay')}`,
  };
}

/** The "今天" cell of Usage > By user: `today / callsPerDay` (an older service sends no `today`: a dash) */
export function todayCell(today: number | undefined, callsPerDay: number | undefined): string {
  return today == null ? '–' : `${today} / ${callsPerDay ?? DEFAULT_LIMITS.callsPerDay}`;
}

// ---------------------------------------------------------------- summaries once saved and tested (AS-4)

/** The host (with the port when it is not the default) of an address, or the trimmed text when it is not an address */
export function hostOf(url: string): string {
  try {
    return new URL(normalizeUrl(url)).host;
  } catch {
    return normalizeUrl(url);
  }
}

/** The conversation figures shown with the usage: the sums over the listed users */
export function convTotals(rows: ReadonlyArray<{ chats: number; messages: number; memories: number }>): { chats: number; messages: number; memories: number } {
  const t = { chats: 0, messages: 0, memories: 0 };
  for (const r of rows) {
    t.chats += r.chats;
    t.messages += r.messages;
    t.memories += r.memories;
  }
  return t;
}

/** The width of a usage bar in percent of the largest value (at least a sliver for anything above 0) */
export function barPercent(v: number, max: number): number {
  if (!(v > 0) || !(max > 0)) return 0;
  return Math.max(2, Math.round((v / max) * 100));
}

/** L-55: a job card's time with its date, local: "2026-11-13 18:04" */
export function jobTime(sec: number): string {
  const d = new Date(sec * 1000);
  const p = (n: number): string => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

// ---------------------------------------------------------------- the form of one connection (v1.3)

/** The form of one connection: what is typed or chosen, before it is saved */
export interface ConnForm {
  name: string;
  provider: AiProvider;
  baseUrl: string;
  format: AiFormat;
  gatewayAuth: AiGatewayAuth;
  /** Typed keys (empty = keep the saved one; never prefilled) */
  keys: Record<AiKeySlot, string>;
  /** "Remove this key" ticked */
  remove: Record<AiKeySlot, boolean>;
  /** New connection only: the id of the connection whose saved keys are used (null = none) */
  reuseFrom: number | null;
}

/** The form of a saved connection: nothing typed, nothing ticked */
export function formOf(c: Pick<AiConnection, 'name' | 'provider' | 'baseUrl' | 'format' | 'gatewayAuth'>): ConnForm {
  return {
    name: c.name, provider: c.provider, baseUrl: c.baseUrl, format: c.format, gatewayAuth: c.gatewayAuth,
    keys: { apiKey: '', gatewayKey: '' }, remove: { apiKey: false, gatewayKey: false }, reuseFrom: null,
  };
}

/** The name a new connection starts with: the host of its address without the port (at most 40 characters) */
export function suggestName(url: string): string {
  try {
    return new URL(normalizeUrl(url)).hostname.slice(0, 40);
  } catch {
    return '';
  }
}

/** The form of a new connection: the OpenAI-compatible provider at its default address, the name follows the host */
export function blankForm(defaults: AiSettings['defaults']): ConnForm {
  const baseUrl = defaults.openai;
  return {
    name: suggestName(baseUrl), provider: 'openai', baseUrl, format: 'openai', gatewayAuth: 'bearer',
    keys: { apiKey: '', gatewayKey: '' }, remove: { apiKey: false, gatewayKey: false }, reuseFrom: null,
  };
}

export const NAME_MAX = 40;

/** The name is wrong the way the server checks it: not 1 to 40 characters, or a control character in it */
export function nameProblem(name: string): boolean {
  const n = name.trim();
  // eslint-disable-next-line no-control-regex
  return n.length < 1 || [...n].length > NAME_MAX || /[\u0000-\u001f\u007f]/.test(n);
}

/** What is wrong with the form before it is sent: only the name (the server checks the rest) */
export const formProblem = (form: Pick<ConnForm, 'name'>): 'name' | null => (nameProblem(form.name) ? 'name' : null);

/** The host a key reuse is confirmed for: `host[:port]` of the address, the port only when it is not the scheme's default */
export function confirmHostOf(url: string): string | null {
  let u: URL;
  try {
    u = new URL(normalizeUrl(url));
  } catch {
    return null;
  }
  if (u.protocol !== 'http:' && u.protocol !== 'https:') return null;
  return u.host.toLowerCase();
}

/** Whether saving moves the saved keys to another provider or origin (then the server asks for a confirmation) */
export function movesKeys(saved: Pick<AiConnection, 'provider' | 'baseUrl'>, form: Pick<ConnForm, 'provider' | 'baseUrl'>): boolean {
  if (saved.provider !== form.provider) return true;
  const a = originOf(saved.baseUrl);
  const b = originOf(form.baseUrl);
  if (a == null && b == null) return normalizeUrl(saved.baseUrl) !== normalizeUrl(form.baseUrl);
  return a !== b;
}

/** Whether a key slot holds a key after the form is saved (a saved key stays unless it is replaced or removed) */
export function keyAfter(saved: Pick<AiConnection, 'keys'> | null, form: ConnForm, slot: AiKeySlot): boolean {
  if (form.keys[slot].trim()) return true;
  if (!fieldsOf(form.provider).slots.includes(slot)) return false;
  if (form.remove[slot]) return false;
  return saved ? saved.keys[slot].set : false;
}

/** Whether the form differs from the saved connection (a new connection always does) */
export function isDirty(saved: AiConnection | null, form: ConnForm): boolean {
  if (!saved) return true;
  if (form.name.trim() !== saved.name || form.provider !== saved.provider || normalizeUrl(form.baseUrl) !== normalizeUrl(saved.baseUrl)) return true;
  if (form.provider === 'gateway' && (form.format !== saved.format || form.gatewayAuth !== saved.gatewayAuth)) return true;
  return KEY_SLOTS.some((k) => form.keys[k].trim() !== '' || form.remove[k]);
}

/**
 * The body of a save: a new connection (`saved` null) sends what is typed; a saved one sends its id and `rev`. The key fields
 * are read as the server reads them: absent = keep, a string = set, `null` = remove (also a slot the provider does not use).
 */
export function connPayload(saved: AiConnection | null, form: ConnForm, confirmHost?: string): AiConnectionSave {
  const body: AiConnectionSave = { name: form.name.trim(), provider: form.provider, baseUrl: normalizeUrl(form.baseUrl) };
  if (form.provider === 'gateway') {
    body.format = form.format;
    body.gatewayAuth = form.gatewayAuth;
  }
  if (saved) {
    body.id = saved.id;
    body.rev = saved.rev;
  }
  const used = fieldsOf(form.provider).slots;
  for (const slot of KEY_SLOTS) {
    const typed = form.keys[slot].trim();
    if (!used.includes(slot)) {
      if (saved?.keys[slot].set) body[slot] = null;
      continue;
    }
    if (typed) body[slot] = typed;
    else if (saved?.keys[slot].set && form.remove[slot]) body[slot] = null;
  }
  if (!saved && form.reuseFrom != null) body.reuseKeysFrom = form.reuseFrom;
  if (confirmHost) body.confirmHost = confirmHost;
  return body;
}

/** The save replaces a saved key: a rotation (the new key is tested before it replaces the old one) */
export function isRotation(saved: AiConnection | null, form: ConnForm): boolean {
  if (!saved) return false;
  return fieldsOf(form.provider).slots.some((k) => form.keys[k].trim() !== '' && saved.keys[k].set);
}

export type ConnResult =
  | { ok: true; key: 'ai.set.conn.ok' | 'ai.set.conn.okNoList' | 'ai.set.conn.rotated'; n: number }
  | { ok: false; key: 'ai.set.conn.fail' | 'ai.set.conn.rotateFail'; code: string };

/** The result line of a save or a retest: what was found, or why nothing was saved */
export function connResult(a: Pick<AiTestAnswer, 'ok' | 'listed' | 'modelCount' | 'code'>, rotation: boolean): ConnResult {
  if (a.ok) {
    if (rotation) return { ok: true, key: 'ai.set.conn.rotated', n: a.modelCount ?? 0 };
    return a.listed === false ? { ok: true, key: 'ai.set.conn.okNoList', n: 0 } : { ok: true, key: 'ai.set.conn.ok', n: a.modelCount ?? 0 };
  }
  return { ok: false, key: rotation ? 'ai.set.conn.rotateFail' : 'ai.set.conn.fail', code: a.code ?? 'ai.internal' };
}

/** The key state of each key slot the provider uses (for the card) */
export function keyLine(c: Pick<AiConnection, 'provider' | 'keys'>): Array<{ slot: AiKeySlot; state: 'none' | 'unreadable' | 'saved'; hint: string }> {
  return fieldsOf(c.provider).slots.map((slot) => {
    const k = c.keys[slot];
    return { slot, state: !k.set ? 'none' : k.unreadable ? 'unreadable' : 'saved', hint: k.hint };
  });
}

// ---------------------------------------------------------------- status cards, slots and the merged model list (v1.3)

export const SLOTS: readonly AiSlot[] = ['default', 'chat', 'hint', 'explain', 'summary', 'generate', 'import'];

/** A connection counts as on unless the service says `enabled: false` (an older service sends nothing) */
export const connOn = (c: Pick<AiConnection, 'enabled'>): boolean => c.enabled !== false;

/** A connection that is on and broken or whose key cannot be read (a connection that is off is never a problem) */
export const connBad = (c: AiConnection): boolean => connOn(c) && (c.status === 'broken' || c.keys.apiKey.unreadable || c.keys.gatewayKey.unreadable);

/** The connections split into the two groups of the list, each in the server's order */
export function connGroups(s: Pick<AiSettings, 'connections'>): { on: AiConnection[]; off: AiConnection[] } {
  return { on: s.connections.filter(connOn), off: s.connections.filter((c) => !connOn(c)) };
}

/** The global feature switch a model row carries (the default row has none) */
export const slotFeature = (slot: AiSlot): AiFeatureKey | null => (slot === 'default' ? null : slot);

/** Whether a slot's feature is switched on (the default slot has no switch: always on) */
export function slotOn(s: Pick<AiSettings, 'featureSwitches'>, slot: AiSlot): boolean {
  const f = slotFeature(slot);
  return f == null || s.featureSwitches[f] !== false;
}

export interface StatusCard {
  id: 'main' | 'conn' | 'model' | 'features';
  tone: 'ok' | 'warn' | 'off' | 'error';
  /** Message key and its parameters (the page adds the names of the features that are off) */
  key: string;
  params: Record<string, string | number>;
  offFeatures: AiFeatureKey[];
}

const poolById = (s: Pick<AiSettings, 'pool'>, id: number | null): AiPoolModel | null => (id == null ? null : s.pool.find((m) => m.id === id) ?? null);

/** The pool model a slot holds itself (null: empty, it follows the default) */
export const slotModel = (s: Pick<AiSettings, 'pool' | 'slots'>, slot: AiSlot): AiPoolModel | null => poolById(s, s.slots[slot].modelId);
/** The pool model a slot really uses now (its own, else the default; null: none is usable) */
export const slotUsing = (s: Pick<AiSettings, 'pool' | 'slots'>, slot: AiSlot): AiPoolModel | null => poolById(s, s.slots[slot].using?.modelId ?? null);

/** The default model holds a model that cannot be used: a WARNING only, nothing is switched to another model */
export const defaultUnusable = (s: Pick<AiSettings, 'pool' | 'slots'>): boolean => s.slots.default.modelId != null && s.slots.default.using == null;

const card = (id: StatusCard['id'], tone: StatusCard['tone'], key: string, params: StatusCard['params'] = {}, offFeatures: AiFeatureKey[] = []): StatusCard =>
  ({ id, tone, key, params, offFeatures });

/** The four status cards: AI+ switch, connections, default model, features */
export function statusCards(s: AiSettings): StatusCard[] {
  const main = s.state === 'on' ? card('main', 'ok', 'ai.set.card.main.on')
    : s.state === 'paused'
      ? (s.pauseReason === 'keyProblem' ? card('main', 'error', 'ai.set.card.main.pausedKey') : card('main', 'warn', 'ai.set.card.main.pausedNoRoute'))
      : s.state === 'setup' ? card('main', 'off', 'ai.set.card.main.setup') : card('main', 'off', 'ai.set.card.main.off');
  // Only connections that are on count; one that is off is not a problem
  const onConns = s.connections.filter(connOn);
  const bad = onConns.filter((c) => c.status === 'broken' || c.keys.apiKey.unreadable || c.keys.gatewayKey.unreadable).length;
  const conn = !s.connections.length ? card('conn', 'off', 'ai.set.card.conn.none')
    : bad ? card('conn', 'warn', 'ai.set.card.conn.someBad', { n: onConns.length, bad })
      : onConns.length < s.connections.length ? card('conn', 'ok', 'ai.set.card.conn.someOff', { n: s.connections.length, on: onConns.length })
        : card('conn', 'ok', 'ai.set.card.conn.allOk', { n: s.connections.length });
  const def = slotModel(s, 'default');
  const model = !def ? card('model', 'off', 'ai.set.card.model.none')
    : !connOnById(s, def.connectionId) ? card('model', 'warn', 'ai.set.card.model.off', { conn: def.connectionName, model: def.model })
    : defaultUnusable(s) ? card('model', 'warn', 'ai.set.card.model.bad', { conn: def.connectionName, model: def.model })
      : card('model', 'ok', 'ai.set.card.model.value', { conn: def.connectionName, model: def.model });
  const off = (Object.keys(s.featureSwitches) as AiFeatureKey[]).filter((k) => !s.featureSwitches[k]);
  const features = off.length ? card('features', 'warn', 'ai.set.card.features.someOff', {}, off) : card('features', 'ok', 'ai.set.card.features.allOn');
  return [main, conn, model, features];
}

/** The line under a slot row: its own model is unusable and the default stands in, or nothing can be used at all */
export type SlotNote = { kind: 'connOff'; conn: string } | { kind: 'fallback'; conn: string } | { kind: 'noRoute' } | null;

/** Whether the connection with this id is on (an unknown id counts as on: the pool lists only existing connections) */
export const connOnById = (s: Pick<AiSettings, 'connections'>, id: number): boolean => {
  const c = s.connections.find((x) => x.id === id);
  return c == null || connOn(c);
};

export function slotNote(s: AiSettings, slot: AiSlot): SlotNote {
  const own = slotModel(s, slot);
  const st = s.slots[slot];
  // v1.9: the model the row really calls sits on a connection that is off (its own, or the default it follows): the call fails
  if (own && !connOnById(s, own.connectionId)) return { kind: 'connOff', conn: own.connectionName };
  if (!own && slot !== 'default') {
    const def = slotModel(s, 'default');
    if (def && !connOnById(s, def.connectionId)) return { kind: 'connOff', conn: def.connectionName };
  }
  if (st.using == null) {
    // The default slot with nothing chosen yet is "not set up", not a problem
    if (slot === 'default' && !own) return null;
    return s.state === 'setup' ? null : { kind: 'noRoute' };
  }
  if (own && st.using.fallback === 'default') return { kind: 'fallback', conn: own.connectionName };
  return null;
}

/** The key of a model reference (the value of a select or an option) and back */
export const refKey = (r: ModelRef): string => `${r.connectionId}|${r.model}`;
export function parseRefKey(k: string): ModelRef | null {
  const i = k.indexOf('|');
  const id = Number(k.slice(0, i));
  return i > 0 && Number.isInteger(id) && id > 0 && k.length > i + 1 ? { connectionId: id, model: k.slice(i + 1) } : null;
}
export const refOf = (m: Pick<AiPoolModel, 'connectionId' | 'model'>): ModelRef => ({ connectionId: m.connectionId, model: m.model });

export interface ModelGroup {
  connectionId: number;
  name: string;
  status: 'ok' | 'broken';
  models: Array<{ id: string; context: number | null; inPool: boolean }>;
}

/**
 * The merged model list: one group per connection (in the connections' order), each with its listed models (likely chat
 * models first) and the pool models the listing does not show. `query` filters by substring; a group without a match is left out.
 */
export function modelGroups(list: AiModelList | null, s: Pick<AiSettings, 'pool' | 'connections'>, query: string): ModelGroup[] {
  const out: ModelGroup[] = [];
  for (const c of s.connections) {
    if (!connOn(c)) continue;
    const listed = list?.connections.find((x) => x.id === c.id);
    const byId = new Map<string, { id: string; context: number | null; inPool: boolean }>();
    for (const m of listed?.models ?? []) byId.set(m.id, { id: m.id, context: m.context, inPool: m.poolId != null && m.status === 'ok' });
    for (const m of s.pool) {
      if (m.connectionId !== c.id) continue;
      const have = byId.get(m.model);
      if (have) have.inPool = have.inPool || m.status === 'ok';
      else byId.set(m.model, { id: m.model, context: m.context, inPool: m.status === 'ok' });
    }
    const ids = filterModels(orderModels([...byId.keys()]), query);
    if (!ids.length && query.trim()) continue;
    out.push({ connectionId: c.id, name: c.name, status: c.status, models: ids.flatMap((id) => byId.get(id) ?? []) });
  }
  return out;
}

/** The model preselected for the first AI+ switch-on: the first likely chat model of the first working connection that is on */
export function firstChoice(list: AiModelList | null, s: Pick<AiSettings, 'pool' | 'connections'>): ModelRef | null {
  for (const g of modelGroups(list, s, '')) {
    if (g.status !== 'ok' || !g.models.length) continue;
    const m = g.models.find((x) => !modelKind(x.id)) ?? g.models[0];
    if (m) return { connectionId: g.connectionId, model: m.id };
  }
  return null;
}

/** The pool models a deleted connection's default can move to: tested models of the other working connections that are on */
export function newDefaultChoices(s: Pick<AiSettings, 'pool' | 'connections'>, deleting: number): AiPoolModel[] {
  const ok = new Set(s.connections.filter((c) => c.id !== deleting && c.status === 'ok' && connOn(c)).map((c) => c.id));
  return s.pool.filter((m) => ok.has(m.connectionId) && m.status === 'ok');
}

/** Connections whose saved keys a new connection may reuse (a key is saved and can be read) */
export const reusableFrom = (s: Pick<AiSettings, 'connections'>): AiConnection[] =>
  s.connections.filter((c) => KEY_SLOTS.some((k) => c.keys[k].set && !c.keys[k].unreadable));

/** One usage row by connection or by model: the label (a dash before v1.3, when rows had no connection) */
export const connLabel = (name: string | null | undefined, dash: string): string => (name && name.trim() ? name : dash);
