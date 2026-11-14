// AI settings (#/ai-settings, v0.8, administrators only; main.ts shows "Not allowed" to everyone else). The page always
// renders: it first reads the state of ai-service from the MAIN service (GET /api/auth/state, `aiService`, from ai-service's
// reports) and loads the settings only while ai-service is connected. Otherwise it shows a status panel that names the
// cause (main service down, the AI link off, ai-service not running / stopped / not connected / no report yet; one probe of
// GET /api/ai/status tells "not running" from "not connected") and checks again every 10 s. Sections, in order:
// v1.0 (FB-6): ONE page, four category tabs under the status line (the hash keeps the tab: #/ai-settings?tab=feature | users |
// usage | safety; a missing or unknown value is the first): AI+ 功能 (0 to 3), AI+ 用户 (4), AI 用量 (5), AI 安全 (6 and 7).
//   0. the status line: one sentence while everything is fine, else the four chips (connection, model, test, AI on / off),
//      the connection to ai-service, and a key problem; always above the tabs;
//   1. Connection (a one-line summary with a pencil once it is saved and tested, AS-4): provider (OpenAI-compatible API, Anthropic, Cloudflare AI Gateway), address, keys. Key fields are always
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
//   5. Usage: calls and tokens by user (with today's count), by model, by feature and over time (no chart library: CSS bars);
//   6. Chat retention: days after the last message;
//   7. Safety limits: the per-user AI request caps, the answer and memory lengths (in characters), and the limits that protect
//      the server.
// Typed keys live only in this module's state and are forgotten after a successful save, when the page is left and on
// sign-out.
import '../styles/ai-settings.css';
import {
  clearAiData, enableAi, fetchAiSettings, fetchAiStatus, fetchAiUser, fetchAiUserChat, fetchAiUsers, fetchLimits, fetchUsage,
  listModels, resumeAiUser, saveAiSettings, saveLimits, setAiAdminFree, setAiUserFeatures, suspendAiUser, verifyAi,
} from '../api/ai.ts';
import { fetchAuthState } from '../api/auth.ts';
import { ApiError, isSignInError } from '../api/client.ts';
import { aiBadge } from '../components/ai-badge.ts';
import { markdownEl } from '../components/ai-render.ts';
import { pageHead } from '../components/common.ts';
import { field } from '../components/fields.ts';
import { pagedTable, type PtColumn } from '../components/paged-table.ts';
import { miniIcon, type MiniIcon } from '../components/mini-icons.ts';
import { miniSwitch } from '../components/mini-switch.ts';
import { iconButton } from '../components/icon-button.ts';
import { roleName } from '../components/shell.ts';
import { holdTip, infoTip } from '../components/tooltip.ts';
import { usageBarCard, usageLineCard } from '../components/usage-charts.ts';
import { formatBig, modelRows, purposeRows } from '../domain/usage-bars.ts';
import { fmtAiTime, footerOf } from '../domain/ai-chat.ts';
import { chatTitleEl, chatTitleText, userBody, whoLine } from './ai.ts';
import { renderForbidden } from './forbidden.ts';
import {
  AI_TABS, aiServiceOf, allFine, barPercent, blockTag, callsFigures, canEnable, changesFingerprint, connCollapsed, connUpdate, convTotals,
  DEFAULT_LIMITS, defaultUrl, hostOf, modelCollapsed,
  fieldOfError, fieldsOf, filterModels, FORMATS, formOf, GATEWAY_AUTHS, GATEWAY_PLACEHOLDER, hashOfTab, headerClash, httpWarning,
  inferFormat, isDirty, keyAfter, LIMIT_KEYS, CAP_KEYS, connectionSaved, focusAfterClear, limitIssue, limitsFormOf,
  limitsUpdate, modelKind, needsReentry, orderModels, PROVIDERS, rangeOf, requiredKeysSaved, retentionOf,
  savedToastKey, serviceCause, SERVER_KEYS, statusChips, tabOfHash, testLineOf, todayCell, urlAfterSwitch, type AiTab, type ConnForm,
  type LimitKey, type LimitRanges, type LimitsForm, type LimitValues, type ModelKind, type SettingsField, type SvcCause, type SvcProbe,
  AS_KEYS, BANK_KEYS, compBad, compTotal, genCallLive,
  jobTime,
} from '../domain/ai-settings.ts';
import { ALL_ON, FEATURE_GROUPS, offList, readFeatures, type FeatureKey } from '../domain/ai-features.ts';
import { badgeState, serviceReason } from '../domain/badge-state.ts';
import { aiCodeText, errorText, jobReasonText } from '../i18n/errors.ts';
import { localeTag, t, tn, uiLang, type MsgKey } from '../i18n/index.ts';
import { clockText, waitText } from '../i18n/wait.ts';
import { onRouteChange, onSignedIn, refreshAi, render } from '../state/actions.ts';
import { setAiFreeMode } from '../state/ai.ts';
import { S, onClearUserData } from '../state/store.ts';
import type {
  AiAdminUserDetail, AiAdminUserRow, AiChatDetail, AiKeySlot, AiLimitsAnswer, AiMessage, AiModels, AiProvider, AiService,
  AiSettings, AiUsage, AiVerify, GenAdminJob,
} from '../types/api.ts';
import { fetchAdminGen } from '../api/ai-gen.ts';
import { ask, dialogTitleId } from '../utils/dialog.ts';
import { h, scrollToY, type Child } from '../utils/dom.ts';
import { matches, PHONE_MQ, TOUCH_MQ } from '../utils/media.ts';
import { toast } from '../utils/toast.ts';

const PROVIDER_LABEL: Record<AiProvider, MsgKey> = {
  openai: 'ai.set.provider.openai', anthropic: 'ai.set.provider.anthropic', gateway: 'ai.set.provider.gateway',
};
const GATEWAY_AUTH_LABEL: Record<string, string> = {
  bearer: 'Authorization: Bearer', 'x-api-key': 'x-api-key', 'cf-aig-authorization': 'cf-aig-authorization',
};
const LIMIT_LABEL: Record<LimitKey, MsgKey> = {
  callsPerMinute: 'ai.set.limit.callsPerMinute', callsPerHour: 'ai.set.limit.callsPerHour', callsPerDay: 'ai.set.limit.callsPerDay',
  replyMaxTokens: 'ai.set.limit.replyMaxTokens', memoryTokens: 'ai.set.limit.memoryTokens',
  perUserRunning: 'ai.set.limit.perUserRunning', globalRunning: 'ai.set.limit.globalRunning', chatsPerUser: 'ai.set.limit.chatsPerUser',
  messagesPerChat: 'ai.set.limit.messagesPerChat', memoriesPerUser: 'ai.set.limit.memoriesPerUser',
  genPerDay: 'ai.set.limit.genPerDay', genAdminsExempt: 'ai.set.limit.genAdminsExempt', genMinSource: 'ai.set.limit.genMinSource',
  genMinWrong: 'ai.set.limit.genMinWrong', genBankSingle: 'ai.set.limit.genSingle', genBankMulti: 'ai.set.limit.genMulti',
  genWrongSingle: 'ai.set.limit.genSingle', genWrongMulti: 'ai.set.limit.genMulti', genCallTokens: 'ai.set.limit.genCallTokens',
  genRunning: 'ai.set.limit.genRunning', importMaxMb: 'ai.set.limit.importMaxMb', importMaxQuestions: 'ai.set.limit.importMaxQuestions',
  genMaxCalls: 'ai.set.limit.genMaxCalls', importMaxCalls: 'ai.set.limit.importMaxCalls', recognizeMaxCalls: 'ai.set.limit.recognizeMaxCalls',
  genUnlimited: 'ai.set.limit.unlimited', importUnlimited: 'ai.set.limit.unlimited',
};
/** The category tabs (FB-6, AS-9): the icon, and the full and the short label (the short one under PHONE) */
const TABS: Record<AiTab, { icon: MiniIcon; full: MsgKey; short: MsgKey }> = {
  feature: { icon: 'plug', full: 'ai.set.tab.feature', short: 'ai.set.tab.featureShort' },
  users: { icon: 'users', full: 'ai.set.tab.users', short: 'ai.set.tab.usersShort' },
  usage: { icon: 'chart', full: 'ai.set.tab.usage', short: 'ai.set.tab.usageShort' },
  safety: { icon: 'shield', full: 'ai.set.tab.safety', short: 'ai.set.tab.safetyShort' },
  gen: { icon: 'aiGen', full: 'ai.set.tab.gen', short: 'ai.set.tab.genShort' },
};
const PERIODS = [7, 30, 90, 365] as const;
/** The six features of the usage chart (UC-1), in the order of the bars */
const PURPOSES = ['chat', 'hint', 'explain', 'summary', 'generate', 'import'] as const;
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
/** AS-4: the connection / the model are shown as one-line summaries once saved and tested; the pencil opens them */
let editConn = false;
let editModel = false;

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
let limitsFieldErr: { key: LimitKey; text: string } | null = null;
let limitsSaveError: string | null = null;
let limitsSaving = false;
/** v1.1: the recent AI bank jobs of the AI banks tab (GS-2 item 5) and the saving of a user's feature boxes (FP-4) */
let genJobs: GenAdminJob[] | null = null;
let genJobsError: unknown = null;
let featBusy: number | null = null;
const featErr = new Map<number, string>();
let featSaved = '';

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
  editConn = false;
  editModel = false;
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
    editConn = false;
    editModel = false;
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
    void loadGenJobs();
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

