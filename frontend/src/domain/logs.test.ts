// System log page: the answer parsing, merging (cap, repeats, gaps, a backend restart), the level filter and the search, the
// "at the bottom" test and the poll loop (fake timers: every 0.5 s, one request at a time, longer waits after failed reads,
// none while hidden, paused or left, no timer left behind).
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  EMPTY_LOG, POLL_MAX_MS, POLL_MS, atBottom, chunkOf, createPoller, isLevelFilter, levelOf, levelPasses, lineOf, linePasses, mergeChunk,
  pollDelay, searchPasses, type LogChunk, type LogLine, type PollHost,
} from './logs.ts';

const line = (seq: number, level = 'info', message = `line ${seq}`, target = 'xexam_backend::api'): LogLine =>
  ({ seq, time: '2026-01-02 03:04:05', level, target, message });
const chunk = (lines: LogLine[], last: number, dropped = false, boot: string | null = null): LogChunk =>
  ({ lines, last, dropped, boot });
const seqs = (ls: readonly LogLine[]): number[] => ls.map((l) => l.seq);

test('lines and answers are read defensively', () => {
  assert.deepEqual(lineOf({ seq: 3, time: 't', level: 'warn', target: 'x', message: 'm' }), { seq: 3, time: 't', level: 'warn', target: 'x', message: 'm' });
  assert.equal(lineOf({ time: 't' }), null);
  assert.equal(lineOf(null), null);
  assert.equal(lineOf({ seq: 1, message: 'saved', fields: { id: 4, name: 'demo' } })?.message, 'saved id=4 name=demo');
  assert.equal(lineOf({ seq: 1 })?.level, 'info');
  assert.deepEqual(
    chunkOf({ lines: [{ seq: 1 }, 'junk'], last: 7, dropped: true, boot: 'Ab_-12' }),
    { lines: [lineOf({ seq: 1 }) as LogLine], last: 7, dropped: true, boot: 'Ab_-12' },
  );
  assert.deepEqual(chunkOf('nonsense'), { lines: [], last: null, dropped: false, boot: null });
  assert.equal(chunkOf({ lines: [], last: 1, boot: 'a b' }).boot, null);
});

test('levels: any spelling; Warnings shows warnings and errors, Errors only errors', () => {
  assert.equal(levelOf('ERROR'), 'error');
  assert.equal(levelOf('Warning'), 'warn');
  assert.equal(levelOf('debug'), 'debug');
  assert.equal(levelOf('whatever'), 'info');
  assert.ok(isLevelFilter('warn') && !isLevelFilter('info'));
  const kept = (f: 'all' | 'warn' | 'error'): string[] => ['error', 'warn', 'info', 'debug'].filter((l) => levelPasses(l, f));
  assert.deepEqual(kept('all'), ['error', 'warn', 'info', 'debug']);
  assert.deepEqual(kept('warn'), ['error', 'warn']);
  assert.deepEqual(kept('error'), ['error']);
});

test('search: case-insensitive over time, level, target and message; blank matches all', () => {
  const l = line(1, 'warn', 'Import of demo bank finished', 'xexam_backend::services');
  assert.ok(searchPasses(l, ''));
  assert.ok(searchPasses(l, '  DEMO '));
  assert.ok(searchPasses(l, 'services'));
  assert.ok(searchPasses(l, '03:04'));
  assert.ok(!searchPasses(l, 'nothing like this'));
  assert.ok(linePasses(l, 'warn', 'import'));
  assert.ok(!linePasses(l, 'error', 'import'));
});

