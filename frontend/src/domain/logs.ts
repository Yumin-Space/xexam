// System log page (v0.7, administrators only): pure rules, no DOM and no network, so they are tested on their own (logs.test.ts).
//   · the backend keeps the newest 2000 log events in memory; GET /api/logs?after=<seq> answers the events after `seq`
//     ({"lines", "last", "dropped", "boot"}); the page keeps the same 2000 at most, in sequence order;
//   · "boot" is a random id of the backend process: the sequence numbers start again at 1 after a restart, so the page sends
//     the boot it last saw (?boot=) and starts again from the answer when it changed (the backend then answers with every
//     line it keeps, not only the lines above a number that belonged to the previous process);
//   · "dropped": events between the last one the page has and the oldest one the backend still keeps were lost (the page was
//     paused or hidden for long); the page marks the gap;
//   · the level filter (All / Warnings / Errors) and the text search only change what is shown, never what is kept;
//   · the poll loop asks every 0.5 seconds while the page is open, visible and not paused, one request at a time (the next one
//     is scheduled only after the previous answer arrived, so a slow answer never stacks requests); after a failed read it
//     waits longer each time (1, 2, 4, 8, then every 10 seconds), so a backend that is down or refusing is not asked twice a
//     second. On the backend each request reads the session and the in-memory buffer; the only database writes on that path
//     are the session refresh (at most once an hour) and the purge of expired sessions (at most once an hour).

/** One log event as the page keeps it */
export interface LogLine {
  /** Monotonic sequence number (starts again after a backend restart) */
  seq: number;
  /** Local time with seconds, as the backend writes it */
  time: string;
  /** error, warn, info, debug or trace (the backend's spelling; other spellings are understood too) */
  level: string;
  /** The module that logged it */
  target: string;
  /** The message with its fields */
  message: string;
  /** Set on the first line after a gap: older lines between this one and the previous one were dropped */
  gapBefore?: boolean;
}

/** As many lines as the backend keeps */
export const LOG_CAP = 2000;
/** How often the page asks for new lines while it is visible (v0.7: every 0.5 seconds) */
export const POLL_MS = 500;
/** The longest wait between two reads while they keep failing */
export const POLL_MAX_MS = 10_000;

/**
 * The wait before the next read: `ms` after a good one; after `failures` failed reads in a row, twice as long for each one
 * (1, 2, 4 and 8 seconds at 0.5 s), at most `max` (and never less than `ms`).
 */
export function pollDelay(failures: number, ms = POLL_MS, max = POLL_MAX_MS): number {
  if (!(failures > 0)) return ms;
  return Math.max(ms, Math.min(max, ms * 2 ** Math.min(failures, 30)));
}

export type LevelFilter = 'all' | 'warn' | 'error';
export const LEVEL_FILTERS: readonly LevelFilter[] = ['all', 'warn', 'error'];

export function isLevelFilter(v: unknown): v is LevelFilter {
  return v === 'all' || v === 'warn' || v === 'error';
}

/** The level in a fixed form: error, warn, info, debug, trace (anything unknown counts as info) */
export type Level = 'error' | 'warn' | 'info' | 'debug' | 'trace';

export function levelOf(level: string): Level {
  const l = level.trim().toLowerCase();
  if (l === 'error' || l === 'err') return 'error';
  if (l === 'warn' || l === 'warning') return 'warn';
  if (l === 'debug') return 'debug';
  if (l === 'trace') return 'trace';
  return 'info';
}

/** Whether a line passes the level filter: Warnings shows warnings and errors, Errors only errors */
export function levelPasses(level: string, f: LevelFilter): boolean {
  if (f === 'all') return true;
  const l = levelOf(level);
  return f === 'error' ? l === 'error' : l === 'error' || l === 'warn';
}

/** Case-insensitive search in the time, level, target and message; blank matches everything */
export function searchPasses(line: LogLine, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return [line.time, line.level, line.target, line.message].some((x) => x.toLowerCase().includes(q));
}

export function linePasses(line: LogLine, f: LevelFilter, query: string): boolean {
  return levelPasses(line.level, f) && searchPasses(line, query);
}

