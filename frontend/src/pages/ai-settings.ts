// AI settings (#/ai-settings, v0.8, administrators only; main.ts shows "Not allowed" to everyone else). The page always
// renders: it first reads the state of ai-service from the MAIN service (GET /api/auth/state, `aiService`, from ai-service's
// reports) and loads the settings only while ai-service is connected. Otherwise it shows a status panel that names the
// cause (main service down, the AI link off, ai-service not running / stopped / not connected / no report yet; one probe of
// GET /api/ai/status tells "not running" from "not connected") and checks again every 10 s. Sections, in order:
//   0. the four status chips (connection, model, test, AI on / off), the connection to ai-service, and a key problem;
//   1. Connection: provider (OpenAI-compatible API, Anthropic, Cloudflare AI Gateway), address, keys. Key fields are always
//      empty (never prefilled; "Saved: …1234" under them); an empty field keeps the saved key, unless the provider or the
//      address's origin changed (then every saved key must be typed again). Saving lists the models automatically;
//   2. Model: the current model, one input with the listed models as suggestions (or typed by hand); models that are
//      probably not chat models (embeddings, classifiers, …) come last with a tag, and a hint when one is chosen;
//   3. Test and enable: the AI+ feature can be switched on only after a successful test of the saved settings (a save that
//      changes the connection or the model switches it off for everyone; the texts say so and ask to test, then turn it on);
//   4. Users with AI+ (every administrator and every user with the AI+ feature of User management): their badge, their AI+
//      feature (Pause / Resume, no confirmation; a paused user stays listed and keeps their data; this page never changes the
//      AI+ feature of User management), counts, tokens (not on narrow screens: they are in Usage), and a read-only viewer of
//      their chats and memories with Delete buttons (every view, delete, pause and resume is written to the System log by the
//      AI service). Below 560 px each user is a small card instead of a table row (View stays on the name line);
//   5. Usage: calls and tokens by user, by model and over time (no chart library: CSS bars);
//   6. Chat retention: days after the last message;
//   7. Safety limits (v0.8 spec 14.1): they protect the server; they are not usage limits.
// Typed keys live only in this module's state and are forgotten after a successful save, when the page is left and on
// sign-out.
import '../styles/ai-settings.css';
import {
  clearAiData, enableAi, fetchAiSettings, fetchAiStatus, fetchAiUser, fetchAiUserChat, fetchAiUsers, fetchLimits, fetchUsage,
  listModels, resumeAiUser, saveAiSettings, saveLimits, suspendAiUser, verifyAi,
} from '../api/ai.ts';
import { fetchAuthState } from '../api/auth.ts';
import { ApiError, isSignInError } from '../api/client.ts';
import { aiBadge } from '../components/ai-badge.ts';
import { markdownEl } from '../components/ai-render.ts';
import { pageHead } from '../components/common.ts';
import { field } from '../components/fields.ts';
import { miniIcon } from '../components/mini-icons.ts';
import { miniSwitch } from '../components/mini-switch.ts';
import { icon, ICONS, roleName } from '../components/shell.ts';
import { infoTip } from '../components/tooltip.ts';
import { fmtAiTime, footerOf } from '../domain/ai-chat.ts';
import { chatTitleEl, chatTitleText, userBody, whoLine } from './ai.ts';
import { renderForbidden } from './forbidden.ts';
import {
  aiServiceOf, barPercent, canEnable, changesFingerprint, connUpdate, DEFAULT_LIMITS, defaultUrl, fieldOfError, fieldsOf, filterModels,
  FORMATS, formOf, GATEWAY_AUTHS, GATEWAY_PLACEHOLDER, headerClash, httpWarning, inferFormat, isDirty, keyAfter, LIMIT_KEYS,
  connectionSaved, focusAfterClear, LIMIT_RANGES, limitProblem, limitsFormOf, limitsUpdate, modelKind, needsReentry, orderModels, PROVIDERS, requiredKeysSaved,
  retentionOf, savedToastKey, serviceCause, statusChips, testLineOf, urlAfterSwitch, type ConnForm, type LimitsForm, type ModelKind, type SettingsField,
  type SvcCause, type SvcProbe,
} from '../domain/ai-settings.ts';
import { badgeState } from '../domain/badge-state.ts';
import { aiCodeText, errorText } from '../i18n/errors.ts';
import { localeTag, t, tn, type MsgKey } from '../i18n/index.ts';
import { onRouteChange, onSignedIn, refreshAi, render } from '../state/actions.ts';
import { S, onClearUserData } from '../state/store.ts';
import type {
  AiAdminUserDetail, AiAdminUserRow, AiChatDetail, AiKeySlot, AiLimits, AiLimitsAnswer, AiMessage, AiModels, AiProvider, AiService,
  AiSettings, AiUsage, AiVerify,
} from '../types/api.ts';
import { ask, dialogTitleId } from '../utils/dialog.ts';
import { h, type Child } from '../utils/dom.ts';
import { toast } from '../utils/toast.ts';

const PROVIDER_LABEL: Record<AiProvider, MsgKey> = {
  openai: 'ai.set.provider.openai', anthropic: 'ai.set.provider.anthropic', gateway: 'ai.set.provider.gateway',
};
const GATEWAY_AUTH_LABEL: Record<string, string> = {
  bearer: 'Authorization: Bearer', 'x-api-key': 'x-api-key', 'cf-aig-authorization': 'cf-aig-authorization',
};
const LIMIT_LABEL: Record<keyof AiLimits, MsgKey> = {
  perUserRunning: 'ai.set.limit.perUserRunning', globalRunning: 'ai.set.limit.globalRunning', chatsPerUser: 'ai.set.limit.chatsPerUser',
  messagesPerChat: 'ai.set.limit.messagesPerChat', memoriesPerUser: 'ai.set.limit.memoriesPerUser', memoryChars: 'ai.set.limit.memoryChars',
  messageChars: 'ai.set.limit.messageChars',
};
const PERIODS = [7, 30, 90, 365] as const;
/** The tag of a listed model that is probably not a chat model (round 2) */
const KIND_LABEL: Record<ModelKind, MsgKey> = {
  rerank: 'ai.set.kind.rerank', embedding: 'ai.set.kind.embedding', guard: 'ai.set.kind.guard', image: 'ai.set.kind.image',
  audio: 'ai.set.kind.audio', parse: 'ai.set.kind.parse', vision: 'ai.set.kind.vision',
};
/** The text of each cause of the status panel (contracts 2.9 / 2.10) */
const CAUSE_TEXT: Record<Exclude<SvcCause, 'connected'>, MsgKey> = {
  backendDown: 'ai.err.backendDown', linkOff: 'ai.set.svc.linkOff', notRunning: 'ai.set.svc.notRunning', stopped: 'ai.set.svc.stopped',
  notConnected: 'ai.set.svc.notConnected', waiting: 'ai.set.svc.waiting',
};
/** While the status panel is shown, the page checks again this often */
const RECHECK_MS = 10_000;

// ---------------------------------------------------------------- page-local state

let settings: AiSettings | null = null;
let loadError: unknown = null;
let loading = false;
let wanted = true;

/** What the page shows: the settings (ai-service connected), the status panel, or nothing yet (the first check runs) */
let phase: 'checking' | 'settings' | 'panel' = 'checking';
/** `aiService` of the main service's last answer (null: not read yet, or the answer had none) */
let svc: AiService | null = null;
let cause: Exclude<SvcCause, 'connected'> | null = null;
let svcBusy = false;
let svcTimer: number | null = null;
/** When the panel was first shown (Date.now(); 0 = not shown) */
let panelSince = 0;
/** A call of this page failed because ai-service is gone or not connected (the main service may not have noticed yet) */
let failedAs: 'notRunning' | 'notConnected' | null = null;
let lastDownAt = 0;
/** ai-service says this account is not an administrator (demoted meanwhile): "Not allowed" */
let forbidden = false;

let form: ConnForm | null = null;
let fieldErr: Partial<Record<SettingsField, string>> = {};
let connError: string | null = null;
let saving = false;

let models: AiModels | null = null;
let modelsBusy = false;
let modelsError: string | null = null;
let modelInput = '';
let modelSaving = false;
let modelError: string | null = null;

let testBusy = false;
let testResult: AiVerify | null = null;
let testError: string | null = null;
let enableBusy = false;

let users: AiAdminUserRow[] | null = null;
let usersError: unknown = null;
/** The user whose AI+ feature is being paused or resumed (the buttons wait meanwhile) */
let fnBusy: number | null = null;

