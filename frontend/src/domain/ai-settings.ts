// AI settings (v0.8, administrators): the rules of the settings form as pure functions the page and the tests share. They
// mirror what the AI service checks (it enforces everything; this only decides what the form shows and sends):
//   · which fields each provider has, its default address, and when switching the provider replaces the address;
//   · the API format a gateway address suggests (its last path segment);
//   · key re-entry: when the provider or the address's origin changes, every saved key must be typed again (an empty field
//     then removes it), so a saved key is never sent to a new place;
//   · the header clash of a gateway (its key and the provider key would use the same header), against the typed AND the
//     saved keys; the http:// warning; whether the form differs from the saved settings; whether AI can be enabled;
//   · which error codes a user who is not an administrator sees as "AI is not set up correctly";
//   · the safety limits form (v0.8 spec 14.1) and the retention days;
//   · round 2: why ai-service is not connected (the status panel), and which listed models are probably not chat models.
import type {
  AiFormat, AiGatewayAuth, AiKeySlot, AiProvider, AiService, AiSettings, AiSettingsUpdate, AiVerify,
} from '../types/api.ts';

export const PROVIDERS: readonly AiProvider[] = ['openai', 'anthropic', 'gateway'];
export const FORMATS: readonly AiFormat[] = ['openai', 'anthropic'];
export const GATEWAY_AUTHS: readonly AiGatewayAuth[] = ['bearer', 'x-api-key', 'cf-aig-authorization'];
export const KEY_SLOTS: readonly AiKeySlot[] = ['apiKey', 'gatewayKey'];
/** The example address of a Cloudflare AI Gateway (a placeholder only; never a real address) */
export const GATEWAY_PLACEHOLDER = 'https://ai-gateway.example.com/openai';

/** The connection part of the form: what is typed or chosen, before it is saved */
export interface ConnForm {
  provider: AiProvider;
  baseUrl: string;
  format: AiFormat;
  gatewayAuth: AiGatewayAuth;
  /** Typed keys (empty = keep the saved one; never prefilled) */
  keys: Record<AiKeySlot, string>;
  /** "Remove this key" ticked */
  remove: Record<AiKeySlot, boolean>;
}

/** The form for saved settings: nothing typed, nothing ticked */
export function formOf(s: Pick<AiSettings, 'provider' | 'baseUrl' | 'format' | 'gatewayAuth'>): ConnForm {
  return {
    provider: s.provider, baseUrl: s.baseUrl, format: s.format, gatewayAuth: s.gatewayAuth,
    keys: { apiKey: '', gatewayKey: '' }, remove: { apiKey: false, gatewayKey: false },
  };
}

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

/** The API format a gateway address suggests: ".../anthropic" is Anthropic, anything else OpenAI */
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

/** Whether saving the form needs the saved keys typed again: the provider or the address's origin changes */
export function needsReentry(saved: Pick<AiSettings, 'provider' | 'baseUrl'>, form: Pick<ConnForm, 'provider' | 'baseUrl'>): boolean {
  if (saved.provider !== form.provider) return true;
  const a = originOf(saved.baseUrl);
  const b = originOf(form.baseUrl);
  if (a == null && b == null) return normalizeUrl(saved.baseUrl) !== normalizeUrl(form.baseUrl);
  return a !== b;
}

/** Whether a key slot holds a key after the form is saved */
export function keyAfter(saved: Pick<AiSettings, 'provider' | 'baseUrl' | 'keys'>, form: ConnForm, slot: AiKeySlot): boolean {
  if (form.keys[slot].trim()) return true;
  if (!fieldsOf(form.provider).slots.includes(slot)) return false;
  if (form.remove[slot]) return false;
  if (needsReentry(saved, form)) return false;
  return saved.keys[slot].set;
}

/**
 * A gateway's key and the provider key would go in the same header: the bearer scheme with the OpenAI format, or x-api-key
 * with the Anthropic format, while a provider key is set (typed or saved). Only cf-aig-authorization leaves both free.
 */
