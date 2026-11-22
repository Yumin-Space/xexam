// v2.6: pure logic of the Home AI study assessment capsule: the shape of GET /api/ai/assess/latest, the run state of a
// generation that streams into the card, and which button the card offers. No DOM, no fetch.
import type { ControlState } from './ai-plus.ts';
import type { AskError } from './ai-study.ts';

/** The saved assessment of the viewer (one per user; a new one replaces it) */
export interface AssessSaved {
  text: string;
  /** The server's calendar day it was written (`YYYY-MM-DD`) */
  day: string;
  /** Unix seconds */
  createdAt: number | null;
  latestAt: string | null;
  records: number | null;
  answered: number | null;
  /** Sent to administrators only */
  model: string | null;
}

export interface AssessLatest {
  assess: AssessSaved | null;
  today: string;
  /** Successful assessments the viewer may run per day; null = no limit for this viewer (an administrator who is exempt) */
  perDay: number | null;
  /** Assessments the viewer can still run today; null = no limit */
  remaining: number | null;
  /** Today's assessment is still being written on the server (another tab, or before a reload) */
  running: boolean;
}

const obj = (v: unknown): Record<string, unknown> => (typeof v === 'object' && v !== null ? (v as Record<string, unknown>) : {});
const num = (v: unknown): number | null => (typeof v === 'number' && Number.isFinite(v) ? v : null);
const str = (v: unknown): string | null => (typeof v === 'string' && v !== '' ? v : null);

/** Reads the answer of GET /api/ai/assess/latest; null when it is not that shape. An empty text counts as no assessment. */
export function parseAssessLatest(json: unknown): AssessLatest | null {
  const o = obj(json);
  if (!('assess' in o)) return null;
  const a = obj(o.assess);
  const text = typeof a.text === 'string' ? a.text.trim() : '';
  const assess: AssessSaved | null = text
    ? {
      text,
      day: str(a.day) ?? '',
      createdAt: num(a.createdAt),
      latestAt: str(a.latestAt),
      records: num(a.records),
      answered: num(a.answered),
      model: str(a.model),
    }
    : null;
  return { assess, today: str(o.today) ?? '', perDay: num(o.perDay), remaining: num(o.remaining), running: o.running === true };
}

// ---------------------------------------------------------------- the run (a generation streaming into the card)

export type AssessRun =
  | { phase: 'idle' }
  | { phase: 'streaming'; text: string; thinking: boolean }
  /** A failed request (an HTTP refusal, no answer) or an error event of the stream; the text before it is dropped */
  | { phase: 'error'; error: AskError }
  /** The answer is complete; `text` is shown until the saved copy is read back */
  | { phase: 'done'; text: string };

export const IDLE: AssessRun = { phase: 'idle' };
export const STARTING: AssessRun = { phase: 'streaming', text: '', thinking: false };

/** One server-sent event applied to a run; events after the end of a run are ignored */
export function runReduce(run: AssessRun, ev: { event: string; data: unknown }): AssessRun {
  if (run.phase !== 'streaming') return run;
  const d = obj(ev.data);
  switch (ev.event) {
    case 'delta': {
      const text = typeof d.text === 'string' ? d.text : '';
      return text ? { phase: 'streaming', text: run.text + text, thinking: false } : run;
    }
    case 'thinking':
      return run.text === '' ? { phase: 'streaming', text: '', thinking: true } : run;
    case 'done':
      return { phase: 'done', text: run.text };
    case 'stopped':
      return IDLE;
    case 'error':
      return { phase: 'error', error: { status: 0, code: str(d.code) ?? 'ai.internal' } };
    default:
      return run;
  }
}

// ---------------------------------------------------------------- the card

export type CapsuleAction =
  | { kind: 'none' }
  | { kind: 'ask'; label: 'generate' | 'update' }
  | { kind: 'wait'; label: 'generate' | 'update'; reason: 'running' | 'daily' | 'few' };

/**
 * The button of the card. Without a usable AI+ (no badge, paused, the assessment switch off, service down) there is none and the
 * saved text stands alone. Reasons, in order: a run in progress, the day's assessments used up (remaining 0), fewer than 3 saved records.
 */
export function capsuleAction(i: { ctl: ControlState; hasSaved: boolean; remaining: number | null; perDay?: number | null; running: boolean; few: boolean }): CapsuleAction {
  if (!i.ctl.show || !i.ctl.usable) return { kind: 'none' };
  // 0 per day = the assessment is switched off: no button, only a saved text stands
  if (i.perDay === 0) return { kind: 'none' };
  const label = i.hasSaved ? 'update' : 'generate';
  if (i.running) return { kind: 'wait', label, reason: 'running' };
  if (i.remaining === 0) return { kind: 'wait', label, reason: 'daily' };
  if (i.few) return { kind: 'wait', label, reason: 'few' };
  return { kind: 'ask', label };
}

/** The card is drawn when there is text, a button, or a run to show; otherwise it is hidden */
export const capsuleVisible = (i: { hasSaved: boolean; action: CapsuleAction; run: AssessRun }): boolean =>
  i.hasSaved || i.action.kind !== 'none' || i.run.phase !== 'idle';

/** `YYYY-MM-DD` day line of the saved text: the server's day, else the browser's local day of `createdAt` */
export function assessDay(a: Pick<AssessSaved, 'day' | 'createdAt'>): string {
  if (a.day) return a.day;
  if (a.createdAt == null) return '';
  const d = new Date(a.createdAt * 1000);
  const p = (n: number): string => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** The muted meta line "N left today" when the viewer has a limit and some runs left; null otherwise */
export const remainingNote = (remaining: number | null): number | null => (remaining !== null && remaining > 0 ? remaining : null);

/** After a successful run the count goes down by one (a number only; null stays unlimited) */
export const afterRun = (remaining: number | null): number | null => (remaining === null ? null : Math.max(0, remaining - 1));