let usage: AiUsage | null = null;
let usageDays: number = 7;
let usageError: unknown = null;

let retentionText = '';
let retentionError: string | null = null;
let retentionSaving = false;

let limits: AiLimitsAnswer | null = null;
let limitsForm: LimitsForm | null = null;
let limitsError: unknown = null;
let limitsFieldErr: { key: keyof AiLimits; text: string } | null = null;
let limitsSaveError: string | null = null;
let limitsSaving = false;

function forgetTyped(): void {
  if (form) form = { ...form, keys: { apiKey: '', gatewayKey: '' }, remove: { apiKey: false, gatewayKey: false } };
}

function resetAll(): void {
  settings = null;
  loadError = null;
  loading = false;
  wanted = true;
  stopRecheck();
  phase = 'checking';
  svc = null;
  cause = null;
  svcBusy = false;
  panelSince = 0;
  failedAs = null;
  lastDownAt = 0;
  forbidden = false;
  form = null;
  fieldErr = {};
  connError = null;
  saving = false;
  models = null;
  modelsBusy = false;
  modelsError = null;
  modelInput = '';
  modelError = null;
  testResult = null;
  testError = null;
  users = null;
  usersError = null;
  fnBusy = null;
  usage = null;
  usageError = null;
  retentionText = '';
  retentionError = null;
  limits = null;
  limitsForm = null;
  limitsError = null;
  limitsFieldErr = null;
  limitsSaveError = null;
}

onRouteChange((from, to) => {
  if (to.name === 'aiSettings') {
    wanted = true;
    forbidden = false;
  }
  if (from.name === 'aiSettings' && to.name !== 'aiSettings') {
    stopRecheck();
    // Typed keys never stay in memory once the page is left
    forgetTyped();
    fieldErr = {};
    connError = null;
    testResult = null;
    testError = null;
    if (settings) form = formOf(settings);
  }
});
onClearUserData(resetAll);
onSignedIn(() => {
  wanted = true;
  loadError = null;
});

const onPage = (): boolean => S.route.name === 'aiSettings' && !S.authView && S.phase === 'ready';

/** The AI service is not there (Nginx's 502, no JSON, offline) */
function isUnreachable(e: unknown): boolean {
  if (!(e instanceof ApiError)) return false;
  if (e.code === 'ai.unavailable') return true;
  return e.kind === 'network' || (e.kind === 'status' && e.status >= 500);
}

// ---------------------------------------------------------------- the connection to ai-service (round 2)

function stopRecheck(): void {
  if (svcTimer != null) window.clearTimeout(svcTimer);
  svcTimer = null;
}

function scheduleRecheck(): void {
  stopRecheck();
  if (phase === 'panel' && onPage()) svcTimer = window.setTimeout(() => void checkService(), RECHECK_MS);
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
  phase = 'panel';
  cause = c;
  if (!panelSince) panelSince = Date.now();
}

/** What the page shows of the connection: a check that changes none of it does not redraw the page (no repeated
 *  announcement of the panel's status text every 10 s) */
const svcKey = (): string => [phase, cause, svc?.connected, svc?.lastSeen, svc?.version, svc?.stopped].join('|');

/**
 * Reads the state of ai-service from the main service and decides what the page shows: the settings (loaded now) while it
 * is connected, else the status panel with its cause (checked again every 10 s while the page is open). `manual`: Retry.
 */
async function checkService(manual = false): Promise<void> {
  if (svcBusy) return;
  svcBusy = true;
  wanted = false;
  stopRecheck();
  const before = svcKey();
  let a: AiService | null = null;
  let failed = false;
  try {
    const st = await fetchAuthState();
    // Signed out meanwhile: the session check shows the sign-in page
    if (!st.user) {
      svcBusy = false;
      return;
    }
    a = aiServiceOf(st);
  } catch (e) {
    if (isSignInError(e)) {
      svcBusy = false;
      return;
    }
    failed = true;
  }
  svc = failed ? svc : a;
  let next: SvcCause;
  if (failed) {
    next = 'backendDown';
  } else if (!a) {
    // An answer without aiService (a main service older than this page): ai-service itself tells
    const probe = await probeService();
    next = probe === 'ok' ? 'connected' : probe === 'refused' ? 'notConnected' : 'notRunning';
  } else if (a.connected && failedAs) {
    // The main service still counts the last report (up to 60 s), but a call of this page failed: ask ai-service itself
    const probe = await probeService();
    next = probe === 'ok' ? 'connected' : probe === 'refused' ? 'notConnected' : 'notRunning';
  } else {
    const probe = !a.connected && a.link ? await probeService() : null;
    next = serviceCause({ state: { aiService: a }, probe, waitedMs: panelSince ? Date.now() - panelSince : 0 });
  }
  svcBusy = false;
  if (next === 'connected') {
    failedAs = null;
    phase = 'settings';
    cause = null;
    panelSince = 0;
    if (!loading) void load();
  } else {
    showPanel(next);
    scheduleRecheck();
  }
  if (onPage() && (manual || next === 'connected' || svcKey() !== before)) render();
}

/**
 * A call of this page failed. ai-service gone (502, no JSON, offline) or not connected (503 `ai.backendDown`): the status
 * panel takes over and the page checks why (never the bare "main service" text while the main service answers); 403
 * `ai.adminOnly`: "Not allowed". True when handled here; anything else is shown where it happened.
 */
function aiDown(e: unknown): boolean {
  if (e instanceof ApiError && e.status === 403 && e.code === 'ai.adminOnly') {
    forbidden = true;
    if (onPage()) render();
    return true;
  }
  const why = e instanceof ApiError && e.code === 'ai.backendDown' ? 'notConnected' : isUnreachable(e) ? 'notRunning' : null;
  if (!why) return false;
  failedAs = why;
  if (phase !== 'panel') showPanel(why);
  // At most one immediate check per 10 s (several calls fail at once; a flapping service must not loop)
  const soon = Date.now() - lastDownAt < RECHECK_MS;
  lastDownAt = Date.now();
  if (soon) scheduleRecheck();
  else void checkService();
  if (onPage()) render();
  return true;
}

const num = (n: number): string => n.toLocaleString(localeTag());
const when = (sec: number | null): string => (sec == null ? t('common.none') : fmtAiTime(sec, localeTag()));

// ---------------------------------------------------------------- loading

/** Takes saved settings into the page; the connection form follows unless `keepForm` and it has unsaved changes */
function adopt(s: AiSettings, keepForm = false): void {
  const before = settings;
  settings = s;
  if (!form || !keepForm || !before || !isDirty(before, form)) form = formOf(s);
  modelInput = s.model || modelInput;
  if (!retentionText || (before && retentionText === String(before.retentionDays))) retentionText = String(s.retentionDays);
}

async function load(): Promise<void> {
  wanted = false;
  loading = true;
  try {
    const s = await fetchAiSettings();
    loadError = null;
    adopt(s);
    modelInput = s.model;
    retentionText = String(s.retentionDays);
    void loadUsers();
    void loadUsage();
    void loadLimits();
    // The models are listed by themselves when an administrator has saved the connection (not for the untouched
    // defaults of a fresh instance) and no model is chosen yet
    if (!s.model && connectionSaved(s) && requiredKeysSaved(s) && !s.keyProblem && !models) void detectModels();
  } catch (e) {
    loading = false;
    if (aiDown(e)) return;
    loadError = e;
  } finally {
    loading = false;
  }
  if (onPage()) render();
}

async function loadUsers(): Promise<void> {
  try {
    users = await fetchAiUsers();
    usersError = null;
  } catch (e) {
    if (aiDown(e)) return;
    usersError = e;
  }
  if (onPage()) render();
}

async function loadUsage(): Promise<void> {
  const days = usageDays;
  try {
    const u = await fetchUsage(days);
    if (days !== usageDays) return;
    usage = u;
    usageError = null;
  } catch (e) {
    if (aiDown(e)) return;
    usageError = e;
  }
  if (onPage()) render();
}

async function loadLimits(): Promise<void> {
  try {
    limits = await fetchLimits();
    limitsForm = limitsFormOf(limits.limits);
    limitsError = null;
  } catch (e) {
    if (aiDown(e)) return;
    limitsError = e;
  }
  if (onPage()) render();
}

// ---------------------------------------------------------------- actions

async function detectModels(): Promise<void> {
  if (modelsBusy) return;
  modelsBusy = true;
  modelsError = null;
  if (onPage()) render();
  try {
    models = await listModels();
  } catch (e) {
    models = null;
    if (!aiDown(e)) modelsError = t('ai.set.modelsErr', { reason: errorText(e) });
  }
  modelsBusy = false;
  if (onPage()) render();
  // The listing call is counted in Usage
  void loadUsage();
}