export function headerClash(form: Pick<ConnForm, 'provider' | 'format' | 'gatewayAuth'>, providerKeySet: boolean): boolean {
  if (form.provider !== 'gateway' || !providerKeySet) return false;
  return (form.gatewayAuth === 'bearer' && form.format === 'openai') || (form.gatewayAuth === 'x-api-key' && form.format === 'anthropic');
}

/** Whether the connection form differs from the saved settings (then listing the models and the test wait for Save) */
export function isDirty(saved: Pick<AiSettings, 'provider' | 'baseUrl' | 'format' | 'gatewayAuth'>, form: ConnForm): boolean {
  if (form.provider !== saved.provider || normalizeUrl(form.baseUrl) !== normalizeUrl(saved.baseUrl)) return true;
  if (form.provider === 'gateway' && (form.format !== saved.format || form.gatewayAuth !== saved.gatewayAuth)) return true;
  return KEY_SLOTS.some((k) => form.keys[k].trim() !== '' || form.remove[k]);
}

/**
 * The body of a Save of the connection: only what changed. A typed key replaces the saved one; "Remove this key", a key the
 * new provider does not use, and (key re-entry) an empty field of a saved key after a provider or origin change remove it.
 */
export function connUpdate(saved: Pick<AiSettings, 'provider' | 'baseUrl' | 'format' | 'gatewayAuth' | 'keys'>, form: ConnForm): AiSettingsUpdate {
  const u: AiSettingsUpdate = {};
  if (form.provider !== saved.provider) u.provider = form.provider;
  if (normalizeUrl(form.baseUrl) !== normalizeUrl(saved.baseUrl)) u.baseUrl = normalizeUrl(form.baseUrl);
  if (form.provider === 'gateway') {
    if (form.format !== saved.format || u.provider) u.format = form.format;
    if (form.gatewayAuth !== saved.gatewayAuth || u.provider) u.gatewayAuth = form.gatewayAuth;
  }
  const reentry = needsReentry(saved, form);
  const used = fieldsOf(form.provider).slots;
  for (const slot of KEY_SLOTS) {
    const typed = form.keys[slot].trim();
    const set = saved.keys[slot].set;
    if (!used.includes(slot)) {
      if (set) u[slot] = null;
      continue;
    }
    if (typed) u[slot] = typed;
    else if (set && (form.remove[slot] || reentry)) u[slot] = null;
  }
  return u;
}

/** Whether a Save switches AI off (its settings then need a new test): it changes what the test checked */
export function changesFingerprint(u: AiSettingsUpdate, providerAfter: AiProvider): boolean {
  if (u.provider !== undefined || u.baseUrl !== undefined || u.format !== undefined || u.gatewayAuth !== undefined) return true;
  if (u.model !== undefined || u.apiKey !== undefined) return true;
  return u.gatewayKey !== undefined && providerAfter === 'gateway';
}

/** The toast after a save: a save that switched the AI+ feature off says so (and what to do next) */
export const savedToastKey = (switchedOff: boolean): 'ai.set.savedOff' | 'ai.set.saved' => (switchedOff ? 'ai.set.savedOff' : 'ai.set.saved');

/**
 * The result line under the Test button. `on`: whether the AI+ feature is on NOW (the settings the page holds), not what it
 * was when the test ran: after the administrator turns the feature on or off, the line follows (it never keeps telling
 * "you can turn it on" next to a switch that is on). `none`: nothing to show.
 */
export type TestLine =
  | { kind: 'none' }
  | { kind: 'ok'; key: 'ai.set.testOk' | 'ai.set.testOkOn'; ms: number }
  | { kind: 'fail'; code: string | null; hint: ReturnType<typeof testHint>; keepOn: boolean };