async function loadGenJobs(): Promise<void> {
  try {
    genJobs = await fetchAdminGen(50);
    genJobsError = null;
  } catch (e) {
    if (aiDown(e)) return;
    genJobsError = e;
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
    editConn = false;
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
    editModel = false;
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
  editConn = false;
  editModel = false;
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
    retentionError = null;
    render();
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

/** The text under a limit field: the allowed range, or (for the caps) that a longer period cannot allow fewer requests */
function limitFieldText(key: LimitKey, kind: 'range' | 'order' | 'comp', ranges: LimitRanges): string {
  if (kind === 'order') return t('ai.err.limitOrder');
  if (kind === 'comp') return t('ai.set.gen.totalBad');
  const [min, max] = rangeOf(ranges, key);
  return t('ai.set.limitBad', { min: num(min), max: num(max) });
}

/** The tab whose Save is running (the toast differs) */
let limitsTab: 'safety' | 'gen' = 'safety';

async function saveLimitsForm(tab: 'safety' | 'gen' = 'safety'): Promise<void> {
  limitsTab = tab;
  const l = limits;
  const f = limitsForm;
  if (!l || !f || limitsSaving) return;
  const issue = limitIssue(f, l.ranges);
  if (issue) {
    limitsFieldErr = { key: issue.key, text: limitFieldText(issue.key, issue.kind, l.ranges) };
    render();
    const bad = document.getElementById(`ai-lim-${issue.key}`);
    if (bad) {
      // No default focus scroll: the pinned top bar would cover the field
      bad.focus({ preventScroll: true });
      bad.scrollIntoView({ block: 'center' });
    }
    return;
  }
  const u = limitsUpdate(l.limits, f);
  if (!Object.keys(u).length) {
    limitsFieldErr = null;
    limitsSaveError = null;
    render();
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
    toast(t(limitsTab === 'gen' ? 'ai.set.gen.saved' : 'ai.set.limitsSaved'));
    // The administrator's own block follows the new caps at once (C-7): force a status read, not the cached one
    void refreshAi(true);
    // The counts of the users (and their "today / cap" figures) are measured against the new caps
    void loadUsers();
    void loadUsage();
    void loadGenJobs();
  } catch (e) {
    // The service names the field of a refused value; an order problem without a field belongs under the hourly one
    const named = e instanceof ApiError ? (LIMIT_KEYS as readonly string[]).find((k) => k === e.field) as LimitKey | undefined : undefined;
    const order = e instanceof ApiError && e.code === 'ai.limitOrder';
    const comp = e instanceof ApiError && e.code === 'ai.genCount';
    const key = named ?? (order ? 'callsPerHour' : undefined);
    if (key) {
      limitsFieldErr = { key, text: limitFieldText(key, comp ? 'comp' : order ? 'order' : 'range', l.ranges) };
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

/** A saved and tested part as one line with a pencil (AS-4) */
function summarySection(id: string, head: MsgKey, text: string, editLabel: string, editId: string, onEdit: () => void): HTMLElement {
  const btn = iconButton({ icon: 'pencil', label: editLabel, onClick: onEdit });
  btn.id = editId;
  return h('section', { class: 'panel ais-sec ais-sum', 'aria-labelledby': id },
    h('h2', { id }, t(head)),
    h('p', { class: 'ais-sumtext' }, text),
    btn);
}

function connectionPart(s: AiSettings, f: ConnForm, dirty: boolean): HTMLElement {
  if (!connCollapsed(s, dirty, editConn)) return connectionSection(s, f);
  return summarySection('ais-conn-h', 'ai.set.sec.connection', `${t(PROVIDER_LABEL[s.provider])} · ${hostOf(s.baseUrl)}`,
    t('ai.set.sum.editConn'), 'ai-conn-edit', () => {
      editConn = true;
      render();
      document.getElementById('ai-url')?.focus();
    });
}

function modelPart(s: AiSettings, dirty: boolean): HTMLElement {
  if (!modelCollapsed(s, modelInput, editModel)) return modelSection(s, dirty);
  return summarySection('ais-model-h', 'ai.set.sec.model', s.model, t('ai.set.sum.editModel'), 'ai-model-edit', () => {
    editModel = true;
    // The picker shows the full list (AS-5); a model-list call is not counted against the caps
    if (!models && !modelsBusy) void detectModels();
    render();
    document.getElementById('ai-model')?.focus();
  });
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
      h('span', null, t(PROVIDER_LABEL[p])), miniIcon('check', 'ais-seg-ck')))));

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
    if (show) {
      // Above the field when less than 200 px are free below it (the on-screen keyboard counts: the visual viewport)
      const vv = window.visualViewport;
      const bottom = vv ? vv.offsetTop + vv.height : window.innerHeight;
      listEl.classList.toggle('up', bottom - input.getBoundingClientRect().bottom < 200);
    }
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
      // On touch the field goes near the top of the visual viewport (below the top bar), so the list and the keyboard fit
      if (matches(TOUCH_MQ)) {
        const nudge = (): void => {
          if (!input.isConnected) return;
          const top = (window.visualViewport?.offsetTop ?? 0) + (parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--top-h')) || 0) + 12;
          const delta = input.getBoundingClientRect().top - top;
          if (Math.abs(delta) > 4) scrollToY(window.scrollY + delta, false);
        };
        nudge();
        window.setTimeout(nudge, 300);
      }
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
function badgeFor(u: Pick<AiAdminUserRow, 'role' | 'ai' | 'suspended' | 'features'>, enabled: boolean): HTMLElement | null {
  const why = serviceReason(S.aiService);
  const off = u.role === 'admin' ? [] : offList(readFeatures(u.features));
  return aiBadge(badgeState({
    role: u.role, service: u.ai, suspended: u.suspended === true, available: enabled, ...(why ? { unavailableReason: why } : {}),
  }), { admin: true, off });
}

/** The text of a user's cap tag when tapped: who, which cap, and when the user can use AI again */
function blockedText(name: string, q: { cap: 'minute' | 'hour' | 'day'; limit: number; retryAt: number }): string {
  const nowSec = Date.now() / 1000;
  const cap = t(q.cap === 'minute' ? 'ai.cap.minute' : q.cap === 'hour' ? 'ai.cap.hour' : 'ai.cap.day', { n: num(q.limit) });
  const left = Math.max(1, Math.ceil(q.retryAt - nowSec));
  const when = q.cap === 'day'
    ? t('ai.set.calls.whenDay')
    : left < 60 ? t('ai.set.calls.whenSoon') : t('ai.set.calls.whenWait', { wait: waitText(left), time: clockText(q.retryAt, nowSec) });
  return t('ai.set.calls.blocked', { name, cap, when });
}

/**
 * C-7: "AI requests 1 min 2/5 · 1 h 7/20 · today 37/100" and, while a cap blocks, a warn tag ("Used up · back at 14:05" /
 * "Used up today") that shows the cap text when tapped. The label in front is only for the card layout (the table has the
 * column name).
 */
function callsCell(u: AiAdminUserRow): HTMLElement {
  const td = h('td', { role: 'cell', class: 'ais-calls', 'data-l': t('ai.set.col.calls') });
  const fig = u.exists ? callsFigures(u.calls, limits?.limits) : null;
  if (!fig || !u.calls) return td;
  td.append(h('span', { class: 'ais-callsline' },
    h('span', { class: 'ais-calls-lbl' }, `${t('ai.set.calls.lbl')} `),
    ...[t('ai.set.calls.min', { v: fig.minute }), t('ai.set.calls.hour', { v: fig.hour }), t('ai.set.calls.today', { v: fig.day })]
      .flatMap((x, i): Child[] => [i ? ' · ' : '', h('span', { class: 'ais-calls-fig' }, x)])));
  const tag = blockTag(u.calls);
  const q = u.calls.blocked;
  if (tag && q) {
    td.append(h('button', {
      type: 'button', class: 'ais-blocktag',
      onclick: () => toast(blockedText(u.username, q), 7000),
    }, t(tag.kind === 'today' ? 'ai.set.calls.tagToday' : 'ai.set.calls.tagTimed', { time: clockText(tag.retryAt, Date.now() / 1000) })));
  }
  return td;
}

/** The feature boxes of one user (FP-4): two groups, each on one row; administrators' are ticked and disabled */
function featuresLine(u: AiAdminUserRow): HTMLElement {
  const admin = u.role === 'admin';
  const f = admin ? ALL_ON : readFeatures(u.features);
  const busy = featBusy === u.id;
  const box = (k: FeatureKey): HTMLElement => {
    const full = t(`ai.feature.${k}` as MsgKey);
    const input = h('input', {
      type: 'checkbox', class: 'ais-fbox', id: `ais-feat-${u.id}-${k}`, checked: f[k], disabled: admin || busy,
      'aria-label': `${full}: ${u.username}`, onchange: () => void setFeature(u, k, input.checked),
    });
    return h('label', { class: 'ais-fl', title: full }, input, h('span', null, t(`ai.feature.${k}.short` as MsgKey)));
  };
  const colon = uiLang() === 'en' ? ':' : '：';
  const err = featErr.get(u.id);
  return h('div', { class: 'ais-feats', role: 'group', 'aria-label': `${t('ai.set.feat.title')}: ${u.username}`, title: admin ? t('ai.set.feat.adminAll') : null },
    h('span', { class: 'ais-feat-t' }, t('ai.set.feat.title')),
    h('div', { class: 'ais-fg' }, FEATURE_GROUPS.main.map(box)),
    h('div', { class: 'ais-fg ais-fg-banks' }, h('span', { class: 'ais-fg-l' }, t('ai.feature.group.banks') + colon), FEATURE_GROUPS.banks.map(box)),
    admin ? h('span', { class: 'sr-only' }, t('ai.set.feat.adminAll')) : null,
    admin ? freeSwitch(u) : null,
    err ? h('p', { class: 'ais-ferr ais-feat-err', role: 'alert' }, err) : null);
}

/** A-OWNER-21: an administrator row's AI Chat free mode switch (ordinary users never get one); saved at once */
function freeSwitch(u: AiAdminUserRow): HTMLElement {
  const on = u.freeMode === true;
  const input = h('input', {
    type: 'checkbox', role: 'switch', class: 'ais-fbox', id: `ais-free-${u.id}`, checked: on, disabled: featBusy != null,
    'aria-label': `${t('ai.set.free')}: ${u.username}`, onchange: () => void setFree(u, input.checked),
  });
  return h('div', { class: 'ais-fg ais-fg-free' }, h('label', { class: 'ais-fl', title: t('ai.set.free.tip') }, input, h('span', null, t('ai.set.free'))));
}

async function setFree(u: AiAdminUserRow, on: boolean): Promise<void> {
  if (featBusy != null) return;
  featBusy = u.id;
  featErr.delete(u.id);
  render();
  try {
    const row = await setAiAdminFree(u.id, on);
    if (users) users = users.map((x) => (x.id === u.id ? { ...x, ...row } : x));
    featSaved = t('ai.set.free.saved', { name: u.username, state: t(row.freeMode ? 'settings.on' : 'settings.off') });
    if (u.username === S.user?.username) setAiFreeMode(row.freeMode === true);
  } catch (e) {
    if (!aiDown(e)) featErr.set(u.id, t('ai.set.feat.failed', { reason: errorText(e) }));
  }
  featBusy = null;
  if (onPage()) render();
  document.getElementById(`ais-free-${u.id}`)?.focus();
}

/** One box changed: all six values go to the server at once; a failure puts the box back and shows the error under the line */
async function setFeature(u: AiAdminUserRow, key: FeatureKey, on: boolean): Promise<void> {
  if (featBusy != null) return;
  const next = { ...readFeatures(u.features), [key]: on };
  featBusy = u.id;
  featErr.delete(u.id);
  render();
  try {
    const row = await setAiUserFeatures(u.id, next);
    if (users) users = users.map((x) => (x.id === u.id ? { ...x, ...row } : x));
    featSaved = t('ai.set.feat.saved', { name: u.username });
    void refreshAi(true);
  } catch (e) {
    if (!aiDown(e)) featErr.set(u.id, t('ai.set.feat.failed', { reason: errorText(e) }));
  }
  featBusy = null;
  if (onPage()) render();
  document.getElementById(`ais-feat-${u.id}-${key}`)?.focus();
}

/**
 * The users with AI+ (AS-2). Wider than 900 px a table (the name wraps, the role tag next to a short "Always on" tag, the
 * calls line, a narrow last column of icon buttons); up to 900 px every row is a card of three lines: name, badge and the
 * eye; role, state and the pause / play button; the counts line and the calls line. The cells of the card's counts line are
 * the same cells as the table's counts, so the stylesheet shows one or the other.
 */
function usersSection(s: AiSettings): HTMLElement {
  let body: Child;
  if (!users) {
    body = usersError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.usersErr', { reason: errorText(usersError) })))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  } else if (!users.length) {
    body = h('p', { class: 'ais-note' }, t('ai.set.user.noUsers'));
  } else {
    // Explicit table roles: up to 900 px the CSS turns the rows into cards (display: grid), which would drop the table
    // semantics in some browsers. `data-l` is the column name shown in front of a value inside a card.
    const cell = (label: MsgKey, cls: string, ...kids: Child[]): HTMLElement => h('td', { role: 'cell', class: cls || null, 'data-l': t(label) }, ...kids);
    const row = (u: AiAdminUserRow): HTMLElement => {
      const off = u.suspended === true;
      const view = iconButton({ icon: 'eye', label: t('ai.set.user.viewAria', { name: u.username }), onClick: () => void openViewer(u) });
      view.id = `ais-view-${u.id}`;
      view.classList.add('ais-bview');
      // Administrators are never paused; a deleted user has nothing to pause
      let fn: HTMLButtonElement | null = null;
      if (u.exists && u.role !== 'admin') {
        fn = iconButton({
          icon: off ? 'playCircle' : 'pauseCircle', label: t(off ? 'ai.set.fn.resumeAria' : 'ai.set.fn.suspendAria', { name: u.username }),
          onClick: () => void setFunction(u, !off),
        });
        fn.id = `ais-fn-${u.id}`;
        fn.classList.add('ais-bfn');
        fn.disabled = fnBusy != null;
      }
      const role = u.exists
        ? [h('span', { class: 'tag k' }, roleName(u.role)),
          u.role === 'admin' ? h('span', { class: 'tag g', title: t('ai.set.user.adminAlways') }, t('ai.set.user.alwaysTag')) : null]
        : [];
      const state = u.exists && u.role !== 'admin'
        ? h('span', { class: 'ais-fn-state ' + (off ? 'off' : 'on') }, t(off ? 'ai.set.fn.suspended' : 'ai.set.fn.active'))
        : null;
      return h('tr', { role: 'row', class: u.exists ? null : 'gone' },
        h('th', { scope: 'row', role: 'rowheader', class: 'ais-user' }, h('span', { class: 'ais-uname', title: u.username }, u.username)),
        cell('ai.set.col.badge', 'ais-badge', u.exists ? badgeFor(u, s.enabled) : h('span', { class: 'dim' }, t('ai.set.user.deleted'))),
        cell('ai.set.col.role', 'ais-role', role),
        cell('ai.set.col.function', 'ais-state', state),
        cell('ai.set.user.chats', 'n ais-w', num(u.chats)), cell('ai.set.user.memories', 'n ais-w', num(u.memories)),
        cell('ai.set.user.lastUsed', 'ais-when ais-w', when(u.lastUsedAt)),
        h('td', { role: 'cell', class: 'ais-cline' }, t('ai.set.user.line', { c: num(u.chats), m: num(u.memories), last: when(u.lastUsedAt) })),
        callsCell(u),
        h('td', { role: 'cell', class: 'ais-acts' }, view, fn));
    };
    const c = (key: string, label: MsgKey, cls = '', text = false): PtColumn => ({ key, label: t(label), cls, text });
    const plain = (key: string, label: MsgKey, cls = ''): PtColumn => ({ key, label: t(label), cls, plain: true });
    const numCmp = (f: (r: AiAdminUserRow) => number) => (a: AiAdminUserRow, b: AiAdminUserRow): number => f(a) - f(b);
    body = pagedTable<AiAdminUserRow>({
      id: 'ai-users', rows: users, name: (u) => u.username, cardsAt: 900, roles: true, tableClass: 'ais-table ais-users',
      defaultSort: { key: 'user', dir: 'asc' },
      // The name and the AI+ badge in columns of their own, so names of any length line up (owner's request)
      columns: [
        c('user', 'ai.set.col.user', '', true), plain('badge', 'ai.set.col.badge'), c('role', 'ai.set.col.role', '', true), plain('fn', 'ai.set.col.function'),
        c('chats', 'ai.set.user.chats', 'n ais-w'), c('memories', 'ai.set.user.memories', 'n ais-w'), c('last', 'ai.set.user.lastUsed', 'ais-w'),
        { key: 'line', label: `${t('ai.set.user.chats')} · ${t('ai.set.user.memories')} · ${t('ai.set.user.lastUsed')}`, cls: 'ais-cline', plain: true, srOnly: true },
        plain('calls', 'ai.set.col.calls'),
        { key: 'acts', label: t('ai.set.user.view'), cls: 'ais-acts', plain: true, srOnly: true },
      ],
      cmp: {
        user: (a, b) => a.username.localeCompare(b.username, localeTag()),
        role: (a, b) => roleName(a.role).localeCompare(roleName(b.role), localeTag()),
        chats: numCmp((u) => u.chats), memories: numCmp((u) => u.memories), last: numCmp((u) => u.lastUsedAt ?? 0),
      },
      renderRow: (u) => [row(u), u.exists
        ? h('tr', { role: 'row', class: 'ais-featrow' }, h('td', { role: 'cell', colspan: '10' }, featuresLine(u)))
        : null],
    });
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-users-h' },
    h('h2', { id: 'ais-users-h' }, t('ai.set.sec.users')),
    // the polite text after a features change (no toast, FP-4)
    h('p', { class: 'ais-note ais-feat-saved', role: 'status', 'aria-live': 'polite' }, featSaved),
    body);
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
  // PHONE: a full-screen sheet with a back chevron, the title and ×; the delete buttons become a trash icon (the open chat)
  // and a "Clear…" menu (all chats, all memories). Wider screens keep the dialog with its buttons at the bottom.
  let menuOpen = false;
  const phoneMq = typeof window.matchMedia === 'function' ? window.matchMedia(PHONE_MQ) : null;
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

  const backToList = (): void => {
    const was = chatId;
    chatId = null;
    chat = null;
    draw();
    if (was != null) document.getElementById(`vw-c-${was}`)?.focus();
  };

  function draw(): void {
    const dt = detail;
    const phone = matches(PHONE_MQ);
    const closeBtn = iconButton({ icon: 'x', label: t('common.close'), onClick: () => d.close() });
    closeBtn.classList.add('vw-close');
    const openChat1 = phone && dt != null && vtab === 'chats' && chatId != null;
    let back: HTMLButtonElement | null = null;
    if (openChat1) {
      back = iconButton({ icon: 'back', label: t('ai.back'), onClick: backToList });
      back.id = 'vw-back';
    }
    const tools: Child[] = [];
    if (phone && dt && (dt.chats.length || dt.memories.length)) {
      if (openChat1) {
        const trash = iconButton({ icon: 'trash', tone: 'danger', label: t('ai.set.user.clearChat'), onClick: () => void clear('chat') });
        trash.disabled = busy;
        tools.push(trash);
      }
      const more = iconButton({ icon: 'more', label: t('ai.set.user.clearMenu'), onClick: () => {
        menuOpen = !menuOpen;
        draw();
        document.getElementById('vw-more')?.focus();
      } });
      more.id = 'vw-more';
      more.classList.add('vw-more');
      more.setAttribute('aria-haspopup', 'menu');
      more.setAttribute('aria-expanded', String(menuOpen));
      more.disabled = busy;
      tools.push(more);
    }
    const head = h('div', { class: 'vw-head' },
      back,
      h('h3', { id: titleId }, t('ai.set.user.title', { name: u.username })),
      phone ? null : h('span', { class: 'tag k' }, t('ai.set.user.readOnly')),
      ...tools,
      closeBtn);
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
    // The "Clear…" menu (PHONE): the two bulk deletes, in red text; each still asks first
    const menu = phone && menuOpen
      ? h('div', { class: 'vw-menu', role: 'menu', 'aria-label': t('ai.set.user.clearMenu') },
        dt.chats.length ? h('button', { type: 'button', role: 'menuitem', class: 'vw-menu-i', onclick: () => {
          menuOpen = false;
          void clear('chats');
        } }, t('ai.set.user.clearChats')) : null,
        dt.memories.length ? h('button', { type: 'button', role: 'menuitem', class: 'vw-menu-i', onclick: () => {
          menuOpen = false;
          void clear('memories');
        } }, t('ai.set.user.clearMemories')) : null)
      : null;
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
          // The back control is in the header on PHONE; below 720 px (not PHONE) it stays above the chat
          phone ? null : h('button', { class: 'btn quiet vw-back', id: 'vw-back', type: 'button', onclick: backToList }, miniIcon('back', 'ai-mi'), t('ai.back')),
          pane));
    }
    d.replaceChildren(head,
      ...(menu ? [menu] : []),
      h('div', { class: 'seg vw-tabs', role: 'group' },
        tabBtn('chats', `${t('ai.set.user.chats')} (${num(dt.chats.length)})`),
        tabBtn('memories', `${t('ai.set.user.memories')} (${num(dt.memories.length)})`)),
      h('div', { class: 'vw-body' }, body),
      // PHONE has no bottom row (the duplicate Close and the red buttons moved to the header)
      ...(phone ? [] : [h('div', { class: 'acts vw-acts' },
        vtab === 'chats' && chatId != null
          ? h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void clear('chat') }, t('ai.set.user.clearChat'))
          : null,
        dt.chats.length ? h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void clear('chats') }, t('ai.set.user.clearChats')) : null,
        dt.memories.length ? h('button', { class: 'btn danger', type: 'button', disabled: busy, onclick: () => void clear('memories') }, t('ai.set.user.clearMemories')) : null,
        h('button', { class: 'btn', type: 'button', onclick: () => d.close() }, t('common.close')))]));
  }

  // A turn of the phone between portrait and landscape (or a resize across 600 px) draws the other layout
  const onMq = (): void => {
    menuOpen = false;
    draw();
  };
  phoneMq?.addEventListener('change', onMq);
  // A tap outside the menu closes it; so does Escape (before it closes the sheet)
  d.addEventListener('click', (e) => {
    if (menuOpen && !(e.target as Element | null)?.closest('.vw-menu, .vw-more')) {
      menuOpen = false;
      draw();
    }
  });
  d.addEventListener('cancel', (e) => {
    if (!menuOpen) return;
    e.preventDefault();
    menuOpen = false;
    draw();
    document.getElementById('vw-more')?.focus();
  });
  d.addEventListener('close', () => {
    phoneMq?.removeEventListener('change', onMq);
    d.remove();
    document.getElementById(`ais-view-${u.id}`)?.focus();
  });
  draw();
  document.body.append(d);
  if (typeof d.showModal === 'function') d.showModal();
  else d.setAttribute('open', '');
  d.querySelector<HTMLElement>('.vw-close')?.focus();
  await loadDetail();
}

