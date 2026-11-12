import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { asAskError, createRunner, type RunSpec } from './ai-run.ts';
import { entryView, type AskError, type CallBlock, type SavedPayload, type StreamEvent } from './ai-study.ts';

/** Manual timers: `fire()` runs the oldest pending one */
function fakeEnv(over: { maxEntries?: number; maxPollFailures?: number; blocked?: () => CallBlock | null } = {}) {
  const timers = new Map<number, () => void>();
  let next = 1;
  const changes: string[] = [];
  const refused: AskError[] = [];
  const limited: CallBlock[] = [];
  const env = {
    onCallLimit: (q: CallBlock) => { limited.push(q); },
    setTimer: (fn: () => void) => { timers.set(next, fn); return next++; },
    clearTimer: (h: unknown) => { timers.delete(h as number); },
    onChange: (k: string) => { changes.push(k); },
    onRefuse: (e: AskError) => { refused.push(e); },
    now: () => 5000,
    ...over,
  };
  const fire = async (): Promise<void> => {
    const [id, fn] = [...timers.entries()][0] ?? [];
    if (id == null || !fn) throw new Error('no timer');
    timers.delete(id);
    fn();
    await tick();
  };
  return { env, timers, changes, refused, limited, fire };
}

const tick = (): Promise<void> => new Promise((r) => setImmediate(r));
const EN = 'en' as const;
const nothing: SavedPayload = { chatId: null, latest: null, lastGood: null };
const ans = (o: Record<string, unknown> = {}) => ({
  messageId: 1, status: 'done' as const, error: null, truncated: false, content: 'saved', lang: EN, createdAt: 1, finishedAt: 2, sample: null, ...o,
});

function spec(o: Partial<RunSpec> & { events?: StreamEvent[]; saved?: SavedPayload | null | (() => SavedPayload | null) } = {}): RunSpec & { calls: string[] } {
  const calls: string[] = [];
  return {
    calls,
    loadSaved: o.loadSaved ?? (async () => {
      calls.push('saved');
      return typeof o.saved === 'function' ? o.saved() : o.saved === undefined ? nothing : o.saved;
    }),
    ask: o.ask ?? (async (_s, on) => {
      calls.push('ask');
      for (const ev of o.events ?? []) on(ev);
    }),
    stopChat: o.stopChat ?? (async (id) => { calls.push('stop:' + id); }),
  };
}
const ev = (event: string, data: unknown = {}): StreamEvent => ({ event, data });
const STARTED = [ev('start', { chatId: 7, assistantMessageId: 3, lang: 'en' }), ev('delta', { text: 'Hi' }), ev('done')];