async function saveConnection(): Promise<void> {
  const s = settings;
  const f = form;
  if (!s || !f || saving) return;
  const u = connUpdate(s, f);
  if (!Object.keys(u).length) {
    toast(t('ai.set.nothing'));
    return;
  }
  if (s.enabled && changesFingerprint(u, f.provider)
    && !(await ask({ title: t('ai.set.sec.connection'), body: t('ai.set.saveOffAsk'), ok: t('ai.set.save'), danger: true }))) return;
  saving = true;
  fieldErr = {};
  connError = null;
  render();
  try {
    const next = await saveAiSettings(u);
    // The typed keys are forgotten now
    adopt(next);
    toast(t(savedToastKey(next.switchedOff === true)));
    models = null;
    testResult = null;
    void refreshAi(true);
    if (requiredKeysSaved(next) && !next.keyProblem) void detectModels();
  } catch (e) {
    const where = e instanceof ApiError ? fieldOfError(e) : null;
    if (where) fieldErr[where] = errorText(e);
    else if (!aiDown(e)) connError = errorText(e);
  }
  saving = false;
  if (onPage()) render();
}

/** Saves the model typed or picked; true when the saved model is now the one in the field */
async function saveModel(): Promise<boolean> {
  const s = settings;
  if (!s || modelSaving) return false;
  const model = modelInput.trim();
  if (!model) {
    modelError = aiCodeText('ai.modelMissing');
    render();
    return false;
  }
  if (model === s.model) {
    toast(t('ai.set.nothing'));
    return true;
  }
  if (s.enabled && !(await ask({ title: t('ai.set.sec.model'), body: t('ai.set.saveOffAsk'), ok: t('ai.set.saveModel'), danger: true }))) return false;
  modelSaving = true;
  modelError = null;
  render();
  try {
    const manualModel = !(models?.listed && models.models.includes(model));
    const next = await saveAiSettings({ model, manualModel });
    adopt(next, true);
    modelInput = next.model;
    testResult = null;
    toast(t(savedToastKey(next.switchedOff === true)));
    void refreshAi(true);
  } catch (e) {
    if (!aiDown(e)) modelError = errorText(e);
  }
  modelSaving = false;
  if (onPage()) render();
  return settings?.model === model;
}

/** The model in the field differs from the saved one: the Test button saves it first (one click; the owner found that Test
 *  seemed to do nothing after picking a model) */
const modelUnsaved = (s: AiSettings): boolean => modelInput.trim() !== '' && modelInput.trim() !== s.model;

async function saveAndTest(): Promise<void> {
  if (settings && modelUnsaved(settings) && !(await saveModel())) return;
  await runTest();
}

/** While a test runs: the seconds so far, so a slow provider does not look like a dead button */
let testTimer: number | null = null;

async function runTest(): Promise<void> {
  if (!settings || testBusy) return;
  testBusy = true;
  testResult = null;
  testError = null;
  render();
  const started = Date.now();
  testTimer = window.setInterval(() => {
    const el = document.getElementById('ai-test-status');
    if (el) el.textContent = t('ai.set.testingFor', { s: Math.round((Date.now() - started) / 1000) });
  }, 1000);
  try {
    testResult = await verifyAi();
    // The test changes what is stored (verified, maybe AI switched off): read the settings again
    try {
      adopt(await fetchAiSettings(), true);
    } catch {
      /* the result line is shown anyway */
    }
  } catch (e) {
    if (!aiDown(e)) testError = errorText(e);
  }
  if (testTimer != null) window.clearInterval(testTimer);
  testTimer = null;
  testBusy = false;
  void refreshAi(true);
  if (onPage()) render();
  document.getElementById('ai-test-btn')?.focus();
  // The test call is counted in Usage; a failed test may switch AI off, which changes the badges of Users with AI+
  void loadUsage();
  void loadUsers();
}

async function setEnabled(on: boolean): Promise<void> {
  if (!settings || enableBusy) return;
  enableBusy = true;
  render();
  try {
    adopt(await enableAi(on), true);
    toast(t(on ? 'ai.set.enabled' : 'ai.set.disabled'));
  } catch (e) {
    if (!aiDown(e)) toast(errorText(e), 6000);
  }
  enableBusy = false;
  await refreshAi(true);
  if (onPage()) render();
  document.getElementById('ai-enable')?.focus();
  // The badges of Users with AI+ follow the AI+ feature
  void loadUsers();
}

async function saveRetention(): Promise<void> {
  if (!settings || retentionSaving) return;
  const n = retentionOf(retentionText);
  if (n == null) {
    retentionError = aiCodeText('ai.badRetention');
    render();
    return;
  }
  if (n === settings.retentionDays) {
    toast(t('ai.set.nothing'));
    return;
  }
  retentionSaving = true;
  retentionError = null;
  render();
  try {
    adopt(await saveAiSettings({ retentionDays: n }), true);
    retentionText = String(n);
    toast(t('ai.set.retentionSaved', { n }));
    void refreshAi(true);
  } catch (e) {
    if (!aiDown(e)) retentionError = errorText(e);
  }
  retentionSaving = false;
  if (onPage()) render();
}

async function saveLimitsForm(): Promise<void> {
  const l = limits;
  const f = limitsForm;
  if (!l || !f || limitsSaving) return;
  const bad = limitProblem(f, l.ranges);
  if (bad) {
    const [min, max] = l.ranges[bad];
    limitsFieldErr = { key: bad, text: t('ai.set.limitBad', { min: num(min), max: num(max) }) };
    render();
    document.getElementById(`ai-lim-${bad}`)?.focus();
    return;
  }
  const u = limitsUpdate(l.limits, f);
  if (!Object.keys(u).length) {
    toast(t('ai.set.nothing'));
    return;
  }
  limitsSaving = true;
  limitsFieldErr = null;
  limitsSaveError = null;
  render();
  try {
    limits = await saveLimits(u);
    limitsForm = limitsFormOf(limits.limits);
    toast(t('ai.set.limitsSaved'));
    void refreshAi(true);
  } catch (e) {
    const key = e instanceof ApiError ? (LIMIT_KEYS as readonly string[]).find((k) => k === e.field) as keyof AiLimits | undefined : undefined;
    if (key) {
      const [min, max] = l.ranges[key];
      limitsFieldErr = { key, text: t('ai.set.limitBad', { min: num(min), max: num(max) }) };
    } else if (!aiDown(e)) {
      limitsSaveError = errorText(e);
    }
  }
  limitsSaving = false;
  if (onPage()) render();
}

/**
 * Pauses or resumes a user's AI+ feature (round 2; no confirmation; the API calls are still named suspend / resume). It never
 * changes the AI+ feature of User management: a paused user stays listed here, keeps reading and deleting their own chats
 * and memories, and their running answer stops at once. A success toast only after the server confirmed it.
 */
async function setFunction(u: AiAdminUserRow, suspend: boolean): Promise<void> {
  if (fnBusy != null) return;
  fnBusy = u.id;
  render();
  let down = false;
  try {
    await (suspend ? suspendAiUser(u.id) : resumeAiUser(u.id));
    toast(t(suspend ? 'ai.set.fn.suspendDone' : 'ai.set.fn.resumeDone', { name: u.username }));
  } catch (e) {
    down = aiDown(e);
    if (!down) toast(errorText(e), 6000);
  }
  fnBusy = null;
  if (down) return;
  await loadUsers();
  void refreshAi(true);
  document.getElementById(`ais-fn-${u.id}`)?.focus();
}

// ---------------------------------------------------------------- the connection form, kept in step while typing

/** Updates the parts of the form that depend on what is typed (no redraw, so the caret stays) */
function syncConnection(): void {
  const s = settings;
  const f = form;
  if (!s || !f) return;
  const dirty = isDirty(s, f);
  for (const id of ['ai-savefirst', 'ai-test-savefirst']) {
    const note = document.getElementById(id);
    if (note) note.hidden = !dirty;
  }
  for (const id of ['ai-models-btn', 'ai-test-btn']) {
    const b = document.getElementById(id);
    if (b instanceof HTMLButtonElement) b.disabled = dirty || (id === 'ai-test-btn' ? testBusy : modelsBusy);
  }
  const warn = document.getElementById('ai-url-warn');
  if (warn) warn.hidden = !httpWarning(f.baseUrl);
  const clash = document.getElementById('ai-clash');
  if (clash) clash.hidden = !headerClash(f, keyAfter(s, f, 'apiKey'));
  const reentry = needsReentry(s, f);
  for (const slot of fieldsOf(f.provider).slots) {
    const hint = document.getElementById(`ai-key-${slot}-hint`);
    if (hint) hint.textContent = keyHint(s, slot, reentry);
  }
}