test('merging: the first answer, new lines after it, repeats ignored', () => {
  let r = mergeChunk(EMPTY_LOG, chunk([line(2), line(1), line(3)], 3, true));
  // without lines yet, "dropped" (the buffer has already lost line 0..) marks no gap
  assert.deepEqual(seqs(r.state.lines), [1, 2, 3]);
  assert.equal(r.state.lines[0]?.gapBefore, undefined);
  assert.equal(r.state.last, 3);
  r = mergeChunk(r.state, chunk([line(3), line(4)], 4));
  assert.deepEqual(seqs(r.added), [4]);
  assert.deepEqual(seqs(r.state.lines), [1, 2, 3, 4]);
  r = mergeChunk(r.state, chunk([], 4));
  assert.deepEqual(r.added, []);
  assert.equal(r.state.last, 4);
  assert.equal(r.reset, false);
});

test('merging: a gap is marked on the first new line, and the cap keeps the newest lines', () => {
  let r = mergeChunk(EMPTY_LOG, chunk([line(1), line(2)], 2), 3);
  r = mergeChunk(r.state, chunk([line(7), line(8)], 8, true), 3);
  assert.deepEqual(seqs(r.state.lines), [2, 7, 8]);
  assert.equal(r.state.lines[1]?.gapBefore, true);
  assert.deepEqual(seqs(r.added), [7, 8]);
  r = mergeChunk(r.state, chunk([9, 10, 11, 12, 13].map((n) => line(n)), 13), 3);
  assert.deepEqual(seqs(r.state.lines), [11, 12, 13]);
  assert.deepEqual(seqs(r.added), [11, 12, 13]);
});

test('merging: a lower "last" means the backend restarted: start again from its answer', () => {
  let r = mergeChunk(EMPTY_LOG, chunk([line(40), line(41)], 41));
  r = mergeChunk(r.state, chunk([line(1), line(2)], 2, false));
  assert.equal(r.reset, true);
  assert.deepEqual(seqs(r.state.lines), [1, 2]);
  assert.deepEqual(seqs(r.added), [1, 2]);
  assert.equal(r.state.last, 2);
});

test("merging: another boot id means the backend restarted, even when its numbers passed the page's own", () => {
  // The page saw lines up to 50 of one process; the restarted backend logged 60 lines before the next poll
  let r = mergeChunk(EMPTY_LOG, chunk([line(49), line(50)], 50, false, 'first'));
  assert.equal(r.state.boot, 'first');
  const restarted = Array.from({ length: 60 }, (_, i) => line(i + 1));
  r = mergeChunk(r.state, chunk(restarted, 60, false, 'second'));
  assert.equal(r.reset, true);
  assert.equal(r.state.lines.length, 60, 'the first lines of the new process are kept');
  assert.deepEqual([r.state.last, r.state.boot], [60, 'second']);
  // the same process again: only the new lines
  r = mergeChunk(r.state, chunk([line(61)], 61, false, 'second'));
  assert.equal(r.reset, false);
  assert.deepEqual(seqs(r.added), [61]);
  // an older backend that sends no boot: the page keeps the one it has
  r = mergeChunk(r.state, chunk([line(62)], 62));
  assert.deepEqual([r.reset, r.state.boot], [false, 'second']);
});

test('at the bottom: within the slack', () => {
  assert.ok(atBottom(600, 400, 1000));
  assert.ok(atBottom(580, 400, 1000));
  assert.ok(!atBottom(500, 400, 1000));
  assert.ok(atBottom(0, 400, 300));
});

/** Fake timers and visibility for the poll loop: requests stay open until the test answers (ok) or fails them */
interface FakeHost {
  host: PollHost;
  /** Requests sent so far */
  calls: () => number;
  /** The most requests that ever ran at the same time */
  maxOpen: () => number;
  /** Timers set and not yet fired or cleared */
  timers: () => number;
  /** The delay of every timer set so far, in order */
  delays: number[];
  /** Fires every pending timer */
  fire: () => Promise<void>;
  setVisible: (v: boolean) => void;
  /** Answers the oldest running request (ok = a good read, otherwise a failed one) and lets the loop continue */
  settle: (ok?: boolean) => Promise<void>;
}