describe('open', () => {
  it('nothing saved: asks and streams to the end', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ events: STARTED });
    await r.open('k', s);
    assert.deepEqual(s.calls, ['saved', 'ask']);
    const v = entryView(r.entry('k')!, EN);
    assert.deepEqual([v.phase, v.text], ['done', 'Hi']);
    assert.equal(r.chatIdWhere('k'), 7);
  });
  it('a saved done answer is shown at once and nothing is asked', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ saved: { chatId: 2, latest: ans(), lastGood: null } });
    await r.open('k', s);
    assert.deepEqual(s.calls, ['saved']);
    assert.equal(entryView(r.entry('k')!, EN).text, 'saved');
  });
  it('opening again uses the memory: no second read', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ saved: { chatId: 2, latest: ans(), lastGood: null } });
    await r.open('k', s);
    await r.open('k', s);
    await tick();
    // the second open shows the memory at once and only checks in the background (nothing is asked while the answer exists)
    assert.deepEqual(s.calls, ['saved', 'saved']);
  });
  it('a deleted chat: reopening checks the server and asks again instead of offering the old answer as saved', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    let gone = false;
    const s = spec({ saved: () => (gone ? nothing : { chatId: 2, latest: ans(), lastGood: null }), events: STARTED });
    await r.open('k', s);
    gone = true;
    await r.open('k', s);
    await tick();
    assert.deepEqual(s.calls, ['saved', 'saved', 'ask']);
    assert.equal(r.chatIdWhere('k'), 7);
  });
  it('dropWhere forgets the entries of a deleted chat only', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    await r.open('a', spec({ saved: { chatId: 2, latest: ans(), lastGood: null } }));
    await r.open('b', spec({ saved: { chatId: 3, latest: ans(), lastGood: null } }));
    r.dropWhere((e) => e.chatId === 2);
    assert.equal(r.entry('a'), undefined);
    assert.ok(r.entry('b'));
    r.dropWhere(() => true);
    assert.equal(r.size(), 0);
  });
  it('reopening after a refused ask (busy) reads again instead of keeping the stale note', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const running = { chatId: 9, messageId: 1, chatKind: 'practice', kind: 'hint', questionId: 'q', questionNo: 1, source: 'practice', recordId: null };
    await r.open('k', spec({ ask: async () => { throw { status: 409, code: 'ai.busy', running }; } }));
    assert.equal(entryView(r.entry('k')!, EN).note?.kind, 'busy');
    const s = spec({ events: STARTED });
    await r.open('k', s);
    assert.deepEqual(s.calls, ['saved', 'ask']);
    assert.equal(entryView(r.entry('k')!, EN).note, null);
  });
  it('a limit refusal keeps its number', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    await r.open('k', spec({ ask: async () => { throw { status: 409, code: 'ai.messageLimit', limit: 100 }; } }));
    const v = entryView(r.entry('k')!, EN);
    assert.deepEqual(v.note, { kind: 'error', code: 'ai.messageLimit', status: 409, limit: 100 });
    assert.equal(asAskError({ status: 409, code: 'ai.chatLimit', limit: 7 }).limit, 7);
  });
  it('an `at` that does not parse (loadSaved null) generates without a lookup result', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ saved: null, events: STARTED });
    await r.open('k', s);
    assert.deepEqual(s.calls, ['saved', 'ask']);
    assert.equal(entryView(r.entry('k')!, EN).phase, 'done');
  });
  it('a saved running answer is polled every tick until it ends', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    let n = 0;
    const s = spec({
      saved: undefined,
      loadSaved: async () => {
        n += 1;
        return n < 3
          ? { chatId: 2, latest: ans({ status: 'streaming', content: 'part' + n, finishedAt: null }), lastGood: null }
          : { chatId: 2, latest: ans({ content: 'full' }), lastGood: null };
      },
    });
    await r.open('k', s);
    assert.equal(entryView(r.entry('k')!, EN).phase, 'streaming');
    assert.equal(f.timers.size, 1);
    await f.fire();
    assert.equal(entryView(r.entry('k')!, EN).text, 'part2');
    await f.fire();
    assert.deepEqual([entryView(r.entry('k')!, EN).phase, entryView(r.entry('k')!, EN).text, f.timers.size], ['done', 'full', 0]);
  });
  it('latest failed with a lastGood shows the good one; no ask', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ saved: { chatId: 2, latest: ans({ status: 'error', error: 'ai.up.down', content: '' }), lastGood: ans({ content: 'older' }) } });
    await r.open('k', s);
    assert.equal(s.calls.includes('ask'), false);
    const v = entryView(r.entry('k')!, EN);
    assert.deepEqual([v.text, v.note?.kind], ['older', 'failedPrev']);
  });
  it('a failed read leaves an error entry that can be regenerated', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    await r.open('k', spec({ loadSaved: async () => { throw { status: 403, code: 'ai.suspended' }; } }));
    const v = entryView(r.entry('k')!, EN);
    assert.deepEqual([v.phase, v.canRegenerate], ['error', true]);
    assert.deepEqual(f.refused, [{ status: 403, code: 'ai.suspended' }]);
  });
});