// ---------------------------------------------------------------- usage

type Counts = { calls: number; tokensIn: number; tokensOut: number; errors: number; tokensCached: number; tokensCacheWrite?: number };

/** A-OWNER-84: "Cache write/read: 6.9K/76.3K" (the same pair as the live token text; missing figures count as 0) */
function cacheText(c: { tokensCached?: number; tokensCacheWrite?: number }): string {
  return t('ai.set.cacheLine', { w: formatBig(c.tokensCacheWrite ?? 0), r: formatBig(c.tokensCached ?? 0) });
}

/** The five count cells of a usage row (hidden in the card layout, where `sumCell` says the same in one line) */
function countsCells(c: Counts, mini = false): HTMLElement[] {
  // The mini table of phones (A-OWNER-59) shows compact figures (1.2K, 3.4M); the exact number stays in the tip and the wide table
  const cell = (n: number): HTMLElement => (mini
    ? h('td', { class: 'n ais-c', title: num(n) }, h('span', { class: 'ais-x' }, num(n)), h('span', { class: 'ais-k', 'aria-label': num(n) }, formatBig(n)))
    : h('td', { class: 'n ais-c' }, num(n)));
  return [cell(c.calls), cell(c.tokensIn), cell(c.tokensOut), cell(c.tokensCached), cell(c.errors)];
}