function fakeHost(): FakeHost {
  let calls = 0;
  let visible = true;
  let next = 1;
  let maxOpen = 0;
  const pending = new Map<number, () => void>();
  const running: Array<{ ok: () => void; fail: (e: unknown) => void }> = [];
  const delays: number[] = [];
  const flush = async (): Promise<void> => {
    for (let i = 0; i < 10; i++) await Promise.resolve();
  };
  const host: PollHost = {
    fetch: () => {
      calls++;
      return new Promise<void>((ok, fail) => {
        running.push({ ok, fail });
        maxOpen = Math.max(maxOpen, running.length);
      });
    },
    visible: () => visible,
    setTimer: (fn, ms) => {
      const id = next++;
      pending.set(id, fn);
      delays.push(ms);
      return id;
    },
    clearTimer: (id) => {
      pending.delete(id as number);
    },
  };
  return {
    host,
    calls: () => calls,
    maxOpen: () => maxOpen,
    timers: () => pending.size,
    delays,
    fire: async () => {
      const fns = [...pending.values()];
      pending.clear();
      for (const f of fns) f();
      await flush();
    },
    setVisible: (v) => {
      visible = v;
    },
    settle: async (ok = true) => {
      const r = running.shift();
      if (ok) r?.ok();
      else r?.fail(new Error('read failed'));
      await flush();
    },
  };
}

test('poll interval: every 0.5 seconds; after failed reads 1, 2, 4, 8, then every 10 seconds', () => {
  assert.equal(POLL_MS, 500);
  assert.equal(POLL_MAX_MS, 10_000);
  assert.deepEqual([0, 1, 2, 3, 4, 5, 6, 50, 5000].map((n) => pollDelay(n)), [500, 1000, 2000, 4000, 8000, 10_000, 10_000, 10_000, 10_000]);
  // nonsense counts are no failures; the wait is never shorter than the interval, even with a cap under it
  assert.deepEqual([-1, Number.NaN].map((n) => pollDelay(n)), [500, 500]);
  assert.equal(pollDelay(3, 1500, 1000), 1500);
});

test('poller: asks at once, then one request at a time, every POLL_MS after the previous answer', async () => {
  const f = fakeHost();
  const p = createPoller(f.host);
  p.start();
  assert.equal(f.calls(), 1);
  assert.equal(f.timers(), 0, 'no timer while a request runs');
  await f.settle();
  assert.deepEqual(f.delays, [POLL_MS]);
  await f.fire();
  assert.equal(f.calls(), 2);
  p.start(); // a redraw of the open page does not start a second loop
  assert.equal(f.calls(), 2);
  await f.settle();
  assert.equal(f.timers(), 1);
  assert.deepEqual(f.delays, [POLL_MS, POLL_MS]);
  assert.equal(f.maxOpen(), 1);
});

test('poller: a slow answer never stacks requests', async () => {
  const f = fakeHost();
  const p = createPoller(f.host);
  p.start();
  // the answer takes "long": no timer exists meanwhile, so firing timers sends nothing
  for (let i = 0; i < 20; i++) await f.fire();
  assert.equal(f.calls(), 1);
  assert.ok(p.busy);
  // Resume / visible again while it still runs: still nothing next to it ...
  p.setPaused(true);
  p.setPaused(false);
  p.visibilityChanged();
  assert.equal(f.calls(), 1);
  // ... and one request right after the answer, then the usual interval
  await f.settle();
  assert.equal(f.calls(), 2);
  assert.equal(f.timers(), 0);
  await f.settle();
  assert.deepEqual(f.delays, [POLL_MS]);
  assert.equal(f.maxOpen(), 1);
});

test('poller: failed reads wait longer each time; a good read goes back to POLL_MS', async () => {
  const f = fakeHost();
  const p = createPoller(f.host);
  p.start();
  for (let i = 0; i < 6; i++) {
    await f.settle(false);
    await f.fire();
  }
  assert.equal(p.failures, 6);
  assert.deepEqual(f.delays, [1000, 2000, 4000, 8000, 10_000, 10_000]);
  await f.settle(true);
  assert.equal(p.failures, 0);
  assert.equal(f.delays.at(-1), POLL_MS);
  assert.equal(f.maxOpen(), 1);
});