describe('regenerate', () => {
  const saved = (): RunSpec & { calls: string[] } => spec({ saved: { chatId: 7, latest: ans(), lastGood: null }, events: STARTED });
  it('a finished regenerate becomes the shown answer', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = saved();
    await r.open('k', s);
    await r.regenerate('k', s);
    assert.equal(entryView(r.entry('k')!, EN).text, 'Hi');
  });
  it('a failed one keeps the last good answer under a note', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    await r.open('k', saved());
    await r.regenerate('k', spec({ events: [ev('start', { chatId: 7, assistantMessageId: 4 }), ev('delta', { text: 'x' }), ev('error', { code: 'ai.up.timeout' })] }));
    const v = entryView(r.entry('k')!, EN);
    assert.deepEqual([v.text, v.note?.kind], ['saved', 'failedPrev']);
  });
  it('a withheld one clears its text and keeps the good one', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    await r.open('k', saved());
    await r.regenerate('k', spec({ events: [ev('start', { chatId: 7, assistantMessageId: 4 }), ev('delta', { text: 'B is right' }), ev('error', { code: 'ai.hintLeak', withdraw: true })] }));
    const v = entryView(r.entry('k')!, EN);
    assert.deepEqual([v.text, v.note?.kind], ['saved', 'withheldPrev']);
  });
  it('a refusal before the stream refreshes the status and keeps the answer', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    await r.open('k', saved());
    await r.regenerate('k', spec({ ask: async () => { throw { status: 403, code: 'ai.notAllowed' }; } }));
    assert.deepEqual(f.refused, [{ status: 403, code: 'ai.notAllowed' }]);
    const v = entryView(r.entry('k')!, EN);
    assert.deepEqual([v.text, v.note?.kind], ['saved', 'error']);
  });
  it('a 409 busy does not refresh the status and shows the busy note', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    await r.open('k', saved());
    const running = { chatId: 9, messageId: 1, chatKind: 'practice', kind: 'hint', questionId: 'q', questionNo: 2, source: 'practice', recordId: null };
    await r.regenerate('k', spec({ ask: async () => { throw { status: 409, code: 'ai.busy', running }; } }));
    assert.deepEqual(f.refused, []);
    assert.equal(entryView(r.entry('k')!, EN).note?.kind, 'busy');
  });
  it('a stream that breaks after it started becomes polling of the saved route', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({
      saved: { chatId: 7, latest: ans({ status: 'done', content: 'final' }), lastGood: null },
      ask: async (_s, on) => { on(ev('start', { chatId: 7, assistantMessageId: 5 })); on(ev('delta', { text: 'ab' })); throw new TypeError('network'); },
    });
    await r.regenerate('k', s);
    assert.equal(r.entry('k')!.polling, true);
    assert.equal(f.timers.size, 1);
    await f.fire();
    assert.equal(entryView(r.entry('k')!, EN).text, 'final');
    assert.equal(f.timers.size, 0);
  });
  it('a stream that ends without a terminal event is also followed by polling', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    await r.regenerate('k', spec({ events: [ev('start', { chatId: 7, assistantMessageId: 5 })] }));
    assert.equal(r.entry('k')!.polling, true);
    assert.equal(f.timers.size, 1);
  });
});

describe('stop', () => {
  it('aborts the stream, stops that chat only, and keeps the partial text as stopped', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    let signal: AbortSignal | undefined;
    let release: () => void = () => {};
    const s = spec({
      ask: (sig, on) => {
        signal = sig;
        on(ev('start', { chatId: 7, assistantMessageId: 5 }));
        on(ev('delta', { text: 'half' }));
        return new Promise<void>((_res, rej) => {
          release = () => rej(new DOMException('aborted', 'AbortError'));
          sig.addEventListener('abort', release);
        });
      },
    });
    const p = r.regenerate('k', s);
    await tick();
    assert.equal(entryView(r.entry('k')!, EN).canStop, true);
    await r.stop('k', s);
    await p;
    assert.equal(signal?.aborted, true);
    assert.deepEqual(s.calls, ['stop:7']);
    const v = entryView(r.entry('k')!, EN);
    assert.deepEqual([v.phase, v.text], ['stopped', 'half']);
    // a late event after the stop is ignored
    assert.equal(r.entry('k')!.attempt?.status, 'stopped');
  });
  it('stop before the start event only aborts (the chat is not known yet)', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ ask: (sig) => new Promise<void>((_res, rej) => sig.addEventListener('abort', () => rej(new DOMException('a', 'AbortError')))) });
    const p = r.regenerate('k', s);
    await tick();
    await r.stop('k', s);
    await p;
    assert.deepEqual(s.calls, []);
    assert.equal(r.entry('k')!.requesting, false);
  });
  it('stopping a polled answer stops its chat and its polling', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ saved: { chatId: 2, latest: ans({ status: 'streaming', content: 'p', finishedAt: null }), lastGood: null } });
    await r.open('k', s);
    await r.stop('k', s);
    assert.deepEqual(s.calls, ['saved', 'stop:2']);
    assert.equal(f.timers.size, 0);
    assert.equal(entryView(r.entry('k')!, EN).phase, 'stopped');
  });
});