const countsHead = (): HTMLElement[] => [
  h('th', { scope: 'col', class: 'n ais-c' }, t('ai.set.calls')), h('th', { scope: 'col', class: 'n ais-c' }, t('ai.set.tokensIn')),
  h('th', { scope: 'col', class: 'n ais-c' }, t('ai.set.tokensOut')), h('th', { scope: 'col', class: 'n ais-c' }, t('ai.set.tokensCached')),
  h('th', { scope: 'col', class: 'n ais-c' }, t('ai.set.errors')),
];

/** AS-3, up to 560 px: "Calls 17 · Tokens 2,631 / 367" (cached and errors only when not zero; today's count when given) */
function sumCell(c: Counts, today?: string): HTMLElement {
  return h('td', { class: 'ais-usum' },
    t('ai.set.usage.sum', { calls: num(c.calls), tin: num(c.tokensIn), tout: num(c.tokensOut) }),
    c.tokensCached || c.tokensCacheWrite ? t('ai.set.usage.sumCached', { w: formatBig(c.tokensCacheWrite ?? 0), r: formatBig(c.tokensCached ?? 0) }) : '',
    c.errors ? t('ai.set.usage.sumErr', { n: num(c.errors) }) : '',
    today ? t('ai.set.usage.sumToday', { v: today }) : '');
}