test('poller: Resume asks at once even while backing off; opening the page again starts afresh', async () => {
  const f = fakeHost();
  const p = createPoller(f.host);
  p.start();
  await f.settle(false);
  await f.fire();
  await f.settle(false);
  assert.deepEqual(f.delays, [1000, 2000]);
  p.setPaused(true);
  assert.equal(f.timers(), 0, 'paused: the backed-off timer is cleared');
  p.setPaused(false);
  assert.equal(f.calls(), 3, 'Resume asks at once');
  await f.settle(false);
  assert.equal(f.delays.at(-1), 4000, 'and keeps the failure count');
  p.stop();
  assert.equal(f.timers(), 0);
  p.start();
  assert.equal(p.failures, 0, 'a new visit starts without failures');
  assert.equal(f.calls(), 4);
  await f.settle(true);
  assert.equal(f.delays.at(-1), POLL_MS);
});

test('poller: none while hidden or paused; visible again / Resume asks at once; stop ends it', async () => {
  const f = fakeHost();
  const p = createPoller(f.host);
  p.start();
  await f.settle();
  f.setVisible(false);
  p.visibilityChanged();
  assert.equal(f.timers(), 0);
  await f.fire();
  assert.equal(f.calls(), 1, 'nothing while hidden');
  f.setVisible(true);
  p.visibilityChanged();
  assert.equal(f.calls(), 2);
  await f.settle();
  p.setPaused(true);
  assert.ok(p.paused && !p.running);
  assert.equal(f.timers(), 0);
  p.setPaused(false);
  assert.equal(f.calls(), 3);
  await f.settle();
  p.stop();
  assert.equal(f.timers(), 0);
  await f.fire();
  assert.equal(f.calls(), 3);
});

test('poller: hidden and shown again while a request runs: the loop goes on, one request at a time', async () => {
  const f = fakeHost();
  const p = createPoller(f.host);
  p.start();
  f.setVisible(false);
  p.visibilityChanged();
  f.setVisible(true);
  p.visibilityChanged();
  assert.equal(f.calls(), 1);
  await f.settle();
  assert.equal(f.calls(), 2, 'asked again right after the answer');
  await f.settle();
  assert.equal(f.timers(), 1, 'and the loop goes on');
  assert.equal(f.maxOpen(), 1);
});

test('poller: hidden while a request runs: nothing follows its answer until the page is visible again', async () => {
  const f = fakeHost();
  const p = createPoller(f.host);
  p.start();
  f.setVisible(false);
  p.visibilityChanged();
  await f.settle();
  assert.equal(f.timers(), 0);
  assert.equal(f.calls(), 1);
  f.setVisible(true);
  p.visibilityChanged();
  assert.equal(f.calls(), 2);
});

test('poller: left and opened again while a request runs: one loop goes on, no second one', async () => {
  const f = fakeHost();
  const p = createPoller(f.host);
  p.start();
  p.stop();
  p.start();
  assert.equal(f.calls(), 1, 'nothing next to the running request');
  await f.settle();
  assert.equal(f.calls(), 2);
  await f.settle();
  assert.equal(f.timers(), 1);
  await f.fire();
  assert.equal(f.calls(), 3);
  assert.equal(f.maxOpen(), 1);
});

test('poller: a request that finishes after stop schedules nothing, also when it failed', async () => {
  for (const ok of [true, false]) {
    const f = fakeHost();
    const p = createPoller(f.host);
    p.start();
    p.stop();
    await f.settle(ok);
    assert.equal(f.timers(), 0);
    assert.equal(f.calls(), 1);
    assert.ok(!p.busy);
  }
});