describe('busy flow', () => {
  it('stopAndAsk stops exactly the named chat, then asks once', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ events: STARTED });
    await r.stopAndAsk('k', s, 41);
    assert.deepEqual(s.calls, ['stop:41', 'ask']);
    assert.equal(entryView(r.entry('k')!, EN).text, 'Hi');
  });
  it('a second busy answer shows the note again (no loop)', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    let asks = 0;
    const s = spec({ ask: async () => { asks += 1; throw { status: 409, code: 'ai.busy', running: null }; } });
    await r.stopAndAsk('k', s, 41);
    assert.equal(asks, 1);
    assert.equal(entryView(r.entry('k')!, EN).note?.kind, 'busy');
  });
  it('a failing stop does not prevent the ask', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ events: STARTED, stopChat: async () => { throw new Error('x'); } });
    await r.stopAndAsk('k', s, 41);
    assert.equal(entryView(r.entry('k')!, EN).phase, 'done');
  });
});

describe('polling failures, bounds and sign-out', () => {
  it('gives up after the allowed consecutive failures with an error note', async () => {
    const f = fakeEnv({ maxPollFailures: 2 });
    const r = createRunner(f.env);
    let n = 0;
    const s = spec({
      loadSaved: async () => {
        n += 1;
        if (n === 1) return { chatId: 2, latest: ans({ status: 'streaming', content: 'p', finishedAt: null }), lastGood: null };
        throw { status: 0, code: null };
      },
    });
    await r.open('k', s);
    await f.fire();
    assert.equal(f.timers.size, 1, 'one failure: tries again');
    await f.fire();
    assert.equal(f.timers.size, 0);
    assert.equal(r.entry('k')!.polling, false);
    assert.equal(r.entry('k')!.failure?.status, 0);
  });
  it('clearAll aborts streams, stops polls and forgets every entry', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    let aborted = false;
    const hang = spec({ ask: (sig) => new Promise<void>(() => sig.addEventListener('abort', () => { aborted = true; })) });
    void r.regenerate('a', hang);
    await r.open('b', spec({ saved: { chatId: 2, latest: ans({ status: 'streaming', content: 'p', finishedAt: null }), lastGood: null } }));
    assert.equal(f.timers.size, 1);
    r.clearAll();
    assert.equal(aborted, true);
    assert.equal(f.timers.size, 0);
    assert.equal(r.size(), 0);
    assert.equal(r.entry('a'), undefined);
  });
  it('the cache holds at most maxEntries entries and releases the evicted ones', async () => {
    const f = fakeEnv({ maxEntries: 3 });
    const r = createRunner(f.env);
    for (let i = 0; i < 5; i++) await r.open('k' + i, spec({ saved: { chatId: 1, latest: ans(), lastGood: null } }));
    assert.equal(r.size(), 3);
    assert.equal(r.entry('k0'), undefined);
    assert.ok(r.entry('k4'));
  });
  it('drop forgets one entry and its polling (a deleted record) without touching the others', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    await r.open('a', spec({ saved: { chatId: 1, latest: ans({ status: 'streaming', finishedAt: null }), lastGood: null } }));
    await r.open('b', spec({ saved: { chatId: 1, latest: ans(), lastGood: null } }));
    r.drop('a');
    assert.equal(r.entry('a'), undefined);
    assert.ok(r.entry('b'));
    assert.equal(f.timers.size, 0);
  });
  it('a late answer of an older request is ignored after the window moved on', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    let resolveSaved: (v: SavedPayload) => void = () => {};
    const slow = spec({ loadSaved: () => new Promise<SavedPayload>((res) => { resolveSaved = res; }) });
    const p = r.open('k', slow);
    await tick();
    r.clearAll();
    resolveSaved({ chatId: 1, latest: ans({ content: 'late' }), lastGood: null });
    await p;
    assert.equal(r.entry('k'), undefined);
  });
});