export function testLineOf(r: Pick<AiVerify, 'ok' | 'code' | 'latencyMs'> | null, on: boolean): TestLine {
  if (!r) return { kind: 'none' };
  if (r.ok) return { kind: 'ok', key: on ? 'ai.set.testOkOn' : 'ai.set.testOk', ms: r.latencyMs };
  // "stays on" is only true while it is on
  return { kind: 'fail', code: r.code, hint: testHint(r.code), keepOn: on };
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

/** The keys that listing the models and the test need are saved and readable */
export function requiredKeysSaved(s: Pick<AiSettings, 'provider' | 'keys'>): boolean {
  return fieldsOf(s.provider).required.every((k) => s.keys[k].set && !s.keys[k].unreadable);
}

/** An administrator has saved the connection (a saved key counts; so does a save that records who did it, unless the
 *  connection is still the factory one). The untouched defaults of a fresh instance are not a saved connection, and neither
 *  are they after a save of the chat retention alone (that save records who did it too): the OpenAI-compatible provider at
 *  its default address without a key is never listed or tested on its own. */
export function connectionSaved(s: Pick<AiSettings, 'keys' | 'updatedBy' | 'provider' | 'baseUrl' | 'defaults'>): boolean {
  if (KEY_SLOTS.some((k) => s.keys[k].set)) return true;
  const factory = s.provider === 'openai' && normalizeUrl(s.baseUrl) === normalizeUrl(s.defaults.openai);
  return s.updatedBy.trim() !== '' && !factory;
}

/** AI can be switched on: the saved settings passed a test and the keys can be read */
export const canEnable = (s: Pick<AiSettings, 'verified' | 'keyProblem'>): boolean => s.verified && !s.keyProblem;

export type StateChip = 'on' | 'off' | 'unverified';

/** The status line of AI settings (owner's request): connection, model and test each with its own chip, then AI on / off.
 *  `lastTestFailed`: the latest test on this page failed (the server keeps only whether the saved settings passed). */
export interface StatusChips {
  conn: 'ok' | 'none';
  model: 'ok' | 'none';
  test: 'ok' | 'fail' | 'none';
  ai: 'on' | 'off';
}

export function statusChips(
  s: Pick<AiSettings, 'provider' | 'keys' | 'baseUrl' | 'model' | 'verified' | 'enabled' | 'updatedBy' | 'defaults'>, lastTestFailed: boolean,
): StatusChips {
  return {
    conn: s.baseUrl.trim() !== '' && connectionSaved(s) && requiredKeysSaved(s) ? 'ok' : 'none',
    model: s.model.trim() !== '' ? 'ok' : 'none',
    test: s.verified ? 'ok' : lastTestFailed ? 'fail' : 'none',
    ai: s.enabled ? 'on' : 'off',
  };
}

/** On; saved but not tested (something is saved and the test has not passed); else off */
export function stateChip(s: Pick<AiSettings, 'enabled' | 'verified' | 'model' | 'keys'>): StateChip {
  if (s.enabled) return 'on';
  const saved = s.model !== '' || KEY_SLOTS.some((k) => s.keys[k].set);
  return !s.verified && saved ? 'unverified' : 'off';
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
export type SettingsField = AiKeySlot | 'baseUrl' | 'model' | 'retention';

export function fieldOfError(e: { code?: string | undefined; field?: string | undefined }): SettingsField | null {
  if (e.field === 'apiKey' || e.field === 'gatewayKey') return e.field;
  switch (e.code) {
    case 'ai.badUrl':
      return 'baseUrl';
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

/** The ten limits of the AI service (v1.0): the three per-user caps, the two lengths, the five that protect the server */
export type LimitKey =
  | 'callsPerMinute' | 'callsPerHour' | 'callsPerDay' | 'replyMaxChars' | 'memoryChars'
  | 'perUserRunning' | 'globalRunning' | 'chatsPerUser' | 'messagesPerChat' | 'memoriesPerUser';
/** The values of the limits; a key the service did not send falls back to its default where it is read */
export type LimitValues = Partial<Record<LimitKey, number>>;
export type LimitRanges = Partial<Record<LimitKey, [number, number]>>;

/** v1.0: the per-user caps on AI model calls (spec 3.7), shown first, in this order */
export const CAP_KEYS: readonly LimitKey[] = ['callsPerMinute', 'callsPerHour', 'callsPerDay'];
/** v1.0 (FB-5 revised): the length of one answer and of one memory, in characters, second sub-group */
export const LENGTH_KEYS: readonly LimitKey[] = ['replyMaxChars', 'memoryChars'];
/** The limits that protect the server (the five of v0.8 that are left) */
export const SERVER_KEYS: readonly LimitKey[] = ['perUserRunning', 'globalRunning', 'chatsPerUser', 'messagesPerChat', 'memoriesPerUser'];
export const LIMIT_KEYS: readonly LimitKey[] = [...CAP_KEYS, ...LENGTH_KEYS, ...SERVER_KEYS];

/** The defaults and ranges of the spec (the service sends its own; these are used only until it answers) */
export const DEFAULT_LIMITS: Record<LimitKey, number> = {
  callsPerMinute: 5, callsPerHour: 20, callsPerDay: 100, replyMaxChars: 1000, memoryChars: 200,
  perUserRunning: 1, globalRunning: 16, chatsPerUser: 1000, messagesPerChat: 400, memoriesPerUser: 50,
};
export const LIMIT_RANGES: Record<LimitKey, [number, number]> = {
  callsPerMinute: [1, 100], callsPerHour: [1, 2000], callsPerDay: [1, 20000], replyMaxChars: [1, 16000], memoryChars: [50, 2000],
  perUserRunning: [1, 4], globalRunning: [1, 64], chatsPerUser: [10, 100000], messagesPerChat: [20, 5000], memoriesPerUser: [1, 1000],
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
  kind: 'range' | 'order';
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
  return null;
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

/**
 * The max_tokens the service sends for an answer of `chars` characters (FB-5 revised; the twin of `limits::reply_tokens` in
 * ai-service, with the same vectors): Chinese and Japanese 1 character = 1.5 tokens, so an answer can really reach the
 * count; English 4 letters count as 1 character = about 1 token.
 */
export function replyTokens(chars: number, lang: 'zh-CN' | 'en' | 'ja'): number {
  const n = Math.max(0, Math.floor(chars));
  return lang === 'en' ? n : Math.floor((3 * n + 1) / 2);
}

/** The live line under the answer length: what the typed number means as max_tokens, or null while it is not a usable number */
export function replyTokensLine(text: string, ranges?: LimitRanges): { zh: number; en: number } | null {
  const n = whole(text);
  const [min, max] = rangeOf(ranges, 'replyMaxChars');
  if (n == null || n < min || n > max) return null;
  return { zh: replyTokens(n, 'zh-CN'), en: replyTokens(n, 'en') };
}

// ---------------------------------------------------------------- the category tabs (FB-6, AS-9)

/** The four categories of AI settings, in the order of the tab row (no generation tab in v1.0) */
export const AI_TABS = ['feature', 'users', 'usage', 'safety'] as const;
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

/** The connection shows as a one-line summary (with an edit button): saved, tested, nothing typed and no key problem */
export function connCollapsed(
  s: Pick<AiSettings, 'verified' | 'keyProblem' | 'keys' | 'provider' | 'baseUrl' | 'updatedBy' | 'defaults'>, dirty: boolean, editing: boolean,
): boolean {
  return !editing && !dirty && s.verified && !s.keyProblem && s.baseUrl.trim() !== '' && connectionSaved(s) && requiredKeysSaved(s);
}

/** The model shows as a one-line summary: a model is saved, the test passed and the field still holds the saved one */
export function modelCollapsed(s: Pick<AiSettings, 'verified' | 'model'>, typedModel: string, editing: boolean): boolean {
  return !editing && s.verified && s.model.trim() !== '' && (typedModel.trim() === '' || typedModel.trim() === s.model);
}

/** All fine: connection, model and test are OK and AI+ is on (the status line is then one sentence instead of four chips) */
export const allFine = (c: StatusChips, keyProblem: boolean): boolean =>
  c.conn === 'ok' && c.model === 'ok' && c.test === 'ok' && c.ai === 'on' && !keyProblem;

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