function keyHint(s: AiSettings, slot: AiKeySlot, reentry: boolean): string {
  const k = s.keys[slot];
  const state = !k.set ? t('ai.set.keyNone') : k.unreadable ? t('ai.set.keyUnreadable') : k.hint ? t('ai.set.keySaved', { hint: k.hint }) : t('ai.set.keySavedShort');
  if (!k.set) return state;
  return `${state} · ${t(reentry ? 'ai.set.keyReenterHint' : 'ai.set.keyKeep')}`;
}

function fieldError(where: SettingsField): HTMLElement | null {
  const text = fieldErr[where];
  return text ? h('p', { class: 'ais-ferr', role: 'alert' }, text) : null;
}

function connectionSection(s: AiSettings, f: ConnForm): HTMLElement {
  const fields = fieldsOf(f.provider);
  const dirty = isDirty(s, f);
  const pick = (p: AiProvider): void => {
    if (!form || form.provider === p) return;
    const url = urlAfterSwitch(form.baseUrl, form.provider, p, s.defaults);
    form = { ...form, provider: p, baseUrl: url, format: p === 'gateway' ? inferFormat(url) : form.format };
    fieldErr = {};
    render();
  };
  const provider = h('fieldset', { class: 'fld ais-provider' },
    h('legend', { class: 'fld-l' }, t('ai.set.provider')),
    h('div', { class: 'ais-seg' }, PROVIDERS.map((p) => h('label', { class: 'ais-seg-o' },
      h('input', { type: 'radio', name: 'ai-prov', id: `ai-prov-${p}`, value: p, checked: p === f.provider, disabled: saving, onchange: () => pick(p) }),
      h('span', null, t(PROVIDER_LABEL[p]))))));

  const url = field({
    id: 'ai-url', label: t(f.provider === 'gateway' ? 'ai.set.gatewayUrl' : 'ai.set.baseUrl'), value: f.baseUrl, autocomplete: 'url', maxlength: 300,
    hint: f.provider === 'openai' ? t('ai.set.baseUrlHint.openai') : f.provider === 'anthropic' ? t('ai.set.baseUrlHint.anthropic') : undefined,
    disabled: saving,
    onInput: (v) => {
      if (!form) return;
      // A gateway's API format follows the address while it was not chosen by hand
      const followed = form.provider === 'gateway' && form.format === inferFormat(form.baseUrl);
      form = { ...form, baseUrl: v, format: followed ? inferFormat(v) : form.format };
      const sel = document.getElementById('ai-format');
      if (sel instanceof HTMLSelectElement) sel.value = form.format;
      syncConnection();
    },
  });
  url.input.type = 'url';
  url.input.inputMode = 'url';
  if (f.provider === 'gateway') url.input.placeholder = GATEWAY_PLACEHOLDER;
  else url.input.placeholder = defaultUrl(f.provider, s.defaults);

  const gateway = fields.gateway ? h('div', { class: 'ais-row2' },
    h('div', { class: 'fld' },
      h('label', { class: 'fld-l', for: 'ai-format' }, t('ai.set.format')),
      h('select', {
        id: 'ai-format', class: 'ais-select', disabled: saving,
        onchange: (e: Event) => {
          if (!form) return;
          form = { ...form, format: (e.target as HTMLSelectElement).value as ConnForm['format'] };
          syncConnection();
        },
      }, FORMATS.map((x) => h('option', { value: x, selected: x === f.format ? 'selected' : null }, t(x === 'openai' ? 'ai.set.format.openai' : 'ai.set.format.anthropic'))))),
    h('div', { class: 'fld' },
      h('label', { class: 'fld-l', for: 'ai-gwauth' }, t('ai.set.gatewayAuth')),
      h('select', {
        id: 'ai-gwauth', class: 'ais-select', disabled: saving,
        onchange: (e: Event) => {
          if (!form) return;
          form = { ...form, gatewayAuth: (e.target as HTMLSelectElement).value as ConnForm['gatewayAuth'] };
          syncConnection();
        },
      }, GATEWAY_AUTHS.map((x) => h('option', { value: x, selected: x === f.gatewayAuth ? 'selected' : null }, GATEWAY_AUTH_LABEL[x] ?? x))))) : null;

  const reentry = needsReentry(s, f);
  const keyFields = fields.slots.map((slot) => {
    const label = slot === 'gatewayKey' ? t('ai.set.gatewayKey') : f.provider === 'gateway' ? t('ai.set.upstreamKey') : t('ai.set.apiKey');
    const k = s.keys[slot];
    const fld = field({
      id: `ai-key-${slot}`, label, type: 'password', autocomplete: 'new-password', value: f.keys[slot], maxlength: 400,
      hint: keyHint(s, slot, reentry), disabled: saving,
      onInput: (v) => {
        if (!form) return;
        form = { ...form, keys: { ...form.keys, [slot]: v } };
        syncConnection();
      },
    });
    // The reveal button of the field names the key, not "password" (the shared field() labels it for passwords)
    const eye = fld.el.querySelector<HTMLButtonElement>('.fld-eye');
    if (eye) {
      eye.setAttribute('aria-label', t('ai.set.showKey'));
      eye.title = t('ai.set.showKey');
      // field()'s own click handler runs first and sets the password texts; this one runs after it
      eye.addEventListener('click', () => {
        eye.title = t(eye.getAttribute('aria-pressed') === 'true' ? 'ai.set.hideKey' : 'ai.set.showKey');
      });
    }
    const remove = k.set
      ? h('label', { class: 'ais-remove', for: `ai-rm-${slot}` },
        h('input', {
          type: 'checkbox', id: `ai-rm-${slot}`, checked: f.remove[slot], disabled: saving,
          onchange: (e: Event) => {
            if (!form) return;
            form = { ...form, remove: { ...form.remove, [slot]: (e.target as HTMLInputElement).checked } };
            syncConnection();
          },
        }), h('span', null, t('ai.set.keyRemove')))
      : null;
    return h('div', { class: 'ais-key' }, fld.el, remove, fieldError(slot));
  });

  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-conn-h' },
    h('h2', { id: 'ais-conn-h' }, t('ai.set.sec.connection')),
    h('form', {
      id: 'ai-conn', class: 'ais-form', novalidate: true, 'aria-busy': saving ? 'true' : null,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void saveConnection();
      },
    },
    provider,
    // The address first (autocomplete "url"), then the keys ("new-password": a saved Xexam password is never filled in)
    h('div', { class: 'ais-url' }, url.el,
      h('p', { class: 'ais-warn', id: 'ai-url-warn', hidden: !httpWarning(f.baseUrl) }, t('ai.set.httpWarn')),
      fieldError('baseUrl')),
    gateway,
    h('div', { class: 'ais-keys' }, keyFields),
    h('p', { class: 'ais-ferr', id: 'ai-clash', hidden: !headerClash(f, keyAfter(s, f, 'apiKey')) }, aiCodeText('ai.headerClash')),
    connError ? h('div', { class: 'err', role: 'alert' }, h('p', null, connError)) : null,
    h('div', { class: 'acts' },
      h('button', { class: 'btn primary', id: 'ai-save', type: 'submit', disabled: saving }, t(saving ? 'common.saving' : 'ai.set.save')),
      h('p', { class: 'ais-note', id: 'ai-savefirst', hidden: !dirty }, t('ai.set.saveFirst')))));
}

// ---------------------------------------------------------------- model, test and enable

/** The hint under the model field when its model is probably not a chat model ('' = none) */
function kindHint(model: string): string {
  const m = model.trim();
  const k = m ? modelKind(m) : null;
  return k ? t('ai.set.kindHint', { model: m, kind: t(KIND_LABEL[k]) }) : '';
}