describe('asAskError', () => {
  it('passes AskError-shaped objects, maps anything else to no HTTP answer', () => {
    assert.deepEqual(asAskError({ status: 409, code: 'ai.busy' }), { status: 409, code: 'ai.busy' });
    assert.deepEqual(asAskError(new TypeError('x')), { status: 0, code: null });
    assert.deepEqual(asAskError(undefined), { status: 0, code: null });
  });
  it('an ApiError-like object (code undefined, running as the server sent it) is normalised', () => {
    const running = { chatId: 3, messageId: 4, chatKind: 'summary', kind: 'summary', recordId: 2 };
    const e = asAskError({ status: 409, code: 'ai.busy', running });
    assert.equal(e.code, 'ai.busy');
    assert.equal(e.running?.kind, 'summary');
    assert.deepEqual(asAskError({ status: 502, code: undefined }), { status: 502, code: null });
  });
});

describe('peek', () => {
  it('never asks: a memory hit shows at once without any request', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ saved: { chatId: 3, latest: ans(), lastGood: ans() } });
    await r.peek('k', s);
    s.calls.length = 0;
    await r.peek('k', s);
    assert.deepEqual(s.calls, []);
    assert.equal(r.entry('k')?.shown?.content, 'saved');
  });
  it('a saved hit is shown, no ask', async () => {
    const r = createRunner(fakeEnv().env);
    const s = spec({ saved: { chatId: 3, latest: ans(), lastGood: ans() } });
    await r.peek('k', s);
    assert.deepEqual(s.calls, ['saved']);
    assert.equal(entryView(r.entry('k')!, EN).phase, 'done');
  });
  it('nothing saved (a null lookup): the entry is removed again and nothing is asked', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ saved: null });
    await r.peek('k', s);
    assert.deepEqual(s.calls, ['saved'], 'looked up, never asked');
    assert.equal(r.entry('k'), undefined);
    assert.equal(r.size(), 0);
    assert.ok(f.changes.includes('k'), 'the open panel is told to repaint');
  });
  it('nothing saved (an empty payload): an empty entry, nothing asked', async () => {
    const r = createRunner(fakeEnv().env);
    const s = spec();
    await r.peek('k', s);
    assert.deepEqual(s.calls, ['saved']);
    assert.equal(entryView(r.entry('k')!, EN).phase, 'empty');
  });
  it('a failed lookup leaves an error note, never an ask', async () => {
    const r = createRunner(fakeEnv().env);
    const s = spec({ loadSaved: async () => { throw { status: 500, code: 'ai.internal' }; } });
    await r.peek('k', s);
    assert.equal(r.entry('k')?.failure?.status, 500);
    assert.ok(!s.calls.includes('ask'));
  });
  it('a running saved answer is polled', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ saved: { chatId: 3, latest: ans({ status: 'streaming', content: 'Hi' }), lastGood: null } });
    await r.peek('k', s);
    assert.equal(f.timers.size, 1);
  });
  it('open after an empty peek asks exactly once', async () => {
    const r = createRunner(fakeEnv().env);
    const s = spec({ events: STARTED });
    await r.peek('k', s);
    assert.equal(s.calls.filter((c) => c === 'ask').length, 0);
    await r.open('k', s);
    assert.equal(s.calls.filter((c) => c === 'ask').length, 1);
    assert.equal(r.entry('k')?.shown?.content, 'Hi');
  });
  it('a peek that found nothing (entry removed) and then open reads again and asks once', async () => {
    const r = createRunner(fakeEnv().env);
    const s = spec({ saved: null, events: STARTED });
    await r.peek('k', s);
    await r.open('k', s);
    assert.equal(s.calls.filter((c) => c === 'ask').length, 1);
  });
});