/** A field value as text (strings as they are, everything else as JSON) */
function fieldText(v: unknown): string {
  if (typeof v === 'string') return v;
  try {
    return JSON.stringify(v) ?? String(v);
  } catch {
    return String(v);
  }
}

/**
 * One line of the backend's answer in the page's form; null when it is not usable (no sequence number). Accepts the message
 * as `message` (or `msg`) and optional `fields` (an object, written after the message as `key=value`).
 */
export function lineOf(raw: unknown): LogLine | null {
  if (typeof raw !== 'object' || raw === null) return null;
  const o = raw as Record<string, unknown>;
  const seq = o.seq;
  if (typeof seq !== 'number' || !Number.isFinite(seq)) return null;
  const str = (v: unknown): string => (typeof v === 'string' ? v : v == null ? '' : String(v));
  let message = str(o.message ?? o.msg);
  const fields = o.fields;
  if (fields && typeof fields === 'object' && !Array.isArray(fields)) {
    const parts = Object.entries(fields as Record<string, unknown>).map(([k, v]) => `${k}=${fieldText(v)}`);
    if (parts.length) message = [message, ...parts].filter(Boolean).join(' ');
  }
  return { seq, time: str(o.time), level: str(o.level) || 'info', target: str(o.target), message };
}

/** The answer of GET /api/logs, checked */
export interface LogChunk {
  lines: LogLine[];
  last: number | null;
  dropped: boolean;
  /** The id of the backend process that answered (null: not sent, as by an older backend) */
  boot: string | null;
}

export function chunkOf(raw: unknown): LogChunk {
  const o = typeof raw === 'object' && raw !== null ? (raw as Record<string, unknown>) : {};
  const lines = Array.isArray(o.lines) ? o.lines.map(lineOf).filter((x): x is LogLine => x !== null) : [];
  const last = typeof o.last === 'number' && Number.isFinite(o.last) ? o.last : null;
  const boot = typeof o.boot === 'string' && /^[A-Za-z0-9_-]{1,64}$/.test(o.boot) ? o.boot : null;
  return { lines, last, dropped: o.dropped === true, boot };
}

/** What the page keeps: the lines (oldest first, at most LOG_CAP), the last sequence number and the backend process it saw */
export interface LogState {
  lines: LogLine[];
  last: number | null;
  boot: string | null;
}

export const EMPTY_LOG: LogState = { lines: [], last: null, boot: null };

/**
 * Adds an answer to what the page keeps. Lines the page already has are ignored (a repeated answer adds nothing). `dropped`
 * marks a gap before the first new line (only when the page already had lines). Keeps the newest `cap` lines.
 * Another `boot` than the page's own, or a `last` lower than the page's own, means the backend restarted (its numbers start
 * again at 1, and it answers an unknown `boot` or an `after` it has not reached yet with every line it keeps): the page then
 * starts again from the answer, with `reset` (redraw all).
 * Returns the new state and the lines that were appended (to draw them without redrawing the list).
 */
export function mergeChunk(state: LogState, chunk: LogChunk, cap = LOG_CAP): { state: LogState; added: LogLine[]; reset: boolean } {
  const incoming = [...chunk.lines].sort((a, b) => a.seq - b.seq);
  const boot = chunk.boot ?? state.boot;
  const restarted = (state.boot != null && chunk.boot != null && chunk.boot !== state.boot)
    || (state.last != null && chunk.last != null && chunk.last < state.last);
  if (restarted) {
    const lines = incoming.length > cap ? incoming.slice(incoming.length - cap) : incoming;
    return { state: { lines, last: chunk.last, boot }, added: lines, reset: true };
  }
  const since = state.last;
  const fresh = since == null ? incoming : incoming.filter((l) => l.seq > since);
  if (fresh.length && chunk.dropped && state.lines.length) fresh[0] = { ...(fresh[0] as LogLine), gapBefore: true };
  let lines = fresh.length ? state.lines.concat(fresh) : state.lines;
  if (lines.length > cap) lines = lines.slice(lines.length - cap);
  const seen = [since, chunk.last, ...fresh.map((l) => l.seq)].filter((x): x is number => x != null);
  const last = seen.length ? Math.max(...seen) : null;
  return { state: { lines, last, boot }, added: fresh.length > cap ? fresh.slice(fresh.length - cap) : fresh, reset: false };
}

