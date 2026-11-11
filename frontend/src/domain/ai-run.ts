// The engine behind the AI popover and the summary overlay (v0.9): owns the entries of ai-study.ts, reads saved answers,
// starts and stops generations, polls a running answer every 2 s, and applies the busy flow. No DOM and no fetch: the
// transport (a `RunSpec` per request), the timers and the notifications are injected, so `node --test` drives it with fakes.
// Rules it keeps (spec 2.3, 2.5, 2.16, 7.8):
//   · opening shows a saved answer at once; only when nothing is saved does it generate;
//   · a closed window never stops a generation (nothing here is tied to a window); reopening finds the entry again;
//   · Stop = abort the fetch + stop THAT chat (never `stop {}`); the attempt becomes `stopped` at once;
//   · a busy answer is never stopped automatically: `stopAndAsk` stops the named chat and asks again once;
//   · a lost stream turns into polling of the saved route.
import {
  type Action, type AskError, type Entry, type SavedPayload, type StreamEvent,
  boundedSet, emptyEntry, MAX_ENTRIES, reduce, refreshesStatus, runningOf,
} from './ai-study.ts';

export interface RunSpec {
  /** Reads the saved answers; null = there is nothing to look up (an `at` that does not parse): generate */
  loadSaved(): Promise<SavedPayload | null>;
  /** Starts a generation; resolves when its stream ends, rejects with an AskError / any error; `on` gets every event */
  ask(signal: AbortSignal, on: (ev: StreamEvent) => void): Promise<void>;
  /** POST /api/ai/stop {chatId} */
  stopChat(chatId: number): Promise<void>;
}

export interface RunnerEnv {
  setTimer(fn: () => void, ms: number): unknown;
  clearTimer(handle: unknown): void;
  /** An entry changed (paint it if it is the open one) */
  onChange(key: string): void;
  /** Called for a refusal that makes the AI status stale (refreshesStatus): refresh it and redraw the controls */
  onRefuse(error: AskError): void;
  /** unix seconds, for the end of an answer */
  now?: () => number;
  pollMs?: number;
  maxEntries?: number;
  /** Consecutive failed polls after which polling gives up with a failure note */
  maxPollFailures?: number;
}

interface Live {
  abort: AbortController | null;
  timer: unknown;
  /** Bumped by every start / stop / clear, so a late answer of an older request is ignored */
  gen: number;
  pollFailures: number;
}

export interface Runner {
  entry(key: string): Entry | undefined;
  /** Opens: memory first, else the saved answer, else a new generation */
  open(key: string, spec: RunSpec): Promise<void>;
  /** Regenerate: a new ask whatever is saved */
  regenerate(key: string, spec: RunSpec): Promise<void>;
  /** Stop: abort the stream and stop the chat's answer */
  stop(key: string, spec: RunSpec): Promise<void>;
  /** The busy flow's button: stop the running chat (named), then ask again once */
  stopAndAsk(key: string, spec: RunSpec, runningChatId: number): Promise<void>;
  /** Forget one entry (a record deleted): stops its polling, keeps a running generation going on the server */
  drop(key: string): void;
  /** Forget every entry the predicate matches (a deleted chat): live parts are released, open windows repaint as empty */
  dropWhere(pred: (e: Entry) => boolean): void;
  /** Sign-out: aborts every stream, stops every poll, forgets everything */
  clearAll(): void;
  /** The chat id of the first entry whose key starts with `prefix` (the session's chat), or null */
  chatIdWhere(prefix: string): number | null;
  size(): number;
}