type UsageUser = AiUsage['byUser'][number];

/** v1.1 (L-54): the "by user" table with a search box, sortable columns and 10 users per page */
function usageUsersTable(rows: UsageUser[], perDay: number | undefined): HTMLElement {
  const nameOf = (r: UsageUser): string => (r.userId == null ? t('ai.set.deletedUsers') : r.username ?? `#${r.userId}`);
  const numCmp = (f: (r: UsageUser) => number) => (a: UsageUser, b: UsageUser): number => f(a) - f(b);
  const col = (key: string, label: MsgKey, cls: string, text = false): PtColumn => ({ key, label: t(label), cls, text });
  return pagedTable<UsageUser>({
    id: 'usage-users', rows, name: nameOf, cardsAt: 'mini', tableClass: 'ais-table ais-mini',
    defaultSort: { key: 'calls', dir: 'desc' },
    columns: [
      col('user', 'ai.set.col.user', '', true), col('calls', 'ai.set.calls', 'n ais-c'), col('tin', 'ai.set.tokensIn', 'n ais-c'),
      col('tout', 'ai.set.tokensOut', 'n ais-c'), { ...col('cached', 'ai.set.tokensCached', 'n ais-c'), plain: true },
      col('errors', 'ai.set.errors', 'n ais-c'), col('today', 'ai.set.col.today', 'n ais-c'), col('last', 'ai.set.user.lastUsed', 'ais-whenh'),
    ],
    cmp: {
      user: (a, b) => nameOf(a).localeCompare(nameOf(b), localeTag()),
      calls: numCmp((r) => r.calls), tin: numCmp((r) => r.tokensIn), tout: numCmp((r) => r.tokensOut), errors: numCmp((r) => r.errors),
      today: numCmp((r) => r.today ?? -1), last: numCmp((r) => r.lastAt ?? 0),
    },
    renderRow: (r) => h('tr', null,
      h('th', { scope: 'row', title: cacheText(r) }, nameOf(r)), countsCells(r, true),
      h('td', { class: 'n ais-c ais-today' }, todayCell(r.today, perDay)),
      h('td', { class: 'ais-when' }, when(r.lastAt))),
  });
}

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
    const cacheNote = h('p', { class: 'ais-note ais-cache' }, cacheText(u.totals));
    // The conversation figures that go with the usage (the owner's "用量跟对话"): the sums over the listed AI+ users
    const conv = users && users.length ? convTotals(users) : null;
    const convs = conv
      ? h('p', { class: 'ais-note ais-convs' }, t('ai.set.usage.convs', { chats: tn('ai.set.usage.nChats', conv.chats, { n: num(conv.chats) }), messages: tn('ai.set.usage.nMessages', conv.messages, { n: num(conv.messages) }), memories: tn('ai.set.usage.nMemories', conv.memories, { n: num(conv.memories) }) }))
      : null;
    const perDay = limits?.limits.callsPerDay;
    if (!u.totals.calls) {
      body = [tiles, cacheNote, convs, h('p', { class: 'ais-note' }, t('ai.set.noUsage'))];
    } else {
      const maxCalls = Math.max(0, ...u.byPeriod.map((p) => p.calls));
      body = [
        tiles,
        cacheNote,
        convs,
        h('h3', { class: 'ais-h3' }, t('ai.set.byUser')),
        usageUsersTable(u.byUser, perDay),
        // v1.1 (UC-1, A-OWNER-39 / 40): vertical bars by model, calls or stacked tokens; the model lists are the "other" bar;
        // the numbers stay in the visually hidden table
        usageBarCard({ pref: 'model', kind: 'model', exact: num, rows: modelRows(u.byModel, t('ai.set.uc.other')) }),
        h('div', { class: 'ais-tablewrap sr-only' }, h('table', { class: 'ais-table ais-cards' },
          h('thead', null, h('tr', null, h('th', { scope: 'col' }, t('ai.set.col.model')), countsHead())),
          // The model-listing calls are stored without a model (kind 'models'): labelled like "Deleted users" above
          h('tbody', null, u.byModel.map((r) => h('tr', null,
            r.model ? h('th', { scope: 'row', class: 'ais-mono' }, r.model) : h('th', { scope: 'row' }, t('ai.set.modelListCalls')),
            countsCells(r), sumCell(r)))))),
        // v0.9: what the calls were for (AI Chat, hint, explanation, summary), in the server's order
        u.byPurpose
          ? usageBarCard({
            pref: 'purpose', kind: 'purpose', exact: num,
            rows: purposeRows(PURPOSES, u.byPurpose, u.totals, (k) => t(`ai.set.usage.purpose.${k}` as MsgKey), t('ai.set.uc.other')),
          })
          : null,
        u.byPurpose
          ? h('div', { class: 'ais-tablewrap sr-only' }, h('table', { class: 'ais-table ais-cards' },
            h('thead', null, h('tr', null, h('th', { scope: 'col' }, t('ai.set.usage.byPurpose')), countsHead())),
            h('tbody', null, u.byPurpose.map((r) => h('tr', null,
              h('th', { scope: 'row' }, t(`ai.set.usage.purpose.${r.purpose}` as MsgKey)), countsCells(r), sumCell(r))))))
          : null,
        // A-OWNER-63: the same 调用次数 | Token 数 switch as the bar charts
        usageLineCard({ unit: u.unit, from: u.from, to: u.to, rows: u.byPeriod, format: num }),
        h('div', { class: 'ais-tablewrap sr-only' }, h('table', { class: 'ais-table ais-cards ais-period' },
          h('thead', null, h('tr', null,
            h('th', { scope: 'col' }, t(u.unit === 'month' ? 'ai.set.col.month' : 'ai.set.col.day')),
            h('th', { scope: 'col', class: 'ais-barcol' }, h('span', { class: 'sr-only' }, t('ai.set.calls'))),
            countsHead())),
          h('tbody', null, u.byPeriod.map((r) => h('tr', { class: r.calls ? null : 'zero' },
            h('th', { scope: 'row', class: 'num' }, r.period),
            h('td', { class: 'ais-barcol', 'aria-hidden': 'true' }, h('span', { class: 'ais-bar' }, h('i', { style: `width:${barPercent(r.calls, maxCalls)}%` }))),
            countsCells(r), sumCell(r)))))),
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

/** One number field of the limits form (whole numbers; 16 px text on touch comes from the stylesheet) */
function limitInput(k: LimitKey, f: LimitsForm, bad: boolean, describedBy: string): HTMLInputElement {
  const [min, max] = rangeOf(limits?.ranges, k);
  const input = h('input', {
    type: 'number', id: `ai-lim-${k}`, class: 'ais-num', min: String(min), max: String(max), step: '1', inputmode: 'numeric',
    value: f[k], disabled: limitsSaving, 'aria-describedby': describedBy, 'aria-invalid': bad ? 'true' : null,
    oninput: () => {
      if (limitsForm) limitsForm = { ...limitsForm, [k]: input.value };
    },
  });
  return input;
}

/** C-7: the per-user caps, first: three compact fields in one row, one hint line and the (i) tip with the full rule */
function capsGroup(l: AiLimitsAnswer, f: LimitsForm): HTMLElement {
  const tip = infoTip('ai-caps-tip', t('ai.set.caps.tipBtn'), t('ai.set.caps.tip'));
  const bad = CAP_KEYS.find((k) => limitsFieldErr?.key === k);
  return h('div', { class: 'ais-subgroup ais-caps-group', role: 'group', 'aria-labelledby': 'ais-caps-h' },
    h('h3', { class: 'ais-h3', id: 'ais-caps-h' }, t('ai.set.caps.title'), tip.el),
    h('div', { class: 'ais-caps' }, CAP_KEYS.map((k) => {
      const [min, max] = rangeOf(l.ranges, k);
      const def = defOf(l, k);
      const input = limitInput(k, f, limitsFieldErr?.key === k, `ai-caps-hint ai-caps-tip ai-lim-${k}-range`);
      return h('div', { class: 'fld ais-cap' },
        h('label', { class: 'fld-l', for: input.id }, t(LIMIT_LABEL[k])),
        input,
        // The range and the default stay available to screen readers without taking room on the row
        h('span', { class: 'sr-only', id: `ai-lim-${k}-range` }, t('ai.set.limitHint', { min: num(min), max: num(max), def: num(def) })));
    })),
    h('p', { class: 'fld-hint ais-caps-hint', id: 'ai-caps-hint' }, t('ai.set.caps.hint')),
    bad ? h('p', { class: 'ais-ferr', role: 'alert' }, limitsFieldErr?.text ?? '') : null);
}

/** The default of a limit: the service's own when it sent one, else the spec's */
const defOf = (l: AiLimitsAnswer, k: LimitKey): number => (l.defaults as LimitValues)[k] ?? DEFAULT_LIMITS[k];

/** FB-5 (revised): the length of one answer and of one memory, in tokens, between the caps and the server limits */
function lengthGroup(l: AiLimitsAnswer, f: LimitsForm): HTMLElement {
  const tip = infoTip('ai-length-tip', t('ai.set.limits.lengthTipBtn'), t('ai.set.limits.lengthTip'));
  const field = (k: LimitKey, extra: Child[], describedBy: string): HTMLElement => {
    const [min, max] = rangeOf(l.ranges, k);
    const def = defOf(l, k);
    const bad = limitsFieldErr?.key === k;
    const input = limitInput(k, f, bad, `ai-lim-${k}-hint ${describedBy}`);
    return h('div', { class: 'fld ais-length' },
      h('label', { class: 'fld-l', for: input.id }, t(LIMIT_LABEL[k])),
      input,
      h('p', { class: 'fld-hint', id: `ai-lim-${k}-hint` }, t('ai.set.limitHint', { min: num(min), max: num(max), def: num(def) })),
      extra,
      bad ? h('p', { class: 'ais-ferr', role: 'alert' }, limitsFieldErr?.text ?? '') : null);
  };
  return h('div', { class: 'ais-subgroup ais-length-group', role: 'group', 'aria-labelledby': 'ais-length-h' },
    h('h3', { class: 'ais-h3', id: 'ais-length-h' }, t('ai.set.limits.length'), tip.el),
    h('div', { class: 'ais-lengths' },
      field('replyMaxTokens', [], 'ai-length-tip'),
      field('memoryTokens', [], 'ai-length-tip')));
}

/** Fills the defaults of `keys` into the form (not saved until Save); the other tab's fields keep what they hold */
function resetKeys(keys: readonly LimitKey[]): void {
  if (!limits || !limitsForm) return;
  const d = limitsFormOf(limits.defaults);
  const next = { ...limitsForm };
  for (const k of keys) next[k] = d[k];
  limitsForm = next;
  limitsFieldErr = null;
  render();
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
    capsGroup(l, f),
    lengthGroup(l, f),
    h('div', { class: 'ais-subgroup', role: 'group', 'aria-labelledby': 'ais-srv-h' },
      h('h3', { class: 'ais-h3', id: 'ais-srv-h' }, t('ai.set.server.title')),
      h('div', { class: 'ais-limitgrid' }, SERVER_KEYS.map((k) => {
        const [min, max] = rangeOf(l.ranges, k);
        const def = defOf(l, k);
        const bad = limitsFieldErr?.key === k;
        const input = limitInput(k, f, bad, `ai-lim-${k}-hint`);
        return h('div', { class: 'fld ais-limit' },
          h('label', { class: 'fld-l', for: input.id }, t(LIMIT_LABEL[k])),
          input,
          h('p', { class: 'fld-hint', id: `ai-lim-${k}-hint` }, t('ai.set.limitHint', { min: num(min), max: num(max), def: num(def) })),
          bad ? h('p', { class: 'ais-ferr', role: 'alert' }, limitsFieldErr?.text ?? '') : null);
      }))),
    limitsSaveError ? h('div', { class: 'err', role: 'alert' }, h('p', null, limitsSaveError)) : null,
    h('div', { class: 'acts' },
      h('button', { class: 'btn primary', id: 'ai-limits-save', type: 'submit', disabled: limitsSaving }, t(limitsSaving ? 'common.saving' : 'ai.set.save')),
      h('button', {
        class: 'btn', id: 'ai-limits-reset', type: 'button', disabled: limitsSaving,
        onclick: () => resetKeys(AS_KEYS),
      }, t('ai.set.limitsReset'))));
  }
  return h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-lim-h' },
    h('h2', { id: 'ais-lim-h' }, t('ai.set.sec.limits')),
    h('p', { class: 'ais-lead' }, t('ai.set.limitsLeadV10')),
    body);
}

