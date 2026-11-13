// AI-generated banks (v1.1, section 5): who sees which control and in which state (GB-1), the names (GB-4 / GP-9 twins of
// the Rust helpers), the progress card model (GB-7), the AI+题集 tip choice (GB-6 / IM-8). Pure: no DOM, no i18n calls
// (it runs under `node --test`); message keys are returned as strings and the components translate them.
import type { BankAi, GenJob, GenStatus } from '../types/api.ts';
import type { ControlView, UnavailableReason } from './ai-features.ts';

// ---------------------------------------------------------------- names (GP-9 twins)

export const NAME_MAX_HOME = 60;
export const NAME_MAX_BANK = 120;

/** Invisible format characters (GP-9): bidi marks / overrides / isolates, zero-width space, word joiner, BOM (U+200C / U+200D stay) */
export function isHiddenFormat(ch: string): boolean {
  const c = ch.codePointAt(0) ?? 0;
  return c === 0x061c || c === 0x200b || c === 0x200e || c === 0x200f || (c >= 0x202a && c <= 0x202e) || (c >= 0x2060 && c <= 0x2069) || c === 0xfeff;
}

const CONTROL = /\p{Cc}/u;

/** Drops control and invisible format characters (the `strip` of GP-9) */
export function stripName(s: string): string {
  return [...s].filter((c) => !CONTROL.test(c) && !isHiddenFormat(c)).join('');
}

export type NameCheck = { ok: true; name: string } | { ok: false; error: 'empty' | 'long' | 'bad' };

/** `trim` like Rust's: JavaScript's also strips U+FEFF, which the server treats as a hidden character (refused, not trimmed) */
const trimLikeRust = (s: string): string => s.replace(/^[^\S\uFEFF]+|[^\S\uFEFF]+$/g, '');

/**
 * GP-9 `check_name`: trimmed, 1 .. 60 characters (Unicode scalar values, never UTF-16 units), no control or invisible format
 * characters. Checked in the order empty, long, bad.
 */
export function checkName(raw: string): NameCheck {
  const name = trimLikeRust(raw);
  const len = [...name].length;
  if (len === 0) return { ok: false, error: 'empty' };
  if (len > NAME_MAX_HOME) return { ok: false, error: 'long' };
  if ([...name].some((c) => CONTROL.test(c) || isHiddenFormat(c))) return { ok: false, error: 'bad' };
  return { ok: true, name };
}

/** GP-9 `bank_mode_name`: `<source>_<first 8 characters of the user name>_<at>`, the whole within 120 characters (the source is cut) */
export function bankModeName(source: string, username: string, at: string): string {
  const user = [...stripName(username)].slice(0, 8).join('');
  const suffix = `_${user}_${at}`;
  const room = Math.max(0, NAME_MAX_BANK - [...suffix].length);
  const head = [...stripName(source).trim()].slice(0, room).join('');
  return head + suffix;
}

/** The suggested name of a Home generation: `MM-DD HH:mm` of the browser's local time (GB-3) */
export function stampText(now: Date): string {
  const p = (n: number): string => String(n).padStart(2, '0');
  return `${p(now.getMonth() + 1)}-${p(now.getDate())} ${p(now.getHours())}:${p(now.getMinutes())}`;
}

/** The `YYYY-MM-DD HH:mm` of a server time `YYYY-MM-DD HH:mm:ss` (the bank's `ai.at`); unchanged when it does not match */
export function minuteText(at: string): string {
  const m = /^(\d{4}-\d{2}-\d{2} \d{2}:\d{2})(?::\d{2})?$/.exec(at);
  return m ? m[1]! : at;
}

// ---------------------------------------------------------------- who sees what (GB-1)

export type GenMode = 'bank' | 'scope' | 'wrong';

/** The generation's own reasons (greyed without the 无法使用 tag), in the order they are checked */
export type GenOwnReason = 'busy' | 'quota' | 'noScope' | 'aiSource' | 'fewSource' | 'fewWrong';
export type GenReason = 'paused' | UnavailableReason | 'featureOff' | GenOwnReason;

export interface GenViewCtx {
  mode: GenMode;
  /** Home: the number of checked sheets of non-AI banks (0 = nothing usable checked) */
  checkedSheets?: number;
  /** Home: every checked sheet belongs to an AI bank */
  onlyAi?: boolean;
  /** Home: AI banks are among the checked ones (they are skipped) */
  skippedAi?: boolean;
  /** A-OWNER-52b: ANY AI bank among the checked sheets blocks generation (every mode) */
  hasAi?: boolean;
  /** Bank management: the bank itself is an AI bank (never a source) */
  aiBank?: boolean;
  /** Active single / multiple-choice questions (bank mode, scope) or such questions in the wrong-answer book (wrong mode) */
  have: number;
}