export function createRunner(env: RunnerEnv): Runner {
  const entries = new Map<string, Entry>();
  const live = new Map<string, Live>();
  const pollMs = env.pollMs ?? 2000;
  const max = env.maxEntries ?? MAX_ENTRIES;
  const maxFail = env.maxPollFailures ?? 5;
  const now = env.now ?? ((): number => Math.floor(Date.now() / 1000));

  const lv = (key: string): Live => {
    let l = live.get(key);
    if (!l) {
      l = { abort: null, timer: null, gen: 0, pollFailures: 0 };
      live.set(key, l);
    }
    return l;
  };

  const apply = (key: string, a: Action): Entry => {
    const next = reduce(entries.get(key) ?? emptyEntry(), a);
    boundedSet(entries, key, next, max);
    // An evicted entry's live parts are released too
    for (const k of [...live.keys()]) if (!entries.has(k)) release(k);
    env.onChange(key);
    return next;
  };

  const release = (key: string): void => {
    const l = live.get(key);
    if (!l) return;
    l.gen += 1;
    l.abort?.abort();
    if (l.timer != null) env.clearTimer(l.timer);
    live.delete(key);
  };

  const stopPoll = (key: string): void => {
    const l = live.get(key);
    if (l?.timer != null) env.clearTimer(l.timer);
    if (l) l.timer = null;
  };

  const startPoll = (key: string, spec: RunSpec): void => {
    const l = lv(key);
    if (l.timer != null) return;
    const gen = l.gen;
    const tick = (): void => {
      l.timer = null;
      if (l.gen !== gen) return;
      void spec.loadSaved().then((saved) => {
        if (l.gen !== gen) return;
        l.pollFailures = 0;
        if (saved) apply(key, { type: 'saved', saved });
        if (entries.get(key)?.polling) l.timer = env.setTimer(tick, pollMs);
      }, (err: unknown) => {
        if (l.gen !== gen) return;
        l.pollFailures += 1;
        if (l.pollFailures >= maxFail) {
          apply(key, { type: 'loadFailed', error: asAskError(err) });
          return;
        }
        l.timer = env.setTimer(tick, pollMs);
      });
    };
    l.timer = env.setTimer(tick, pollMs);
  };

  const failed = (key: string, err: unknown, gen: number): void => {
    const l = lv(key);
    if (l.gen !== gen) return;
    const e = asAskError(err);
    const cur = entries.get(key);
    if (e.status === 0 && cur?.attempt?.status === 'streaming') {
      // A stream that broke after it started: the server goes on writing, so follow the saved route
      apply(key, { type: 'lost' });
      return;
    }
    apply(key, { type: 'askFailed', error: e });
    if (refreshesStatus(e)) env.onRefuse(e);
  };

  const startAsk = (key: string, spec: RunSpec): Promise<void> => {
    const l = lv(key);
    stopPoll(key);
    l.abort?.abort();
    l.gen += 1;
    const gen = l.gen;
    const ac = new AbortController();
    l.abort = ac;
    apply(key, { type: 'ask' });
    const on = (ev: StreamEvent): void => {
      if (l.gen !== gen) return;
      apply(key, { type: 'event', ev, now: now() });
    };
    return spec.ask(ac.signal, on).then(() => {
      if (l.gen !== gen) return;
      // The stream ended without a terminal event: the answer is still running on the server
      const cur = entries.get(key);
      if (cur?.attempt?.status === 'streaming') {
        apply(key, { type: 'lost' });
        startPoll(key, spec);
      } else if (cur?.requesting) {
        apply(key, { type: 'askFailed', error: { status: 0, code: null } });
      }
    }, (err: unknown) => {
      if (ac.signal.aborted || l.gen !== gen) return;
      failed(key, err, gen);
      if (entries.get(key)?.polling) startPoll(key, spec);
    });
  };

  const read = async (key: string, spec: RunSpec): Promise<void> => {
    const l = lv(key);
    l.gen += 1;
    const gen = l.gen;
    apply(key, { type: 'loading' });
    let saved: SavedPayload | null;
    try {
      saved = await spec.loadSaved();
    } catch (err) {
      if (l.gen !== gen) return;
      const e = asAskError(err);
      apply(key, { type: 'loadFailed', error: e });
      if (refreshesStatus(e)) env.onRefuse(e);
      return;
    }
    if (l.gen !== gen) return;
    if (!saved || !saved.latest) {
      if (saved) apply(key, { type: 'saved', saved });
      await startAsk(key, spec);
      return;
    }
    const next = apply(key, { type: 'saved', saved });
    if (next.polling) startPoll(key, spec);
  };

  // A finished entry shown from memory is checked against the server in the background: the answer may be gone (its chat was
  // deleted by the user in another place, by an administrator or in another tab). Gone → it is asked again, as on a first open.
  const verify = (key: string, spec: RunSpec): void => {
    const l = lv(key);
    const gen = l.gen;
    void spec.loadSaved().then((saved) => {
      if (l.gen !== gen || !saved || saved.latest) return;
      const cur = entries.get(key);
      if (!cur || cur.loading || cur.requesting || cur.attempt?.status === 'streaming') return;
      apply(key, { type: 'saved', saved });
      void startAsk(key, spec);
    }, () => {
      // Offline: what is shown stays
    });
  };

  return {
    entry: (key) => entries.get(key),
    async open(key, spec) {
      const cur = entries.get(key);
      const stale = cur != null && cur.failure != null && !cur.requesting && cur.attempt?.status !== 'streaming';
      if (cur && !cur.loading && !stale) {
        // In memory: show it at once; a running answer without a stream keeps being polled
        boundedSet(entries, key, cur, max);
        env.onChange(key);
        if (cur.polling) startPoll(key, spec);
        else if (!cur.requesting && cur.attempt?.status !== 'streaming') verify(key, spec);
        return;
      }
      if (cur?.loading && lv(key).abort == null) return; // a read is already running
      await read(key, spec);
    },
    regenerate: (key, spec) => startAsk(key, spec),
    async stop(key, spec) {
      const l = lv(key);
      const cur = entries.get(key);
      l.gen += 1;
      l.abort?.abort();
      l.abort = null;
      stopPoll(key);
      const chatId = cur?.chatId ?? null;
      apply(key, { type: 'localStop' });
      if (chatId != null) {
        try {
          await spec.stopChat(chatId);
        } catch {
          // The answer may have ended on its own; nothing to undo
        }
      }
    },
    async stopAndAsk(key, spec, runningChatId) {
      try {
        await spec.stopChat(runningChatId);
      } catch {
        // Ended already, or refused: the ask below shows the busy note again if it is still running
      }
      await startAsk(key, spec);
    },
    drop(key) {
      release(key);
      entries.delete(key);
    },
    dropWhere(pred) {
      for (const [k, e] of [...entries]) {
        if (!pred(e)) continue;
        release(k);
        entries.delete(k);
        env.onChange(k);
      }
    },
    clearAll() {
      for (const k of [...live.keys()]) release(k);
      live.clear();
      entries.clear();
    },
    chatIdWhere(prefix) {
      for (const [k, e] of entries) if (k.startsWith(prefix) && e.chatId != null) return e.chatId;
      return null;
    },
    size: () => entries.size,
  };
}

/**
 * Any thrown value as an AskError. An error with a numeric `status` (the frontend's ApiError: `status`, `code`, and the
 * `running` of a 409 ai.busy, which the API layer must keep, INTEGRATION.md) keeps them; anything else is "no HTTP answer".
 */
export function asAskError(v: unknown): AskError {
  if (typeof v === 'object' && v !== null) {
    const o = v as { status?: unknown; code?: unknown; running?: unknown; limit?: unknown };
    if (typeof o.status === 'number') {
      const e: AskError = { status: o.status, code: typeof o.code === 'string' && o.code ? o.code : null };
      const running = runningOf(o.running);
      if (running) e.running = running;
      if (typeof o.limit === 'number' && Number.isFinite(o.limit)) e.limit = o.limit;
      return e;
    }
  }
  return { status: 0, code: null };
}