// ---------------------------------------------------------------- the AI banks tab (GS-2, IS-2, IS-3)

/** A-OWNER-50: either "unlimited" switch also lifts the answer length per call (a shared box), so it greys with them */
function syncCallLength(): void {
  const el = document.getElementById('ai-lim-genCallTokens') as HTMLInputElement | null;
  if (el && limitsForm) el.disabled = limitsSaving || limitsForm.genUnlimited === '1' || limitsForm.importUnlimited === '1';
}

/** One numeric field of the AI banks tab: label, input, the range / hint line and its error */
function genField(k: LimitKey, l: AiLimitsAnswer, f: LimitsForm, hintKey: MsgKey | null, extra: Child[] = [], unlimitedKey?: LimitKey, showSwitch = true): HTMLElement {
  const [min, max] = rangeOf(l.ranges, k);
  const def = defOf(l, k);
  const bad = limitsFieldErr?.key === k;
  const input = limitInput(k, f, bad, `ai-lim-${k}-hint`);
  if (k === 'genCallTokens' && (f.genUnlimited === '1' || f.importUnlimited === '1')) input.disabled = true;
  // A-OWNER-45: the "unlimited" switch greys the number box (the value is kept)
  let sw: HTMLElement | null = null;
  if (unlimitedKey) {
    const uk = unlimitedKey;
    const on = f[uk] === '1';
    input.disabled = limitsSaving || on;
    if (showSwitch) {
    const box = h('input', {
      type: 'checkbox', id: `ai-lim-${uk}`, checked: on, disabled: limitsSaving,
      onchange: () => {
        if (limitsForm) limitsForm = { ...limitsForm, [uk]: box.checked ? '1' : '0' };
        input.disabled = limitsSaving || box.checked;
        syncCallLength();
        // The import switch also covers the recognition box
        if (uk === 'importUnlimited') {
          const rec = document.getElementById('ai-lim-recognizeMaxCalls') as HTMLInputElement | null;
          if (rec) rec.disabled = limitsSaving || box.checked;
        }
      },
    });
    sw = h('label', { class: 'ais-check', for: box.id }, box, h('span', null, t('ai.set.limit.unlimited')));
    }
  }
  return h('div', { class: 'fld ais-genfld' },
    h('label', { class: 'fld-l', for: input.id }, t(LIMIT_LABEL[k])),
    sw ? h('div', { class: 'ais-numrow' }, input, sw) : input,
    h('p', { class: 'fld-hint', id: `ai-lim-${k}-hint` }, hintKey ? t(hintKey) + ' ' : '', t('ai.set.limitHint', { min: num(min), max: num(max), def: num(def) })),
    unlimitedKey && showSwitch ? h('p', { class: 'fld-hint' }, t('ai.set.limit.unlimitedHint')) : null,
    extra,
    bad && limitsFieldErr?.text ? h('p', { class: 'ais-ferr', role: 'alert' }, limitsFieldErr.text) : null);
}