describe('call caps (blocked)', () => {
  const block: CallBlock = { cap: 'hour', limit: 20, retryAt: 9000 };
  const when = (b: CallBlock | null) => ({ blocked: () => b });
  it('open on a question with nothing saved does not ask; local ai.callLimit failure', async () => {
    const f = fakeEnv(when(block));
    const r = createRunner(f.env);
    const s = spec({ events: STARTED });
    await r.open('k', s);
    assert.deepEqual(s.calls, ['saved']);
    const e = r.entry('k')!;
    assert.deepEqual(e.failure, { status: 429, code: 'ai.callLimit', quota: block });
    assert.equal(entryView(e, EN).note?.kind, 'error');
    assert.equal(e.requesting, false);
    assert.equal(f.limited.length, 0, 'a local refusal is not reported as a server refusal');
  });
  it('open on a question with a saved answer still shows it: no ask, no cap note', async () => {
    const r = createRunner(fakeEnv(when(block)).env);
    const s = spec({ saved: { chatId: 3, latest: ans(), lastGood: ans() } });
    await r.open('k', s);
    assert.deepEqual(s.calls, ['saved']);
    assert.equal(entryView(r.entry('k')!, EN).note, null);
    assert.equal(r.entry('k')?.failure, null);
  });
  it('regenerate never asks and keeps the shown answer under the cap note', async () => {
    let b: CallBlock | null = null;
    const r = createRunner(fakeEnv({ blocked: () => b }).env);
    const s = spec({ saved: { chatId: 3, latest: ans(), lastGood: ans() } });
    await r.open('k', s);
    b = block;
    await r.regenerate('k', s);
    assert.ok(!s.calls.includes('ask'));
    const v = entryView(r.entry('k')!, EN);
    assert.equal(v.text, 'saved');
    assert.equal(v.note?.kind === 'error' && v.note.code, 'ai.callLimit');
    assert.equal(v.note?.kind === 'error' && v.note.quota?.retryAt, 9000);
  });
  it('stopAndAsk neither stops the other answer nor asks', async () => {
    const r = createRunner(fakeEnv(when(block)).env);
    const s = spec({ events: STARTED });
    await r.stopAndAsk('k', s, 12);
    assert.deepEqual(s.calls, []);
    assert.equal(r.entry('k')?.failure?.code, 'ai.callLimit');
  });
  it('the block ending frees the next ask', async () => {
    let b: CallBlock | null = block;
    const r = createRunner(fakeEnv({ blocked: () => b }).env);
    const s = spec({ events: STARTED });
    await r.open('k', s);
    b = null;
    await r.regenerate('k', s);
    assert.ok(s.calls.includes('ask'));
    assert.equal(r.entry('k')?.shown?.content, 'Hi');
  });
  it('a blocked regenerate leaves a running answer alone', async () => {
    let b: CallBlock | null = null;
    const r = createRunner(fakeEnv({ blocked: () => b }).env);
    let finish: () => void = () => undefined;
    const s = spec({ ask: (_sig, on) => new Promise<void>((res) => { on(ev('start', { chatId: 1, assistantMessageId: 2 })); finish = res; }) });
    const running = r.regenerate('k', s);
    await tick();
    b = block;
    await r.regenerate('k', s);
    assert.equal(r.entry('k')?.failure, null);
    assert.equal(r.entry('k')?.attempt?.status, 'streaming');
    finish();
    await running;
  });
  it('a server ai.callLimit refusal reports the quota to the state', async () => {
    const f = fakeEnv();
    const r = createRunner(f.env);
    const s = spec({ ask: async () => { throw { status: 429, code: 'ai.callLimit', quota: block }; } });
    await r.regenerate('k', s);
    assert.deepEqual(f.limited, [block]);
    assert.equal(r.entry('k')?.failure?.quota?.cap, 'hour');
  });
  it('asAskError keeps a valid quota only for ai.callLimit', () => {
    assert.deepEqual(asAskError({ status: 429, code: 'ai.callLimit', quota: block }), { status: 429, code: 'ai.callLimit', quota: block });
    assert.deepEqual(asAskError({ status: 429, code: 'ai.callLimit', quota: { cap: 'week', limit: 1, retryAt: 2 } }), { status: 429, code: 'ai.callLimit' });
    assert.deepEqual(asAskError({ status: 429, code: 'ai.other', quota: block }), { status: 429, code: 'ai.other' });
  });
});