function modelSection(s: AiSettings, dirty: boolean): HTMLElement {
  const list = models?.listed ? models.models : [];
  // Likely chat models first, then the ones that are probably not chat models (marked, never hidden)
  const offered = orderModels(s.model && !list.includes(s.model) ? [s.model, ...list] : list);
  let status: Child = null;
  if (modelsBusy) status = h('p', { class: 'ais-note', role: 'status' }, t('ai.set.detecting'));
  else if (modelsError) status = h('p', { class: 'ais-ferr', role: 'alert' }, modelsError);
  else if (models && !models.listed) status = h('p', { class: 'ais-note' }, t('ai.set.noList'));
  else if (models) status = h('p', { class: 'ais-note', role: 'status' }, tn('ai.set.models', models.models.length));
  // The picker: a text field (typing filters; any model id can still be typed by hand) with its own scrolling list under
  // it, instead of the browser's datalist, which shows a list of 80+ gateway models as one huge popup (owner's report)
  let shown: string[] = [];
  let active = -1;
  let typed = false;
  const listEl = h('ul', { id: 'ai-model-list', class: 'ais-combo-list', role: 'listbox', 'aria-label': t('ai.set.pickModel') });
  listEl.hidden = true;
  const firstHint = kindHint(modelInput);
  const hintEl = h('p', { class: 'ais-note ais-kindhint', id: 'ai-model-kind', hidden: !firstHint }, firstHint);
  const syncHint = (): void => {
    const text = kindHint(input.value);
    hintEl.textContent = text;
    hintEl.hidden = !text;
  };
  const pick = (m: string): void => {
    input.value = m;
    modelInput = m;
    active = -1;
    open(false);
    syncTestLabel();
    syncHint();
  };
  const paintList = (): void => {
    shown = filterModels(offered, typed ? input.value : '');
    listEl.replaceChildren(...(shown.length
      ? shown.map((m, i) => {
        const kind = modelKind(m);
        return h('li', {
          id: `ai-model-o${i}`, role: 'option', class: 'ais-combo-o' + (i === active ? ' on' : '') + (kind ? ' other' : ''),
          'aria-selected': String(i === active),
          // mousedown, not click: it comes before the field's blur, which closes the list
          onmousedown: (e: MouseEvent) => {
            e.preventDefault();
            pick(m);
          },
        // The tag is part of the option's text, so screen readers hear it too
        }, h('span', { class: 'ais-combo-id' }, m), kind ? h('span', { class: 'ais-kind' }, t(KIND_LABEL[kind])) : null);
      })
      : [h('li', { class: 'ais-combo-none', role: 'presentation' }, t('ai.set.noMatch'))]));
    if (active >= 0) input.setAttribute('aria-activedescendant', `ai-model-o${active}`);
    else input.removeAttribute('aria-activedescendant');
    document.getElementById(`ai-model-o${active}`)?.scrollIntoView({ block: 'nearest' });
  };
  function open(on: boolean): void {
    const show = on && offered.length > 0;
    listEl.hidden = !show;
    input.setAttribute('aria-expanded', String(show));
    if (show) paintList();
  }
  const input = h('input', {
    type: 'text', id: 'ai-model', class: 'ais-model', value: modelInput, maxlength: '200', role: 'combobox',
    'aria-autocomplete': 'list', 'aria-expanded': 'false', 'aria-controls': 'ai-model-list', 'aria-describedby': 'ai-model-kind',
    placeholder: t('ai.set.modelFilter'), autocomplete: 'off', spellcheck: 'false', disabled: modelSaving,
    onfocus: () => {
      typed = false;
      active = -1;
      open(true);
    },
    onblur: () => open(false),
    oninput: () => {
      modelInput = input.value;
      typed = true;
      active = -1;
      open(true);
      syncTestLabel();
      syncHint();
    },
    onkeydown: (e: KeyboardEvent) => {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        if (listEl.hidden) open(true);
        if (!shown.length) return;
        active = e.key === 'ArrowDown' ? Math.min(shown.length - 1, active + 1) : Math.max(0, active - 1);
        paintList();
      } else if (e.key === 'Enter' && !listEl.hidden && shown[active] !== undefined) {
        e.preventDefault();
        pick(shown[active] ?? '');
      } else if (e.key === 'Escape' && !listEl.hidden) {
        e.preventDefault();
        open(false);
      }
    },
  });
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-model-h' },
    h('h2', { id: 'ais-model-h' }, t('ai.set.sec.model')),
    h('p', { class: 'ais-current' }, s.model ? t('ai.set.currentModel', { model: s.model }) : t('ai.set.noModel')),
    h('div', { class: 'fld ais-modelfld' },
      h('label', { class: 'fld-l', for: 'ai-model' }, t('ai.set.pickModel')),
      h('div', { class: 'ais-inline' }, h('div', { class: 'ais-combo' }, input, listEl),
        h('button', {
          class: 'btn', id: 'ai-models-btn', type: 'button', disabled: dirty || modelsBusy, onclick: () => void detectModels(),
        }, t('ai.set.refresh')),
        h('button', { class: 'btn primary', id: 'ai-model-save', type: 'button', disabled: modelSaving, onclick: () => void saveModel() }, t('ai.set.saveModel'))),
      hintEl),
    status,
    modelError ? h('p', { class: 'ais-ferr', role: 'alert' }, modelError) : null);
}

/** The Test button's label follows the model field (Test, or Save model and test) */
function syncTestLabel(): void {
  const b = document.getElementById('ai-test-btn');
  if (b && settings && !testBusy) b.textContent = t(modelUnsaved(settings) ? 'ai.set.saveTest' : 'ai.set.test');
}

function testLine(): Child {
  if (testBusy) {
    return h('div', null, h('p', { class: 'ais-note', role: 'status', id: 'ai-test-status' }, t('ai.set.testing')),
      h('p', { class: 'ais-note' }, t('ai.set.testSlow')));
  }
  if (testError) return h('p', { class: 'ais-ferr', role: 'alert' }, t('ai.set.testFail', { reason: testError }));
  // Whether the AI+ feature is on is read from the settings now, so the line follows the switch (never stale after it)
  const line = testLineOf(testResult, settings?.enabled ?? testResult?.enabled ?? false);
  if (line.kind === 'none') return null;
  if (line.kind === 'ok') return h('p', { class: 'ais-ok', role: 'status' }, t(line.key, { ms: num(line.ms) }));
  return h('div', { class: 'ais-fail', role: 'alert' },
    h('p', null, t('ai.set.testFail', { reason: aiCodeText(line.code ?? 'ai.internal') })),
    line.hint ? h('p', null, t(line.hint === 'auth' ? 'ai.set.testHint.auth' : line.hint === 'model' ? 'ai.set.testHint.model'
      : line.hint === 'slow' ? 'ai.set.testHint.slow' : 'ai.set.testHint.connect')) : null,
    line.keepOn ? h('p', null, t('ai.set.testKeepOn')) : null);
}

function testSection(s: AiSettings, dirty: boolean): HTMLElement {
  const can = canEnable(s);
  const blocked = !s.enabled && !can;
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-test-h' },
    h('h2', { id: 'ais-test-h' }, t('ai.set.sec.test')),
    h('div', { class: 'ais-inline' },
      h('button', { class: 'btn', id: 'ai-test-btn', type: 'button', disabled: dirty || testBusy || modelSaving, onclick: () => void saveAndTest() },
        t(testBusy ? 'ai.set.testing' : modelUnsaved(s) ? 'ai.set.saveTest' : 'ai.set.test'))),
    h('p', { class: 'ais-note', id: 'ai-test-savefirst', hidden: !dirty }, t('ai.set.saveFirst')),
    testLine(),
    s.verifiedAt ? h('p', { class: 'ais-note' }, t('ai.set.verifiedAt', { time: when(s.verifiedAt) })) : null,
    h('div', { class: 'ais-enable' },
      h('span', { class: 'ais-enable-l', id: 'ai-enable-l' }, t('ai.set.enable')),
      miniSwitch({
        id: 'ai-enable', checked: s.enabled, labelledBy: 'ai-enable-l', describedBy: blocked ? 'ai-enable-tip' : undefined,
        blocked, disabled: enableBusy, onToggle: (v) => void setEnabled(v),
      }),
      blocked ? h('p', { class: 'ais-note', id: 'ai-enable-tip' }, t('ai.set.enableTip')) : null));
}

// ---------------------------------------------------------------- users with AI+ and their viewer

/** The badge of a listed user (one rule for every badge, domain/badge-state.ts): this page is shown only while ai-service is
 *  connected, so AI+ is available exactly when the AI+ feature is on */
function badgeFor(u: Pick<AiAdminUserRow, 'role' | 'ai' | 'suspended'>, enabled: boolean): HTMLElement | null {
  return aiBadge(badgeState({ role: u.role, service: u.ai, suspended: u.suspended === true, available: enabled }), { admin: true });
}