export type GenViewState =
  | { show: false }
  | { show: true; usable: true; need: number; have: number }
  | {
    show: true; usable: false; reason: GenReason;
    /** The 无法使用 tag: service problems and the feature switch (SH-17) */
    tag: boolean;
    /** The catalog key of the reason text, and its parameters */
    key: string;
    params: Record<string, number>;
    /** The sentence `aigen.why.aiSkipped` follows the reason (AI banks were among the checked ones) */
    skippedNote: boolean;
  };

const SERVICE_KEY: Record<'paused' | UnavailableReason | 'featureOff', string> = {
  paused: 'ai.badge.tip.suspended', off: 'ai.badge.tip.off', down: 'ai.badge.tip.down', unavailable: 'ai.badge.unavailable',
  loading: 'aiplus.why.loading', featureOff: 'ai.err.featureOff',
};

/** `genView` of GB-1: `view` is the M-7 decision of the `generate` feature (bank management needed) */
export function genView(view: ControlView, gen: GenStatus | null, ctx: GenViewCtx): GenViewState {
  if (view.view === 'hidden' || gen == null) return { show: false };
  const none = { params: {}, skippedNote: false };
  if (view.view === 'paused') return { show: true, usable: false, reason: 'paused', tag: true, key: SERVICE_KEY.paused, ...none };
  if (view.view === 'unavailable') {
    return { show: true, usable: false, reason: view.reason, tag: view.reason !== 'loading', key: SERVICE_KEY[view.reason], ...none };
  }
  if (view.view === 'featureOff') return { show: true, usable: false, reason: 'featureOff', tag: true, key: SERVICE_KEY.featureOff, ...none };
  const own = (reason: GenOwnReason, key: string, params: Record<string, number> = {}, skippedNote = false): GenViewState =>
    ({ show: true, usable: false, reason, tag: false, key, params, skippedNote });
  if (gen.job && (gen.job.state === 'queued' || gen.job.state === 'running')) return own('busy', 'aigen.why.busy');
  if (!gen.unlimited && gen.left === 0) {
    return gen.perDay === 0 ? own('quota', 'aigen.why.quotaZero') : own('quota', 'aigen.why.quota', { limit: gen.perDay ?? 0 });
  }
  if (ctx.mode !== 'bank') {
    if (ctx.hasAi) return own('aiSource', 'aigen.why.aiInScope');
    if ((ctx.checkedSheets ?? 0) === 0 && !ctx.onlyAi) return own('noScope', 'aigen.why.noScope');
    if (ctx.onlyAi) return own('aiSource', 'aigen.why.aiSource');
  } else if (ctx.aiBank) {
    return own('aiSource', 'aigen.why.aiSource');
  }
  const need = ctx.mode === 'wrong' ? gen.minWrong : gen.minSource;
  if (ctx.have < need) {
    return own(ctx.mode === 'wrong' ? 'fewWrong' : 'fewSource', ctx.mode === 'wrong' ? 'aigen.why.fewWrong' : 'aigen.why.fewSource',
      { need, have: ctx.have }, ctx.skippedAi === true);
  }
  return { show: true, usable: true, need, have: ctx.have };
}

/** The number of questions one generation writes in a mode (bank / scope use the bank composition, wrong the wrong one) */
export function genCount(gen: GenStatus, mode: GenMode): number {
  const c = mode === 'wrong' ? gen.wrong : gen.bank;
  return c.single + c.multi;
}

// ---------------------------------------------------------------- the AI+题集 tip (GB-6 / IM-8)

export interface AiSetTip {
  key: string;
  /** Parameters for the catalog text; `time` is `YYYY-MM-DD HH:mm`; name / file only when present */
  params: { name?: string; file?: string; time: string };
}

/** Which tip text a bank's `ai` object gets (the second line `aigen.badge.tipCheck` follows in the component) */
export function aiSetTip(ai: BankAi): AiSetTip {
  const params: AiSetTip['params'] = { time: minuteText(ai.at) };
  const named = ai.author != null && ai.author !== '';
  if (named) params.name = ai.author!;
  if (ai.source === 'file') {
    if (ai.file != null) params.file = ai.file;
    const base = ai.mode === 'material' ? 'aigen.badge.tipFileMaterial' : 'aigen.badge.tipFile';
    return { key: named ? base : `${base}NoAuthor`, params };
  }
  return { key: named ? 'aigen.badge.tip' : 'aigen.badge.tipNoAuthor', params };
}