/** Whether a scroll box is at its bottom (within `slack` pixels): new lines then keep it at the bottom */
export function atBottom(scrollTop: number, clientHeight: number, scrollHeight: number, slack = 24): boolean {
  return scrollHeight - (scrollTop + clientHeight) <= slack;
}

/** What the poll loop needs from the page and the browser (injected, so the loop is tested with fake timers) */
export interface PollHost {
  /**
   * Fetches and applies one answer; the loop waits for it before it schedules the next one. Rejects when the read failed:
   * the loop then waits longer before the next one (see pollDelay).
   */
  fetch: () => Promise<void>;
  visible: () => boolean;
  setTimer: (fn: () => void, ms: number) => unknown;
  clearTimer: (id: unknown) => void;
}

export interface Poller {
  /** The page is open: poll now and then every POLL_MS (unless paused or hidden; longer after failed reads) */
  start: () => void;
  /** The page was left: no more requests */
  stop: () => void;
  /** Pause / Resume (Resume asks at once) */
  setPaused: (p: boolean) => void;
  /** The page became visible or hidden (visibilitychange) */
  visibilityChanged: () => void;
  readonly running: boolean;
  readonly paused: boolean;
  /** Failed reads in a row (0 after a good one) */
  readonly failures: number;
  /** A request is running */
  readonly busy: boolean;
}

/**
 * The poll loop of the log page: one request at a time, the next one `ms` after the previous one finished (longer after
 * failed reads, see pollDelay); none while the page is hidden, paused or left, and no timer is left behind then.
 * Resume, the page becoming visible again and the page being opened again ask at once, also after failures (the
 * administrator asked). When that happens while a request is still running, nothing is sent next to it: the loop asks
 * right after that answer instead (so a quick hide / show or leave / come back never ends the loop or doubles it).
 * A request that is still running when the loop stops is let finish, but nothing follows. The failure count goes back to
 * 0 after a good read and when the page is opened again.
 */
export function createPoller(host: PollHost, ms = POLL_MS, maxMs = POLL_MAX_MS): Poller {
  let open = false;
  let paused = false;
  let timer: unknown = null;
  let inFlight = false;
  /** Asked to read at once while a request was running: read again as soon as it finished */
  let again = false;
  let failures = 0;

  const active = (): boolean => open && !paused && host.visible();
  const clear = (): void => {
    if (timer != null) host.clearTimer(timer);
    timer = null;
  };
  const schedule = (): void => {
    clear();
    if (active()) timer = host.setTimer(run, pollDelay(failures, ms, maxMs));
  };
  async function run(): Promise<void> {
    timer = null;
    if (!active()) return;
    if (inFlight) {
      again = true;
      return;
    }
    inFlight = true;
    try {
      await host.fetch();
      failures = 0;
    } catch {
      // the page shows the error; the loop keeps trying, less often each time
      failures++;
    } finally {
      inFlight = false;
    }
    // Stopped, paused or hidden meanwhile: active() is false, so neither of these sends anything or sets a timer
    if (again) {
      again = false;
      now();
    } else {
      schedule();
    }
  }
  function now(): void {
    clear();
    if (active()) void run();
  }

  return {
    start: () => {
      if (open) return;
      open = true;
      failures = 0;
      now();
    },
    stop: () => {
      open = false;
      again = false;
      clear();
    },
    setPaused: (p) => {
      if (paused === p) return;
      paused = p;
      if (p) {
        again = false;
        clear();
      } else {
        now();
      }
    },
    visibilityChanged: () => {
      if (host.visible()) {
        now();
      } else {
        again = false;
        clear();
      }
    },
    get running() {
      return open && !paused;
    },
    get paused() {
      return paused;
    },
    get failures() {
      return failures;
    },
    get busy() {
      return inFlight;
    },
  };
}