/** The AI+ feature of a listed user: its state (Active / Paused) and one button (Pause / Resume); administrators are never paused */
function functionCell(u: AiAdminUserRow): HTMLElement {
  const td = (...kids: Child[]): HTMLElement => h('td', { role: 'cell', class: 'ais-fncell', 'data-l': t('ai.set.col.function') }, ...kids);
  if (!u.exists) return td();
  if (u.role === 'admin') return td(h('span', { class: 'ais-note' }, t('ai.set.user.adminAlways')));
  const off = u.suspended === true;
  return td(h('div', { class: 'ais-fn' },
    h('span', { class: 'ais-fn-state ' + (off ? 'off' : 'on') }, t(off ? 'ai.set.fn.suspended' : 'ai.set.fn.active')),
    h('button', {
      class: 'btn ai-mini', id: `ais-fn-${u.id}`, type: 'button', disabled: fnBusy != null,
      'aria-label': t(off ? 'ai.set.fn.resumeAria' : 'ai.set.fn.suspendAria', { name: u.username }),
      onclick: () => void setFunction(u, !off),
    }, t(off ? 'ai.set.fn.resume' : 'ai.set.fn.suspend'))));
}

function usersSection(s: AiSettings): HTMLElement {
  let body: Child;
  if (!users) {
    body = usersError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.usersErr', { reason: errorText(usersError) })))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  } else if (!users.length) {
    body = h('p', { class: 'ais-note' }, t('ai.set.user.noUsers'));
  } else {
    // Explicit table roles: below 560 px the CSS turns the rows into cards (display: block), which would drop the table
    // semantics in some browsers. `data-l` is the column name shown in front of a value inside a card.
    const col = (key: MsgKey, cls = ''): HTMLElement => h('th', { scope: 'col', role: 'columnheader', class: cls || null }, t(key));
    const cell = (label: MsgKey, cls: string, ...kids: Child[]): HTMLElement => h('td', { role: 'cell', class: cls || null, 'data-l': t(label) }, ...kids);
    body = h('div', { class: 'ais-tablewrap' }, h('table', { class: 'ais-table ais-users', role: 'table' },
      h('thead', { role: 'rowgroup' }, h('tr', { role: 'row' },
        // The name and the AI+ badge in columns of their own, so names of any length line up (owner's request)
        col('ai.set.col.user'), col('ai.set.col.badge'), col('ai.set.col.role'), col('ai.set.col.function'),
        col('ai.set.user.chats', 'n'), col('ai.set.user.memories', 'n'), col('ai.set.tokensIn', 'n ais-tok'), col('ai.set.tokensOut', 'n ais-tok'),
        col('ai.set.user.lastUsed'),
        h('th', { scope: 'col', role: 'columnheader', class: 'ais-view' }, h('span', { class: 'sr-only' }, t('ai.set.user.view'))))),
      h('tbody', { role: 'rowgroup' }, users.map((u) => h('tr', { role: 'row', class: u.exists ? null : 'gone' },
        h('th', { scope: 'row', role: 'rowheader', class: 'ais-user' }, h('span', { class: 'ais-uname' }, u.username)),
        cell('ai.set.col.badge', 'ais-badge', u.exists ? badgeFor(u, s.enabled) : h('span', { class: 'dim' }, t('ai.set.user.deleted'))),
        cell('ai.set.col.role', '', roleName(u.role)),
        functionCell(u),
        cell('ai.set.user.chats', 'n', num(u.chats)), cell('ai.set.user.memories', 'n', num(u.memories)),
        cell('ai.set.tokensIn', 'n ais-tok', num(u.tokensIn)), cell('ai.set.tokensOut', 'n ais-tok', num(u.tokensOut)),
        cell('ai.set.user.lastUsed', 'ais-when', when(u.lastUsedAt)),
        h('td', { role: 'cell', class: 'ais-view' }, h('button', {
          class: 'btn ai-mini', id: `ais-view-${u.id}`, type: 'button', 'aria-label': t('ai.set.user.viewAria', { name: u.username }),
          onclick: () => void openViewer(u),
        }, t('ai.set.user.view'))))))));
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-users-h' },
    h('h2', { id: 'ais-users-h' }, t('ai.set.sec.users')), body);
}

function viewerMessage(m: AiMessage): HTMLElement {
  const user = m.role === 'user';
  const f = footerOf(m);
  const foot = !f ? null
    : f.kind === 'writing' ? t('ai.writing')
      : f.kind === 'stopped' ? t('ai.status.stopped')
        : f.kind === 'error' ? t('ai.status.error', { reason: aiCodeText(f.code) })
          : f.kind === 'noAnswer' ? t('ai.status.noAnswer', { reason: aiCodeText(f.code) })
            : t('ai.status.truncated');
  return h('li', { class: 'ai-msg ' + (user ? 'user' : 'assistant') },
    whoLine(m),
    ...(user ? userBody(m) : [markdownEl(m.content)]),
    foot ? h('p', { class: 'ai-foot' + (m.status === 'error' ? ' bad' : '') }, foot) : null);
}

