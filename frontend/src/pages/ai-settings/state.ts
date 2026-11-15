// AI settings page: the state every section shares (one object, so the sections can live in their own files), a few helpers,
// and the hooks the entry file fills in (which breaks the import cycle between the entry and the sections). Typed keys live
// only in this state: they are forgotten after a successful save, when the page is left and on sign-out.
import type { UsagePeriod } from '../../api/ai-admin.ts';
import { jobTime } from '../../domain/ai-settings.ts';
import { parsePeriod } from '../../domain/usage-bars.ts';
import type { ConnForm, LimitKey, LimitsForm, SettingsField, SvcCause } from '../../domain/ai-settings.ts';
import { localeTag, t } from '../../i18n/index.ts';
import { render } from '../../state/actions.ts';
import { S } from '../../state/store.ts';
import type {
  AiAdminUserRow, AiLimitsAnswer, AiModelList, AiService, AiSettings, AiSlot, AiUsage, GenAdminJob,
} from '../../types/api.ts';

const PERIOD_KEY = 'xexam.ais.usagePeriod';

/** v1.4: the remembered usage period (an unknown or missing value is 7 days) */
function readPeriod(): UsagePeriod {
  try {
    return parsePeriod(localStorage.getItem(PERIOD_KEY));
  } catch {
    return 7;
  }
}

export function writePeriod(p: UsagePeriod): void {
  try {
    localStorage.setItem(PERIOD_KEY, String(p));
  } catch {
    // not remembered
  }
}

/** While the status panel is shown, the page checks again this often */
export const RECHECK_MS = 10_000;

/** One line of feedback under a control: green when it worked, red when it did not */
export interface Msg { ok: boolean; text: string }

export interface PageState {
  settings: AiSettings | null;
  loadError: unknown;
  loading: boolean;
  wanted: boolean;
  /** What the page shows: the settings (ai-service connected), the status panel, or nothing yet (the first check runs) */
  phase: 'checking' | 'settings' | 'panel';
  svc: AiService | null;
  cause: Exclude<SvcCause, 'connected'> | null;
  svcBusy: boolean;
  svcTimer: number | null;
  panelSince: number;
  failedAs: 'notRunning' | 'notConnected' | null;
  lastDownAt: number;
  /** ai-service says this account is not an administrator (demoted meanwhile): "Not allowed" */
  forbidden: boolean;

  // ---- connections: which card's form is open ('new', a connection id or none), and the save in progress
  open: 'new' | number | null;
  form: ConnForm | null;
  fieldErr: Partial<Record<SettingsField, string>>;
  connBusy: boolean;
  connSecs: number;
  connTimer: number | null;
  connMsg: Msg | null;
  /** Retest per connection id: busy, or the last result line */
  retest: Record<number, { busy: boolean; msg: Msg | null }>;

  // ---- models and the AI+ switch
  models: AiModelList | null;
  modelsBusy: boolean;
  modelsError: string | null;
  /** The model chosen for the first AI+ switch-on (a `connectionId|model` key) */
  enableChoice: string;
  enableBusy: boolean;
  enableMsg: Msg | null;
  slotBusy: Partial<Record<AiSlot, boolean>>;
  slotMsg: Partial<Record<AiSlot, Msg | null>>;
  /** The slot whose "type a model name" row is open */
  manualOpen: AiSlot | null;
  featBusy: number | null;
  switchBusy: boolean;
  switchMsg: Msg | null;

  // ---- users, usage, limits, AI banks
  users: AiAdminUserRow[] | null;
  usersError: unknown;
  /** The user whose AI+ feature is being paused or resumed (the buttons wait meanwhile) */
  fnBusy: number | null;
  usage: AiUsage | null;
  /** Usage period: days, or '12h' / 'today' */
  usageDays: UsagePeriod;
  usageError: unknown;
  retentionText: string;
  retentionError: string | null;
  retentionSaving: boolean;
  limits: AiLimitsAnswer | null;
  limitsForm: LimitsForm | null;
  limitsError: unknown;
  limitsFieldErr: { key: LimitKey; text: string } | null;
  limitsSaveError: string | null;
  limitsSaving: boolean;
  limitsTab: 'safety' | 'gen';
  genJobs: GenAdminJob[] | null;
  genJobsError: unknown;
  featErr: Map<number, string>;
  featSaved: string;
  shareBusy: boolean;
}

function fresh(): PageState {
  return {
    settings: null, loadError: null, loading: false, wanted: true, phase: 'checking', svc: null, cause: null, svcBusy: false,
    svcTimer: null, panelSince: 0, failedAs: null, lastDownAt: 0, forbidden: false,
    open: null, form: null, fieldErr: {}, connBusy: false, connSecs: 0, connTimer: null, connMsg: null, retest: {},
    models: null, modelsBusy: false, modelsError: null, enableChoice: '', enableBusy: false, enableMsg: null, slotBusy: {}, slotMsg: {},
    manualOpen: null, featBusy: null, switchBusy: false, switchMsg: null,
    users: null, usersError: null, fnBusy: null, usage: null, usageDays: readPeriod(), usageError: null,
    retentionText: '', retentionError: null, retentionSaving: false,
    limits: null, limitsForm: null, limitsError: null, limitsFieldErr: null, limitsSaveError: null, limitsSaving: false, limitsTab: 'safety',
    genJobs: null, genJobsError: null, featErr: new Map(), featSaved: '', shareBusy: false,
  };
}

/** The one state object of the page */
export const A: PageState = fresh();

/** Forgets everything (sign-out, another account) */
export function resetState(): void {
  if (A.svcTimer != null) window.clearTimeout(A.svcTimer);
  if (A.connTimer != null) window.clearInterval(A.connTimer);
  Object.assign(A, fresh());
}

/** Filled in by the entry file: loads the settings, switches the tab, redraws the shell of the page */
export const hooks: { load: () => Promise<void>; selectTab: (k: 'feature' | 'users' | 'usage' | 'safety' | 'gen') => void } = {
  load: async () => {},
  selectTab: () => {},
};

export const onPage = (): boolean => S.route.name === 'aiSettings' && !S.authView && S.phase === 'ready';
/** Redraws the page when it is the one on screen */
export function redraw(): void {
  if (onPage()) render();
}

export const num = (n: number): string => n.toLocaleString(localeTag());
export const when = (sec: number | null): string => (sec == null ? t('common.none') : jobTime(sec));

/** The comma of a list of names, per language */
export const listSep = (lang: string): string => (lang === 'en' ? ', ' : '、');

/** Takes saved settings into the page; the retention field follows unless it holds something typed */
export function adopt(s: AiSettings): void {
  const before = A.settings;
  A.settings = s;
  if (!A.retentionText || (before && A.retentionText === String(before.retentionDays))) A.retentionText = String(s.retentionDays);
}
