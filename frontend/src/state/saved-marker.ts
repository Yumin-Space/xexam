// The server-side marker of an unfinished practice / exam (v3.1), reported so the server can send reminders. Everything here
// is best effort: a lost report only means a missing reminder, so nothing in this module may disturb the quiz.
//
// The browser keeps one saved session per user. Every save of it is reported (debounced); the server drops the marker when
// the answers are submitted (POST /api/attempt), when the session is discarded here, or when the device signs out.
import type { SavedReport } from '../types/api.ts';
import { discardSaved, reportSaved } from '../api/saved.ts';
import { storageOwner } from './storage-keys.ts';

/** At most one report per minute for the same session while only the answers change (the server also throttles) */
const MIN_GAP_S = 60;

/** The calls this module makes; tests swap them */
const io = {
  report: reportSaved as (b: SavedReport, keepalive?: boolean) => Promise<void>,
  discard: discardSaved as (key: string) => Promise<void>,
  now: (): number => Date.now() / 1000,
};

/** Test hook: replace the network calls and the clock (only the given parts change) */
export function setSavedIo(part: Partial<typeof io>): void {
  Object.assign(io, part);
}

/** The newest save not sent yet (null = everything is sent) */
let pending: SavedReport | null = null;
/** The last report that went out, and when */
let last: { at: number; report: SavedReport } | null = null;
/** True while nobody is signed in (sign-out, a lost session): nothing is reported then */
let paused = false;

const same = (a: SavedReport, b: SavedReport): boolean =>
  a.key === b.key && a.kind === b.kind && a.answered === b.answered && a.total === b.total && a.startedAt === b.startedAt && a.timeLeft === b.timeLeft;

function active(): boolean {
  return !paused && storageOwner() != null;
}

function send(r: SavedReport, keepalive: boolean): void {
  last = { at: io.now(), report: r };
  pending = null;
  // A hidden page may be closing: a plain fetch could be aborted, so it always goes out with keepalive then
  const hiding = (globalThis as { document?: { visibilityState?: string } }).document?.visibilityState === 'hidden';
  void io.report(r, keepalive || hiding);
}

/**
 * A save of the session was written to this browser. Sent when the session is new to the server, when the time ran out, or
 * when something changed and the last report is at least a minute old; otherwise kept for `flushSaved`.
 */
export function noteSaved(se: SavedReport): void {
  if (!active()) return;
  const r: SavedReport = { ...se };
  pending = r;
  if (!last || last.report.key !== r.key) {
    send(r, false);
    return;
  }
  const l = last.report;
  const timeUpChanged = (l.timeLeft === 0) !== (r.timeLeft === 0);
  const progressed = l.answered !== r.answered || l.total !== r.total || l.kind !== r.kind || l.startedAt !== r.startedAt;
  if (timeUpChanged || (progressed && io.now() - last.at >= MIN_GAP_S)) send(r, false);
}

/** The page is going away (pagehide, hidden, leaving the quiz): send what is pending, with `keepalive` so it survives */
export function flushSaved(): void {
  if (!active() || !pending) return;
  if (last && same(last.report, pending)) {
    pending = null;
    return;
  }
  send(pending, true);
}

/** The saved session was thrown away here (resume card "Discard", overwritten, its questions are gone): drop its marker */
export function noteDiscarded(key: string): void {
  if (pending?.key === key) pending = null;
  if (last?.report.key === key) last = null;
  if (storageOwner() == null) return;
  void io.discard(key);
}

/** The session was submitted: the server drops its marker by itself, so only forget the local state (no resurrecting report) */
export function noteFinished(key: string): void {
  if (pending?.key === key) pending = null;
  if (last?.report.key === key) last = null;
}

/** Nobody is signed in any more: forget everything and report nothing until `resumeSaved` */
export function pauseSaved(): void {
  paused = true;
  pending = null;
  last = null;
}

/** A user signed in (or the sign-in was renewed): reports flow again, from a clean slate */
export function resumeSaved(): void {
  paused = false;
  pending = null;
  last = null;
}