/** The read-only viewer of a user's chats and memories (every view is written to the System log by the AI service) */
async function openViewer(u: AiAdminUserRow): Promise<void> {
  const titleId = dialogTitleId();
  let detail: AiAdminUserDetail | null = null;
  let err: unknown = null;
  let vtab: 'chats' | 'memories' = 'chats';
  let chat: AiChatDetail | null = null;
  let chatId: number | null = null;
  let chatErr: unknown = null;
  let busy = false;
  const d = h('dialog', { class: 'dlg-viewer', 'aria-labelledby': titleId });

  const loadDetail = async (): Promise<void> => {
    try {
      detail = await fetchAiUser(u.id);
      err = null;
      if (chatId != null && !detail.chats.some((c) => c.id === chatId)) {
        chatId = null;
        chat = null;
      }
    } catch (e) {
      if (aiDown(e)) {
        d.close();
        return;
      }
      err = e;
    }
    draw();
  };
  const openChat = async (id: number): Promise<void> => {
    chatId = id;
    chat = null;
    chatErr = null;
    draw();
    try {
      chat = await fetchAiUserChat(u.id, id);
    } catch (e) {
      if (aiDown(e)) {
        d.close();
        return;
      }
      chatErr = e;
    }
    draw();
    document.getElementById('vw-back')?.focus();
  };
  const clear = async (what: 'chat' | 'chats' | 'memories'): Promise<void> => {
    const dt = detail;
    if (!dt || busy) return;
    const picked = what === 'chat' ? dt.chats.find((c) => c.id === chatId) : undefined;
    const title = picked ? chatTitleText(picked) : what === 'chat' ? t('ai.new') : '';
    const body = what === 'chat'
      ? t('ai.set.user.clearChat.ask', { title, name: u.username })
      : what === 'chats'
        ? t('ai.set.user.clearChats.ask', { n: dt.chats.length, name: u.username })
        : t('ai.set.user.clearMemories.ask', { n: dt.memories.length, name: u.username });
    const heading = t(what === 'chat' ? 'ai.set.user.clearChat' : what === 'chats' ? 'ai.set.user.clearChats' : 'ai.set.user.clearMemories');
    if (!(await ask({ title: heading, body, ok: t('ai.set.user.clearOk'), danger: true }))) return;
    busy = true;
    draw();
    try {
      await clearAiData(what === 'chat' && chatId != null ? { userId: u.id, what: 'chat', chatId } : { userId: u.id, what: what === 'memories' ? 'memories' : 'chats' });
      toast(t('ai.set.user.cleared'));
      if (what !== 'memories') {
        chatId = null;
        chat = null;
      }
    } catch (e) {
      if (aiDown(e)) {
        d.close();
        return;
      }
      toast(errorText(e), 6000);
    }
    busy = false;
    await loadDetail();
    // The button that had the focus is gone (the dialog was drawn again): keep it inside the dialog
    document.getElementById(focusAfterClear(what, detail?.chats.map((c) => c.id) ?? []))?.focus();
    void loadUsers();
    void loadUsage();
  };

  function draw(): void {
    const dt = detail;
    const head = h('div', { class: 'vw-head' },
      h('h3', { id: titleId }, t('ai.set.user.title', { name: u.username })),
      h('span', { class: 'tag k' }, t('ai.set.user.readOnly')),
      h('button', { class: 'icon-btn vw-close', type: 'button', 'aria-label': t('common.close'), title: t('common.close'), onclick: () => d.close() }, icon(ICONS.close)));
    if (!dt) {
      d.replaceChildren(head, err
        ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.user.err', { reason: errorText(err) })))
        : h('p', { class: 'dim', role: 'status' }, t('common.loading')));
      return;
    }
    const tabBtn = (which: 'chats' | 'memories', label: string): HTMLElement => h('button', {
      type: 'button', id: `vw-tab-${which}`, 'aria-pressed': String(vtab === which),
      onclick: () => {
        vtab = which;
        draw();
        document.getElementById(`vw-tab-${which}`)?.focus();
      },
    }, label);
    let body: Child;
    if (!dt.chats.length && !dt.memories.length) {
      body = h('p', { class: 'ais-note' }, t('ai.set.user.noData'));
    } else if (vtab === 'memories') {
      body = dt.memories.length
        ? h('ul', { class: 'vw-mems' }, dt.memories.map((m) => h('li', null,
          h('span', { class: 'ai-mem-src', role: 'img', 'aria-label': t(m.source === 'chat' ? 'ai.mem.source.chat' : 'ai.mem.source.manual') },
            miniIcon(m.source === 'chat' ? 'chat' : 'pencil', 'ai-mi')),
          h('span', null, m.content))))
        : h('p', { class: 'ais-note' }, t('ai.mem.empty'));
    } else {
      const list = h('ul', { class: 'vw-chats' }, dt.chats.map((c) => h('li', null, h('button', {
        type: 'button', class: 'ai-chat-open' + (c.id === chatId ? ' on' : ''), id: `vw-c-${c.id}`, 'aria-current': c.id === chatId ? 'true' : null,
        onclick: () => void openChat(c.id),
      }, chatTitleEl(c), h('span', { class: 'ai-chat-at' }, when(c.updatedAt))))));
      const pane = chatId == null
        ? h('p', { class: 'ais-note vw-pick' }, t('ai.set.user.pick'))
        : chatErr
          ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.user.err', { reason: errorText(chatErr) })))
          : !chat
            ? h('p', { class: 'dim', role: 'status' }, t('common.loading'))
            : h('ol', { class: 'ai-msgs' }, chat.messages.map((m) => viewerMessage(m)));
      body = h('div', { class: 'vw-panes ' + (chatId == null ? 'on-list' : 'on-chat') },
        h('div', { class: 'vw-list' }, dt.chats.length ? list : h('p', { class: 'ais-note' }, t('ai.empty'))),
        h('div', { class: 'vw-chat' },
          h('button', {
            class: 'btn quiet vw-back', id: 'vw-back', type: 'button',
            onclick: () => {
              const was = chatId;
              chatId = null;
              chat = null;
              draw();
              if (was != null) document.getElementById(`vw-c-${was}`)?.focus();
            },
          }, miniIcon('back', 'ai-mi'), t('ai.back')),
          pane));
    }
    d.replaceChildren(head,
      h('div', { class: 'seg vw-tabs', role: 'group' },
        tabBtn('chats', `${t('ai.set.user.chats')} (${num(dt.chats.length)})`),
        tabBtn('memories', `${t('ai.set.user.memories')} (${num(dt.memories.length)})`)),
      h('div', { class: 'vw-body' }, body),
      h('div', { class: 'acts vw-acts' },
        vtab === 'chats' && chatId != null
          ? h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void clear('chat') }, t('ai.set.user.clearChat'))
          : null,
        dt.chats.length ? h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void clear('chats') }, t('ai.set.user.clearChats')) : null,
        dt.memories.length ? h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void clear('memories') }, t('ai.set.user.clearMemories')) : null,
        h('button', { class: 'btn', type: 'button', onclick: () => d.close() }, t('common.close'))));
  }

  d.addEventListener('close', () => {
    d.remove();
    document.getElementById(`ais-view-${u.id}`)?.focus();
  });
  draw();
  document.body.append(d);
  if (typeof d.showModal === 'function') d.showModal();
  else d.setAttribute('open', '');
  (d.querySelector<HTMLElement>('.vw-close'))?.focus();
  await loadDetail();
}

// ---------------------------------------------------------------- usage

function countsCells(c: { calls: number; tokensIn: number; tokensOut: number; errors: number; tokensCached: number }): HTMLElement[] {
  return [h('td', { class: 'n' }, num(c.calls)), h('td', { class: 'n' }, num(c.tokensIn)), h('td', { class: 'n' }, num(c.tokensOut)), h('td', { class: 'n' }, num(c.tokensCached)), h('td', { class: 'n' }, num(c.errors))];
}

const countsHead = (): HTMLElement[] => [
  h('th', { scope: 'col', class: 'n' }, t('ai.set.calls')), h('th', { scope: 'col', class: 'n' }, t('ai.set.tokensIn')),
  h('th', { scope: 'col', class: 'n' }, t('ai.set.tokensOut')), h('th', { scope: 'col', class: 'n' }, t('ai.set.tokensCached')),
  h('th', { scope: 'col', class: 'n' }, t('ai.set.errors')),
];

function usageSection(): HTMLElement {
  const select = h('select', {
    id: 'ai-days', class: 'ais-select',
    onchange: () => {
      usageDays = Number(select.value) || 7;
      usage = null;
      render();
      void loadUsage();
    },
  }, PERIODS.map((p) => h('option', { value: String(p), selected: p === usageDays ? 'selected' : null }, t('ai.set.days', { n: p }))));
  let body: Child;
  const u = usage;
  if (!u) {
    body = usageError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.usageErr', { reason: errorText(usageError) })))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  } else {
    const tiles = h('dl', { class: 'ais-totals' },
      ([['ai.set.calls', u.totals.calls], ['ai.set.tokensIn', u.totals.tokensIn], ['ai.set.tokensOut', u.totals.tokensOut], ['ai.set.tokensCached', u.totals.tokensCached], ['ai.set.errors', u.totals.errors]] as Array<[MsgKey, number]>)
        .map(([k, v]) => h('div', { class: 'ais-total' }, h('dt', null, t(k)), h('dd', { class: 'num' }, num(v)))));
    if (!u.totals.calls) {
      body = [tiles, h('p', { class: 'ais-note' }, t('ai.set.noUsage'))];
    } else {
      const maxCalls = Math.max(0, ...u.byPeriod.map((p) => p.calls));
      body = [
        tiles,
        h('h3', { class: 'ais-h3' }, t('ai.set.byUser')),
        h('div', { class: 'ais-tablewrap' }, h('table', { class: 'ais-table' },
          h('thead', null, h('tr', null, h('th', { scope: 'col' }, t('ai.set.col.user')), countsHead(), h('th', { scope: 'col' }, t('ai.set.user.lastUsed')))),
          h('tbody', null, u.byUser.map((r) => h('tr', null,
            h('th', { scope: 'row' }, r.userId == null ? t('ai.set.deletedUsers') : r.username ?? `#${r.userId}`), countsCells(r),
            h('td', { class: 'ais-when' }, when(r.lastAt))))))),
        h('h3', { class: 'ais-h3' }, t('ai.set.byModel')),
        h('div', { class: 'ais-tablewrap' }, h('table', { class: 'ais-table' },
          h('thead', null, h('tr', null, h('th', { scope: 'col' }, t('ai.set.col.model')), countsHead())),
          // The model-listing calls are stored without a model (kind 'models'): labelled like "Deleted users" above
          h('tbody', null, u.byModel.map((r) => h('tr', null,
            r.model ? h('th', { scope: 'row', class: 'ais-mono' }, r.model) : h('th', { scope: 'row' }, t('ai.set.modelListCalls')),
            countsCells(r)))))),
        // v0.9: what the calls were for (AI Chat, hint, explanation, summary), in the server's order
        u.byPurpose
          ? h('h3', { class: 'ais-h3' }, t('ai.set.usage.byPurpose'))
          : null,
        u.byPurpose
          ? h('div', { class: 'ais-tablewrap' }, h('table', { class: 'ais-table' },
            h('thead', null, h('tr', null, h('th', { scope: 'col' }, t('ai.set.usage.byPurpose')), countsHead())),
            h('tbody', null, u.byPurpose.map((r) => h('tr', null,
              h('th', { scope: 'row' }, t(`ai.set.usage.purpose.${r.purpose}` as MsgKey)), countsCells(r))))))
          : null,
        h('h3', { class: 'ais-h3' }, t('ai.set.byPeriod')),
        h('div', { class: 'ais-tablewrap' }, h('table', { class: 'ais-table ais-period' },
          h('thead', null, h('tr', null,
            h('th', { scope: 'col' }, t(u.unit === 'month' ? 'ai.set.col.month' : 'ai.set.col.day')),
            h('th', { scope: 'col', class: 'ais-barcol' }, h('span', { class: 'sr-only' }, t('ai.set.calls'))),
            countsHead())),
          h('tbody', null, u.byPeriod.map((r) => h('tr', null,
            h('th', { scope: 'row', class: 'num' }, r.period),
            h('td', { class: 'ais-barcol', 'aria-hidden': 'true' }, h('span', { class: 'ais-bar' }, h('i', { style: `width:${barPercent(r.calls, maxCalls)}%` }))),
            countsCells(r)))))),
      ];
    }
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-usage-h' },
    h('div', { class: 'ais-sechead' },
      h('h2', { id: 'ais-usage-h' }, t('ai.set.sec.usage')),
      h('div', { class: 'ais-period-pick' }, h('label', { class: 'fld-l', for: 'ai-days' }, t('ai.set.period')), select)),
    body);
}