/** The facts line of an AI bank in Bank management (GB-2, P-16): key and parameters */
export function aiMetaLine(ai: BankAi): { key: string; params: Record<string, string> } {
  if (ai.source === 'file') {
    return { key: ai.mode === 'material' ? 'aigen.bank.metaFileMaterial' : 'aigen.bank.metaFile', params: { file: ai.file ?? '' } };
  }
  if (ai.source === 'bank') return { key: 'aigen.bank.metaBank', params: { from: ai.from[0] ?? '' } };
  return { key: ai.source === 'wrong' ? 'aigen.bank.metaWrong' : 'aigen.bank.metaScope', params: {} };
}

// ---------------------------------------------------------------- the progress card (GB-7)

export type CardPhase = 'queued' | 'writing' | 'saving';

/** What the progress card of a running job shows: the status line key and whether the bar is determinate */
export function runningLine(job: GenJob): { key: 'aigen.card.queued' | 'aigen.card.writing' | 'aigen.card.saving'; pct: number } {
  const pct = job.goal > 0 ? Math.min(100, Math.max(0, Math.round((job.made / job.goal) * 100))) : 0;
  if (job.state === 'queued') return { key: 'aigen.card.queued', pct: 0 };
  if (job.phase === 'saving') return { key: 'aigen.card.saving', pct: 100 };
  return { key: 'aigen.card.writing', pct };
}

/** The active job (queued / running), or null */
export const activeJob = (gen: GenStatus | null): GenJob | null =>
  gen?.job && (gen.job.state === 'queued' || gen.job.state === 'running') ? gen.job : null;

/**
 * The cards of the status (GB-7): the active job first, then the unseen finished ones (newest first, at most 3; a cancelled
 * job only when it is an expired import, P-18). Never twice the same id.
 */
export function cardJobs(gen: GenStatus | null): GenJob[] {
  if (!gen) return [];
  const out: GenJob[] = [];
  const seen = new Set<number>();
  const add = (j: GenJob): void => {
    if (!seen.has(j.id)) {
      seen.add(j.id);
      out.push(j);
    }
  };
  const act = activeJob(gen);
  if (act) add(act);
  for (const j of gen.unseen.slice(0, 3)) {
    if (j.state === 'cancelled' && j.error !== 'ai.import.expired') continue;
    add(j);
  }
  return out;
}

/** Polling pace (GB-7 / P-18): 2 s on Home / Bank management with a visible page, else 10 s; never on the quiz; null = stop */
export function pollInterval(route: string, visible: boolean, job: GenJob | null): number | null {
  if (!job) return null;
  if (route === 'quiz') return null;
  const active = job.state === 'queued' || job.state === 'running';
  if (!active) return null;
  if (job.phase === 'confirm') return 10_000;
  return (route === 'home' || route === 'banks') && visible ? 2_000 : 10_000;
}

/** A job that went from active to a finished state between two statuses (the toast / refresh moment); null when none did */
export function finishedBetween(prev: GenJob | null, next: GenStatus | null): GenJob | null {
  if (!prev || (prev.state !== 'queued' && prev.state !== 'running')) return null;
  const found = next ? [...next.unseen, ...(next.job ? [next.job] : [])].find((j) => j.id === prev.id) : undefined;
  return found && (found.state === 'done' || found.state === 'failed') ? found : null;
}

/** The AI+题集 marker data of a bank id (A-OWNER-27): the bank's `ai` field, null for an ordinary or unknown bank */
export function aiOfBank(banks: ReadonlyArray<{ id: number; ai?: BankAi }>, id: number): BankAi | null {
  return banks.find((b) => b.id === id)?.ai ?? null;
}

/** A-OWNER-53d: the catalog key of the word after "AI+" by origin: 错题本 mode, AI file import, or original banks (scope / Bank management) */
export function aiSetWordKey(ai: BankAi): 'aigen.badge.wordWrong' | 'aigen.badge.wordImport' | 'aigen.badge.wordPractice' {
  return ai.source === 'wrong' ? 'aigen.badge.wordWrong' : ai.source === 'file' ? 'aigen.badge.wordImport' : 'aigen.badge.wordPractice';
}