/** `单选 [14] 多选 [6] · 共 20 道` of one mode, the total live (red with the rule's text outside 1 .. 100) */
function compRow(mode: 'bank' | 'wrong', l: AiLimitsAnswer, f: LimitsForm): HTMLElement {
  const sk: LimitKey = mode === 'bank' ? 'genBankSingle' : 'genWrongSingle';
  const mk: LimitKey = mode === 'bank' ? 'genBankMulti' : 'genWrongMulti';
  const sIn = limitInput(sk, f, limitsFieldErr?.key === sk, `ai-comp-${mode}-h`);
  const mIn = limitInput(mk, f, limitsFieldErr?.key === mk, `ai-comp-${mode}-h`);
  sIn.setAttribute('aria-label', t('ai.set.limit.genSingle'));
  mIn.setAttribute('aria-label', t('ai.set.limit.genMulti'));
  const total = h('span', { class: 'ais-comp-total', id: `ai-comp-total-${mode}`, 'aria-live': 'polite' });
  const sync = (): void => {
    const v = limitsForm ?? f;
    const bad = compBad(v[sk], v[mk]);
    const n = compTotal(v[sk], v[mk]);
    const el = document.getElementById(`ai-comp-total-${mode}`) ?? total;
    el.classList.toggle('bad', bad);
    el.textContent = Number.isNaN(n) ? t('ai.set.gen.totalBad') : bad ? `${t('ai.set.gen.total', { n })} · ${t('ai.set.gen.totalBad')}` : t('ai.set.gen.total', { n });
  };
  for (const input of [sIn, mIn]) input.addEventListener('input', sync);
  sync();
  void l;
  return h('div', { class: 'ais-comp', role: 'group', 'aria-labelledby': `ai-comp-${mode}-h` },
    h('div', { class: 'ais-comp-head' },
      h('span', { class: 'ais-comp-l', id: `ai-comp-${mode}-h` }, t(mode === 'bank' ? 'ai.set.gen.compBank' : 'ai.set.gen.compWrong')),
      h('span', { class: 'fld-hint ais-comp-hint' }, t(mode === 'bank' ? 'ai.set.gen.compBankHint' : 'ai.set.gen.compWrongHint'))),
    h('div', { class: 'ais-comp-fields' },
      // A-OWNER-87: short visible labels keep the row on one line; the full names stay the inputs' accessible names
      h('label', { class: 'ais-comp-f', for: sIn.id }, h('span', { 'aria-hidden': 'true' }, t('ai.set.limit.genSingleShort')), sIn),
      h('label', { class: 'ais-comp-f', for: mIn.id }, h('span', { 'aria-hidden': 'true' }, t('ai.set.limit.genMultiShort')), mIn),
      total));
}

function genJobsTable(): HTMLElement {
  if (!genJobs) {
    return genJobsError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, errorText(genJobsError)))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading'));
  }
  if (!genJobs.length) return h('p', { class: 'ais-note' }, t('ai.set.gen.jobsEmpty'));
  const col = (k: MsgKey, cls = ''): HTMLElement => h('th', { scope: 'col', class: cls || null }, t(k));
  const state = (j: GenAdminJob): string =>
    t(`ai.set.gen.state.${j.state}` as MsgKey) + (j.state === 'running' && (j.phase === 'reading' || j.phase === 'confirm') ? ` · ${t(`ai.set.gen.phase.${j.phase}` as MsgKey)}` : '');
  const source = (j: GenAdminJob): string =>
    j.source === 'file' ? t('ai.set.gen.source.file', { type: (j.file?.type ?? '').toUpperCase() }) : t(`ai.set.gen.source.${j.source}` as MsgKey);
  const big = (n: number): string => formatBig(n);
  const rows = genJobs.map((j) => {
    const why = j.state === 'failed' || (j.state === 'cancelled' && j.error) ? jobReasonText(j) : null;
    const whyEl = why ? holdTip(h('span', { class: 'ais-genwhy', tabindex: '0' }, h('span', { class: 'ais-d' }, ' — '), why), why) : null;
    const cr = j.usage?.tokensCached ?? 0;
    const cw = j.usage?.tokensCacheWrite ?? 0;
    const tip = t('ai.set.gen.stats.tip', { calls: num(j.calls), tin: num(j.tokensIn), tout: num(j.tokensOut) })
      + (cr > 0 || cw > 0 ? t('ai.set.usage.sumCached', { w: num(cw), r: num(cr) }) : '');
    // L-55: icons carry screen-reader names; the whole figure line has one tip (hover / focus / hold)
    const stats = holdTip(h('span', { class: 'ais-stats', tabindex: '0' },
      h('span', { class: 'ais-st' }, miniIcon('refresh', 'ais-ji'), h('span', { class: 'sr-only' }, t('ai.set.gen.stats.calls') + ' '), num(j.calls)),
      h('span', { class: 'ais-st' }, h('span', { class: 'ais-arr', 'aria-hidden': 'true' }, '↓'), h('span', { class: 'sr-only' }, t('ai.set.gen.stats.in') + ' '), big(j.tokensIn)),
      h('span', { class: 'ais-st' }, h('span', { class: 'ais-arr', 'aria-hidden': 'true' }, '↑'), h('span', { class: 'sr-only' }, t('ai.set.gen.stats.out') + ' '), big(j.tokensOut))), tip);
    const nameText = j.state === 'done' ? j.name : (j.file?.name ?? j.name);
    const skipText = j.skipped && j.skipped.length > 0 ? t('ai.set.gen.skipped', { n: j.skipped.length, list: j.skipped.join(',') }) : null;
    // v1.2 (owner): on wide screens the failure reason and the skipped note go on one full-width line under the job
    // (like the phone card) instead of squeezing the state and count columns; phones keep them inside the card
    const detail = why || skipText
      ? h('tr', { class: 'ais-gendetail ' + j.state },
        h('td', { colspan: '7' },
          why ? h('span', { class: 'ais-genwhy-w' }, why) : null,
          skipText ? h('span', { class: 'ais-skipn-w' }, skipText) : null))
      : null;
    const row = h('tr', { class: 'ais-genjob ' + j.state + (detail ? ' has-detail' : '') },
      h('td', { class: 'ais-when', 'data-l': t('ai.set.gen.col.at') }, jobTime(j.createdAt)),
      h('th', { scope: 'row', class: 'ais-uname' }, j.username),
      h('td', { class: 'ais-gensrc', 'data-l': t('ai.set.gen.col.source') }, source(j)),
      h('td', { class: 'ais-genname', 'data-l': t('ai.set.gen.col.name') },
        h('span', { class: 'ais-srcm' }, source(j)), h('span', { class: j.state === 'done' ? 'ais-nm' : 'ais-nm dim' }, nameText)),
      h('td', { class: 'ais-genst', 'data-l': t('ai.set.gen.col.state') }, h('span', { class: 'ais-jchip ' + j.state }, state(j)), whyEl),
      h('td', { class: 'n ais-genmade', 'data-l': t('ai.set.gen.col.made') }, `${num(j.made)} / ${num(j.goal)}`,
        skipText ? h('span', { class: 'ais-skipn' }, skipText) : null),
      h('td', { class: 'n ais-gencalls', 'data-l': t('ai.set.gen.col.calls') }, stats));
    return detail ? [row, detail] : [row];
  }).flat();
  return h('div', { class: 'ais-tablewrap' }, h('table', { class: 'ais-table ais-genjobs' },
    h('thead', null, h('tr', null, col('ai.set.gen.col.at'), col('ai.set.gen.col.user'), col('ai.set.gen.col.source'), col('ai.set.gen.col.name'),
      col('ai.set.gen.col.state'), col('ai.set.gen.col.made', 'n'), col('ai.set.gen.col.calls', 'n'))),
    h('tbody', null, rows)));
}