// ---------------------------------------------------------------- retention and safety limits

function retentionSection(): HTMLElement {
  const tip = infoTip('ai-retention-tip', t('ai.set.retentionTipBtn'), t('ai.set.retentionTip'));
  const input = h('input', {
    type: 'number', id: 'ai-retention', class: 'ais-num', min: '1', max: '3650', step: '1', inputmode: 'numeric', value: retentionText,
    disabled: retentionSaving, 'aria-describedby': 'ai-retention-tip',
    oninput: () => {
      retentionText = input.value;
    },
  });
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-ret-h' },
    h('h2', { id: 'ais-ret-h' }, t('ai.set.sec.retention')),
    h('form', {
      class: 'ais-inline ais-retention', novalidate: true,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void saveRetention();
      },
    },
    h('label', { class: 'fld-l', for: 'ai-retention' }, t('ai.set.retentionDays')), tip.el, input,
    h('button', { class: 'btn primary', id: 'ai-retention-save', type: 'submit', disabled: retentionSaving }, t('ai.set.save'))),
    retentionError ? h('p', { class: 'ais-ferr', role: 'alert' }, retentionError) : null);
}

function limitsSection(): HTMLElement {
  const l = limits;
  const f = limitsForm;
  let body: Child;
  if (!l || !f) {
    body = limitsError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.limitsErr', { reason: errorText(limitsError) })))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  } else {
    body = h('form', {
      class: 'ais-limits', novalidate: true, 'aria-busy': limitsSaving ? 'true' : null,
      onsubmit: (e: SubmitEvent) => {
        e.preventDefault();
        void saveLimitsForm();
      },
    },
    h('div', { class: 'ais-limitgrid' }, LIMIT_KEYS.map((k) => {
      const [min, max] = l.ranges[k] ?? LIMIT_RANGES[k];
      const def = l.defaults[k] ?? DEFAULT_LIMITS[k];
      const bad = limitsFieldErr?.key === k;
      const input = h('input', {
        type: 'number', id: `ai-lim-${k}`, class: 'ais-num', min: String(min), max: String(max), step: '1', inputmode: 'numeric',
        value: f[k], disabled: limitsSaving, 'aria-describedby': `ai-lim-${k}-hint`, 'aria-invalid': bad ? 'true' : null,
        oninput: () => {
          if (limitsForm) limitsForm = { ...limitsForm, [k]: input.value };
        },
      });
      return h('div', { class: 'fld ais-limit' },
        h('label', { class: 'fld-l', for: input.id }, t(LIMIT_LABEL[k])),
        input,
        h('p', { class: 'fld-hint', id: `ai-lim-${k}-hint` }, t('ai.set.limitHint', { min: num(min), max: num(max), def: num(def) })),
        bad ? h('p', { class: 'ais-ferr', role: 'alert' }, limitsFieldErr?.text ?? '') : null);
    })),
    limitsSaveError ? h('div', { class: 'err', role: 'alert' }, h('p', null, limitsSaveError)) : null,
    h('div', { class: 'acts' },
      h('button', { class: 'btn primary', id: 'ai-limits-save', type: 'submit', disabled: limitsSaving }, t(limitsSaving ? 'common.saving' : 'ai.set.save')),
      h('button', {
        class: 'btn', id: 'ai-limits-reset', type: 'button', disabled: limitsSaving,
        onclick: () => {
          if (!limits) return;
          limitsForm = limitsFormOf(limits.defaults);
          limitsFieldErr = null;
          render();
        },
      }, t('ai.set.limitsReset'))));
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-lim-h' },
    h('h2', { id: 'ais-lim-h' }, t('ai.set.sec.limits')),
    h('p', { class: 'ais-lead' }, t('ai.set.limitsLead')),
    body);
}

// ---------------------------------------------------------------- the page

function statusBanner(s: AiSettings): HTMLElement {
  // Connection, model and test each with its own chip (owner's request), then whether AI is on
  const c = statusChips(s, testResult != null && !testResult.ok);
  const problem = s.keyProblem === 'mismatch' ? 'ai.set.keyProblem' : s.keyProblem === 'invalidEnv' ? 'ai.set.keyEnv' : s.keyProblem === 'invalidFile' ? 'ai.set.keyFile' : null;
  const chip = (cls: string, key: MsgKey): HTMLElement => h('span', { class: 'ais-chip ' + cls }, t(key));
  return h('div', { class: 'ais-status', role: 'status' },
    chip(c.conn === 'ok' ? 'ok' : 'none', c.conn === 'ok' ? 'ai.set.chip.connOk' : 'ai.set.chip.connNone'),
    chip(c.model === 'ok' ? 'ok' : 'none', c.model === 'ok' ? 'ai.set.chip.modelOk' : 'ai.set.chip.modelNone'),
    chip(c.test === 'ok' ? 'ok' : c.test === 'fail' ? 'fail' : 'unverified',
      c.test === 'ok' ? 'ai.set.chip.testOk' : c.test === 'fail' ? 'ai.set.chip.testFail' : 'ai.set.chip.testNone'),
    chip(c.ai === 'on' ? 'on' : 'none', c.ai === 'on' ? 'ai.set.state.on' : 'ai.set.state.off'),
    // Round 2: the connection to ai-service, under the chips (the chips themselves are unchanged)
    svc?.connected
      ? h('p', { class: 'ais-svc-ok' }, t('ai.set.svc.connected', { v: svc.version ?? '–', time: when(svc.lastSeen) }))
      : null,
    problem ? h('p', { class: 'callout bad ais-problem', role: 'alert' }, t(problem)) : null);
}

/** ai-service is not connected (or the main service cannot be reached): why, the last report, and Retry */
function servicePanel(): HTMLElement {
  const c = cause ?? 'notConnected';
  const a = c === 'backendDown' ? null : svc;
  return h('section', { class: 'panel ais-sec ais-svc', 'aria-labelledby': 'ais-svc-h' },
    h('h2', { id: 'ais-svc-h' }, t('ai.set.svc.title')),
    h('p', { class: 'callout warn ais-svc-cause', role: 'status' }, t(CAUSE_TEXT[c])),
    a ? h('p', { class: 'ais-note' }, a.lastSeen != null ? t('ai.set.svc.lastSeen', { time: when(a.lastSeen) }) : t('ai.set.svc.never')) : null,
    a?.version ? h('p', { class: 'ais-note' }, t('ai.set.svc.version', { v: a.version })) : null,
    h('p', { class: 'ais-note' }, t('ai.set.svc.recheck')),
    h('div', { class: 'acts' },
      h('button', { class: 'btn', id: 'ais-svc-retry', type: 'button', 'aria-busy': svcBusy ? 'true' : null, onclick: () => void checkService(true) },
        t('common.retry'))));
}

export function renderAiSettings(): HTMLElement {
  if (wanted && !svcBusy) void checkService();
  if (forbidden) return renderForbidden('forbidden.aiSettings');
  const head = pageHead(t('ai.set.title'));
  if (phase === 'panel') return h('div', { class: 'page page-ais' }, head, servicePanel());
  const s = settings;
  const f = form;
  if (!s || !f) {
    let body: Child;
    if (phase === 'settings' && loadError) {
      body = h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.err', { reason: errorText(loadError) })),
        h('div', { class: 'acts' }, h('button', { class: 'btn', type: 'button', onclick: () => void checkService(true) }, t('common.retry'))));
    } else {
      body = h('p', { class: 'dim', role: 'status' }, t('common.loading'));
    }
    return h('div', { class: 'page page-ais' }, head, body);
  }
  const dirty = isDirty(s, f);
  return h('div', { class: 'page page-ais' },
    head,
    statusBanner(s),
    connectionSection(s, f),
    modelSection(s, dirty),
    testSection(s, dirty),
    usersSection(s),
    usageSection(),
    retentionSection(),
    limitsSection());
}