/** The fifth tab, "AI 题库" (GS-2 + IS-2): rules, composition, length, the import card, one Save, the recent jobs */
function genSection(): HTMLElement[] {
  const l = limits;
  const f = limitsForm;
  if (!l || !f) {
    return [h('section', { class: 'panel ais-sec' }, limitsError
      ? h('div', { class: 'err', role: 'alert' }, h('p', null, t('ai.set.limitsErr', { reason: errorText(limitsError) })))
      : h('p', { class: 'dim', role: 'status' }, t('common.loading')))];
  }
  const card = (id: string, title: MsgKey, ...kids: Child[]): HTMLElement =>
    h('section', { class: 'panel ais-sec ais-gencard', 'aria-labelledby': id }, h('h2', { id }, t(title)), ...kids);
  // A sub-group inside a card (A-OWNER-49): a small heading and its fields
  const sub = (id: string, title: MsgKey, ...kids: Child[]): HTMLElement =>
    h('div', { class: 'ais-subcard', role: 'group', 'aria-labelledby': id }, h('h3', { class: 'ais-subh', id }, t(title)), ...kids);
  const exempt = f.genAdminsExempt === '1';
  const exemptBox = h('input', {
    type: 'checkbox', id: 'ai-lim-genAdminsExempt', checked: exempt, disabled: limitsSaving,
    onchange: () => {
      if (limitsForm) limitsForm = { ...limitsForm, genAdminsExempt: exemptBox.checked ? '1' : '0' };
    },
  });
  const tip = infoTip('ai-gen-tip', t('ai.set.tab.gen'), `${t('ai.set.gen.howTip')}\n\n${t('ai.set.imp.howTip')}`, 'ais-pre');
  const live = h('p', { class: 'fld-hint ais-live', id: 'ai-lim-genCallTokens-live', 'aria-live': 'polite' });
  const liveText = (text: string): string => {
    const v = genCallLive(text, l.ranges);
    return v ? t('ai.set.limit.genCallLive', { q: v.q }) : '';
  };
  live.textContent = liveText(f.genCallTokens);
  const form = h('form', {
    class: 'ais-genform', novalidate: true, 'aria-busy': limitsSaving ? 'true' : null,
    onsubmit: (e: SubmitEvent) => {
      e.preventDefault();
      void saveLimitsForm('gen');
    },
  },
  card('ais-gen-h', 'ai.set.gen.card',
    sub('ais-sub-quota', 'ai.set.sub.quota',
      h('div', { class: 'ais-genrow' },
        genField('genPerDay', l, f, 'ai.set.limit.genPerDayHint'),
        h('label', { class: 'ais-check', for: 'ai-lim-genAdminsExempt' }, exemptBox, h('span', null, t('ai.set.limit.genAdminsExempt'))))),
    sub('ais-sub-source', 'ai.set.sub.source',
      h('div', { class: 'ais-genrow' },
        genField('genMinSource', l, f, 'ai.set.limit.genMinSourceHint'),
        genField('genMinWrong', l, f, null))),
    sub('ais-sub-comp', 'ai.set.gen.comp', compRow('bank', l, f), compRow('wrong', l, f)),
    sub('ais-sub-calls', 'ai.set.sub.calls',
      genField('genCallTokens', l, f, 'ai.set.limit.genCallTokensHint', [live]),
      genField('genMaxCalls', l, f, 'ai.set.limit.genMaxCallsHint', [], 'genUnlimited'),
      genField('genRunning', l, f, null))),
  card('ais-imp-h', 'ai.set.imp.title',
    sub('ais-sub-file', 'ai.set.sub.file',
      h('div', { class: 'ais-genrow' },
        genField('importMaxMb', l, f, 'ai.set.limit.importMaxMbHint'),
        genField('importMaxQuestions', l, f, 'ai.set.limit.importMaxQuestionsHint'))),
    sub('ais-sub-impcalls', 'ai.set.sub.calls',
      genField('recognizeMaxCalls', l, f, 'ai.set.limit.recognizeMaxCallsHint', [], 'importUnlimited', false),
      genField('importMaxCalls', l, f, 'ai.set.limit.importMaxCallsHint', [], 'importUnlimited')),
    h('p', { class: 'ais-note' }, t('ai.set.imp.shared'))),
  limitsSaveError ? h('div', { class: 'err', role: 'alert' }, h('p', null, limitsSaveError)) : null,
  h('div', { class: 'acts ais-genacts' },
    h('button', { class: 'btn primary', id: 'ai-gen-save', type: 'submit', disabled: limitsSaving }, t(limitsSaving ? 'common.saving' : 'ai.set.save')),
    h('button', { class: 'btn', id: 'ai-gen-reset', type: 'button', disabled: limitsSaving, onclick: () => resetKeys(BANK_KEYS) }, t('ai.set.limitsReset'))));
  // the live line follows the typed length without a redraw
  form.addEventListener('input', (e) => {
    if ((e.target as HTMLElement).id === 'ai-lim-genCallTokens') live.textContent = liveText((e.target as HTMLInputElement).value);
  });
  const jobs = h('section', { class: 'panel ais-sec', 'aria-labelledby': 'ais-gen-jobs-h' },
    h('h2', { id: 'ais-gen-jobs-h' }, t('ai.set.gen.jobs')), genJobsTable());
  return [h('div', { class: 'ais-gentitle' }, h('h2', { class: 'ais-gen-h' }, t('ai.set.tab.gen')), tip.el), form, jobs];
}

// ---------------------------------------------------------------- the page

function statusBanner(s: AiSettings): HTMLElement {
  // Connection, model and test each with its own chip (owner's request), then whether AI is on
  const c = statusChips(s, testResult != null && !testResult.ok);
  const problem = s.keyProblem === 'mismatch' ? 'ai.set.keyProblem' : s.keyProblem === 'invalidEnv' ? 'ai.set.keyEnv' : s.keyProblem === 'invalidFile' ? 'ai.set.keyFile' : null;
  const chip = (cls: string, key: MsgKey): HTMLElement => h('span', { class: 'ais-chip ' + cls }, t(key));
  // Everything fine: one calm sentence; the four chips only when something is not OK (AS-4)
  const fine = allFine(c, problem != null);
  return h('div', { class: 'ais-status', role: 'status' },
    ...(fine
      ? [h('span', { class: 'ais-chip ok ais-allok' }, t('ai.set.allOk'))]
      : [chip(c.conn === 'ok' ? 'ok' : 'none', c.conn === 'ok' ? 'ai.set.chip.connOk' : 'ai.set.chip.connNone'),
        chip(c.model === 'ok' ? 'ok' : 'none', c.model === 'ok' ? 'ai.set.chip.modelOk' : 'ai.set.chip.modelNone'),
        chip(c.test === 'ok' ? 'ok' : c.test === 'fail' ? 'fail' : 'unverified',
          c.test === 'ok' ? 'ai.set.chip.testOk' : c.test === 'fail' ? 'ai.set.chip.testFail' : 'ai.set.chip.testNone'),
        chip(c.ai === 'on' ? 'on' : 'none', c.ai === 'on' ? 'ai.set.state.on' : 'ai.set.state.off')]),
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

/** The row of category tabs under the status line (FB-6): icon + label, short labels under PHONE, one row */
function tabsRow(tab: AiTab): HTMLElement {
  return h('div', { class: 'seg ais-tabs', role: 'group', 'aria-label': t('ai.set.tabs.aria') },
    AI_TABS.map((k) => {
      const d = TABS[k];
      const b = h('button', {
        type: 'button', id: `ais-tab-${k}`, 'aria-pressed': String(k === tab), 'aria-controls': 'ais-card', onclick: () => selectTab(k),
      }, miniIcon(d.icon, 'ais-tab-ic'), h('span', { class: 'ais-tl-full' }, t(d.full)), h('span', { class: 'ais-tl-short', 'aria-hidden': 'true' }, t(d.short)));
      // The full name is the accessible name and, by hold (touch) or hover, the tip
      holdTip(b, t(d.full));
      return b;
    }));
}

/** Shows a category: the hash keeps it (`#/ai-settings?tab=users`), replaced in place, so Back does not walk through tabs */
function selectTab(k: AiTab): void {
  if (tabOfHash(window.location.hash) === k && window.location.hash.includes('tab=')) return;
  try {
    window.history.replaceState(window.history.state, '', hashOfTab(k));
  } catch {
    window.location.hash = hashOfTab(k);
  }
  render();
  document.getElementById(`ais-tab-${k}`)?.focus();
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
  const tab = tabOfHash(window.location.hash);
  // Only the shown category is in the DOM; the status line stays above in every tab
  let card: Child[];
  switch (tab) {
    case 'users':
      card = [usersSection(s)];
      break;
    case 'usage':
      card = [usageSection()];
      break;
    case 'safety':
      card = [retentionSection(), limitsSection()];
      break;
    case 'gen':
      card = genSection();
      break;
    default:
      card = [connectionPart(s, f, dirty), modelPart(s, dirty), testSection(s, dirty)];
  }
  return h('div', { class: 'page page-ais' },
    head,
    statusBanner(s),
    tabsRow(tab),
    h('section', { class: 'ais-card', id: 'ais-card', 'aria-labelledby': `ais-tab-${tab}` }, card));
}
