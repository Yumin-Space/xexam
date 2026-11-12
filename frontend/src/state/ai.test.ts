// What the signed-in user may do with AI (v0.8): 'loading' until the first answer, then 'ready' or (any failure)
// 'unavailable'; at most every 60 s unless forced; never while a new password must be chosen; an answer for the previous
// user is dropped; and when the page must be drawn again. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { beforeEach, mock, test } from 'node:test';
import {
  AI_STATUS_MAX_AGE_MS, aiBadgeOn, aiChatVisible, aiEnabled, aiFreeMode, aiModeShown, aiNeedsRender, aiStatus, aiSuspended, callBlock, noteCallBlock,
  onCallBlockChange, recheckCallBlock, RECHECK_MIN_MS, refreshAiStatus, resetAiStatus, setAiFreeMode, statusOf,
  type AiStatus,
} from './ai.ts';
import type { AiCallBlock } from '../types/api.ts';

const answer = (o: Record<string, unknown> = {}) => ({
  enabled: true, allowed: true, hasData: false, running: null, retentionDays: 90, memoryLimit: 50, memoryMaxChars: 300,
  maxMessageChars: 16000, keyProblem: false, configured: false, ...o,
});

beforeEach(() => resetAiStatus());

test('it starts as loading, then becomes ready with the answer', async () => {
  assert.deepEqual(aiStatus(), { state: 'loading' });
  const changed = await refreshAiStatus({ mustChangePassword: false, now: 1000, fetcher: async () => answer() });
  assert.equal(changed, true);
  const st = aiStatus();
  assert.equal(st.state, 'ready');
  assert.ok(st.state === 'ready' && st.allowed && st.enabled && st.messageChars === 16000 && st.memoryChars === 300);
  assert.equal(aiBadgeOn(), true);
  assert.equal(aiChatVisible(), true);
  assert.equal(aiEnabled(), true);
});

test('any failure is "unavailable", silently (no throw); a malformed answer too', async () => {
  assert.equal(await refreshAiStatus({ mustChangePassword: false, fetcher: async () => { throw new Error('502'); } }), true);
  assert.deepEqual(aiStatus(), { state: 'unavailable' });
  assert.equal(aiChatVisible(), false);
  resetAiStatus();
  await refreshAiStatus({ mustChangePassword: false, fetcher: async () => '<html>' });
  assert.deepEqual(aiStatus(), { state: 'unavailable' });
  assert.deepEqual(statusOf({ enabled: 'yes' }), { state: 'unavailable' });
});

test('without the length fields the status falls back to 16000 message / 200 memory characters (v1.0 C-8d)', () => {
  const st = statusOf({ enabled: true, allowed: true, hasData: false, running: null, memoryLimit: 50 });
  assert.ok(st.state === 'ready' && st.messageChars === 16000 && st.memoryChars === 200);
});

test('not asked while a new password must be chosen', async () => {
  let calls = 0;
  await refreshAiStatus({ mustChangePassword: true, fetcher: async () => { calls += 1; return answer(); } });
  assert.equal(calls, 0);
  assert.deepEqual(aiStatus(), { state: 'unavailable' });
});

test('read again at most every 60 s unless forced', async () => {
  let calls = 0;
  const fetcher = async (): Promise<unknown> => {
    calls += 1;
    return answer();
  };
  await refreshAiStatus({ mustChangePassword: false, now: 1000, fetcher });
  await refreshAiStatus({ mustChangePassword: false, now: 1000 + AI_STATUS_MAX_AGE_MS - 1, fetcher });
  assert.equal(calls, 1);
  await refreshAiStatus({ mustChangePassword: false, now: 1000 + AI_STATUS_MAX_AGE_MS - 1, force: true, fetcher });
  assert.equal(calls, 2);
  await refreshAiStatus({ mustChangePassword: false, now: Date.now() + AI_STATUS_MAX_AGE_MS + 1, fetcher });
  assert.equal(calls, 3);
});

test('an answer that arrives after a sign-out is dropped', async () => {
  let release: (v: unknown) => void = () => {};
  const pending = refreshAiStatus({ mustChangePassword: false, fetcher: () => new Promise((r) => { release = r; }) });
  resetAiStatus();
  release(answer());
  assert.equal(await pending, false);
  assert.deepEqual(aiStatus(), { state: 'loading' });
});

test('the page is drawn again when the state, enabled, allowed, suspended or hasData change', () => {
  const ready = (o: Record<string, unknown> = {}): AiStatus => statusOf(answer(o));
  assert.equal(aiNeedsRender({ state: 'loading' }, ready()), true);
  assert.equal(aiNeedsRender({ state: 'loading' }, { state: 'unavailable' }), true);
  assert.equal(aiNeedsRender({ state: 'unavailable' }, { state: 'unavailable' }), false);
  assert.equal(aiNeedsRender(ready(), ready()), false);
  assert.equal(aiNeedsRender(ready(), ready({ allowed: false })), true, 'AI+ switched off');
  assert.equal(aiNeedsRender(ready(), ready({ enabled: false })), true, 'AI switched off');
  assert.equal(aiNeedsRender(ready({ allowed: false }), ready({ allowed: false, hasData: true })), true, 'the first chat');
  assert.equal(aiNeedsRender(ready(), ready({ retentionDays: 30 })), false);
  // round 2: an administrator suspends the AI+ function (allowed may already be false, e.g. the AI+ service is off too)
  assert.equal(aiNeedsRender(ready({ allowed: false }), ready({ allowed: false, suspended: true })), true, 'suspended');
  assert.equal(aiNeedsRender(ready({ suspended: true }), ready({ suspended: true })), false);
});

test('round 2: "suspended" is read from the answer (missing or not a boolean = false)', () => {
  const sus = statusOf(answer({ allowed: false, suspended: true, hasData: true }));
  assert.ok(sus.state === 'ready' && sus.suspended === true && sus.allowed === false);
  assert.equal(aiSuspended(sus), true);
  assert.equal(aiBadgeOn(sus), false);
  assert.equal(aiChatVisible(sus), true, 'read only: the user keeps their chats and memories');
  const old = statusOf(answer());
  assert.ok(old.state === 'ready' && old.suspended === false);
  const odd = statusOf(answer({ suspended: 'yes' }));
  assert.ok(odd.state === 'ready' && odd.suspended === false);
  assert.equal(aiSuspended({ state: 'unavailable' }), false);
  assert.equal(aiSuspended({ state: 'loading' }), false);
});

test('the menu and the badge: allowed shows both; data only shows AI Chat (read only)', () => {
  const dataOnly = statusOf(answer({ allowed: false, hasData: true }));
  assert.equal(aiBadgeOn(dataOnly), false);
  assert.equal(aiChatVisible(dataOnly), true);
  const neither = statusOf(answer({ allowed: false, hasData: false, enabled: false }));
  assert.equal(aiChatVisible(neither), false);
  assert.equal(aiEnabled(neither), false);
  // the spec's other spelling of the limits (14.1) is read too
  const st = statusOf(answer({ messageChars: 20000, memoryChars: 500 }));
  assert.ok(st.state === 'ready' && st.messageChars === 20000 && st.memoryChars === 500);
  const running = statusOf(answer({ running: { chatId: 4, messageId: 9 } }));
  assert.ok(running.state === 'ready' && running.running?.chatId === 4);
});

test('the free mode of the status: false unless the server says so; the switch updates the kept status; sign-out forgets it', async () => {
  assert.equal((statusOf(answer()) as { freeMode: boolean }).freeMode, false);
  assert.equal((statusOf(answer({ freeMode: 'yes' })) as { freeMode: boolean }).freeMode, false);
  await refreshAiStatus({ mustChangePassword: false, now: 1000, force: true, fetcher: async () => answer({ freeMode: true }) });
  assert.equal(aiFreeMode(), true, 'read from the server, so a reload or another device shows it');
  setAiFreeMode(false);
  assert.equal(aiFreeMode(), false);
  setAiFreeMode(true);
  assert.equal(aiFreeMode(), true);
  resetAiStatus();
  assert.equal(aiFreeMode(), false);
  setAiFreeMode(true);
  assert.equal(aiFreeMode(), false, 'nothing to update before the first answer');
});

test('the AI chat mode section of Settings: administrators only, and only while the AI service is on', () => {
  const on = statusOf(answer({ enabled: true }));
  const off = statusOf(answer({ enabled: false }));
  assert.equal(aiModeShown(true, on), true);
  assert.equal(aiModeShown(false, on), false, 'never for other users');
  assert.equal(aiModeShown(true, off), false);
  assert.equal(aiModeShown(true, { state: 'loading' }), false);
  assert.equal(aiModeShown(true, { state: 'unavailable' }), false);
});

// v1.0 per-user AI request caps ----------------------------------------------------------------------------------------

const FUTURE = Math.floor(Date.now() / 1000) + 3600;
const blockOf = (o: Record<string, unknown> = {}) => ({ cap: 'hour', limit: 20, retryAt: FUTURE, ...o });
const callsOf = (o: Record<string, unknown> = {}) => ({
  used: { minute: 2, hour: 7, day: 37 }, caps: { minute: 5, hour: 20, day: 100 }, blocked: null, ...o,
});

test('caps: statusOf reads calls (used, caps, blocked); a malformed field drops calls; absent stays absent', () => {
  const st = statusOf(answer({ calls: callsOf({ blocked: blockOf() }) }));
  assert.ok(st.state === 'ready' && st.calls);
  assert.deepEqual(st.calls.used, { minute: 2, hour: 7, day: 37 });
  assert.deepEqual(st.calls.caps, { minute: 5, hour: 20, day: 100 });
  assert.deepEqual(st.calls.blocked, { cap: 'hour', limit: 20, retryAt: FUTURE });
  const none = statusOf(answer({ calls: callsOf() }));
  assert.ok(none.state === 'ready' && none.calls?.blocked === null);
  const bad: Array<Record<string, unknown>> = [
    { used: { minute: -1, hour: 0, day: 0 } },
    { used: { minute: 1, hour: 'x', day: 0 } },
    { caps: null },
    { blocked: { cap: 'week', limit: 5, retryAt: FUTURE } },
    { blocked: { cap: 'day', limit: 'a', retryAt: FUTURE } },
    { blocked: { cap: 'day', limit: 5, retryAt: 0 } },
    { blocked: 'yes' },
  ];
  for (const o of bad) {
    const s = statusOf(answer({ calls: callsOf(o) }));
    assert.ok(s.state === 'ready' && s.calls === undefined, JSON.stringify(o));
  }
  const absent = statusOf(answer());
  assert.ok(absent.state === 'ready' && !('calls' in absent));
});

test('caps: aiNeedsRender is true when calls.blocked changes, not when only the counts change', () => {
  const ready = (calls?: unknown): AiStatus => statusOf(answer(calls === undefined ? {} : { calls }));
  assert.equal(aiNeedsRender(ready(callsOf()), ready(callsOf({ used: { minute: 3, hour: 8, day: 38 } }))), false);
  assert.equal(aiNeedsRender(ready(callsOf()), ready(callsOf({ blocked: blockOf() }))), true, 'blocked now');
  assert.equal(aiNeedsRender(ready(callsOf({ blocked: blockOf() })), ready(callsOf())), true, 'freed');
  assert.equal(aiNeedsRender(ready(callsOf({ blocked: blockOf() })), ready(callsOf({ blocked: blockOf({ cap: 'day', limit: 100 }) }))), true, 'another cap');
  assert.equal(aiNeedsRender(ready(callsOf({ blocked: blockOf() })), ready(callsOf({ blocked: blockOf() }))), false);
  assert.equal(aiNeedsRender(ready(), ready(callsOf())), false, 'calls appearing without a block');
});

test('caps: noteCallBlock sets the block and listeners fire on every change (set, replaced, cleared by reset)', () => {
  let fired = 0;
  const off = onCallBlockChange(() => { fired += 1; });
  try {
    assert.equal(callBlock(), null);
    noteCallBlock(blockOf() as AiCallBlock);
    assert.deepEqual(callBlock(), blockOf());
    assert.equal(fired, 1);
    noteCallBlock(blockOf() as AiCallBlock);
    assert.equal(fired, 1, 'the same block again is no change');
    noteCallBlock(blockOf({ cap: 'day', limit: 100 }) as AiCallBlock);
    assert.equal(fired, 2, 'replaced');
    resetAiStatus();
    assert.equal(callBlock(), null);
    assert.equal(fired, 3, 'cleared');
  } finally {
    off();
  }
});

test('caps: a block passed its retryAt is null at once, and the timer clears it and fires the listeners', () => {
  mock.timers.enable({ apis: ['setTimeout', 'Date'], now: 1_000_000 });
  let fired = 0;
  const off = onCallBlockChange(() => { fired += 1; });
  try {
    noteCallBlock({ cap: 'minute', limit: 5, retryAt: 1030 });
    assert.equal(fired, 1);
    assert.equal(callBlock()?.cap, 'minute');
    mock.timers.tick(29_000);
    assert.equal(callBlock()?.cap, 'minute');
    assert.equal(callBlock(1_031_000), null, 'already passed on a later clock');
    mock.timers.tick(1_000);
    assert.equal(callBlock(), null);
    assert.equal(fired, 2, 'the timer cleared it');
  } finally {
    off();
    mock.timers.reset();
  }
});

test('caps: the status sets the block; a status without one clears it only if the read started after the last refusal', async () => {
  let fired = 0;
  const off = onCallBlockChange(() => { fired += 1; });
  try {
    await refreshAiStatus({ mustChangePassword: false, now: 1000, force: true, fetcher: async () => answer({ calls: callsOf({ blocked: blockOf() }) }) });
    assert.deepEqual(callBlock(), blockOf());
    assert.equal(fired, 1);
    // a refusal is noted at 5000; a read that started at 4000 (answer on its way) says "no block": ignored
    noteCallBlock(blockOf({ cap: 'minute', limit: 5 }) as AiCallBlock, 5000);
    assert.equal(fired, 2);
    await refreshAiStatus({ mustChangePassword: false, now: 4000, force: true, fetcher: async () => answer({ calls: callsOf() }) });
    assert.equal(callBlock()?.cap, 'minute', 'an older read must not undo the refusal');
    // a read that started later clears it
    await refreshAiStatus({ mustChangePassword: false, now: 6000, force: true, fetcher: async () => answer({ calls: callsOf() }) });
    assert.equal(callBlock(), null);
    assert.equal(fired, 3);
    // a status without calls at all (no AI+ service) clears like "no block"
    noteCallBlock(blockOf() as AiCallBlock, 7000);
    await refreshAiStatus({ mustChangePassword: false, now: 8000, force: true, fetcher: async () => answer() });
    assert.equal(callBlock(), null);
    // a failed read leaves the block alone
    noteCallBlock(blockOf() as AiCallBlock, 9000);
    await refreshAiStatus({ mustChangePassword: false, now: 9500, force: true, fetcher: async () => { throw new Error('502'); } });
    assert.deepEqual(callBlock(), blockOf());
  } finally {
    off();
  }
});

test('caps: recheckCallBlock forces a read, throttled to one per 5 s; false when the administrator raised the caps', async () => {
  let reads = 0;
  let blocked: unknown = blockOf();
  const fetcher = async (): Promise<unknown> => {
    reads += 1;
    return answer({ calls: callsOf({ blocked }) });
  };
  noteCallBlock(blockOf() as AiCallBlock, 100);
  assert.equal(await recheckCallBlock({ now: 1000, fetcher }), true, 'still blocked');
  assert.equal(reads, 1);
  blocked = null;
  assert.equal(await recheckCallBlock({ now: 1000 + RECHECK_MIN_MS - 1, fetcher }), true, 'within 5 s: answered from the local state');
  assert.equal(reads, 1);
  assert.equal(await recheckCallBlock({ now: 1000 + RECHECK_MIN_MS, fetcher }), false, 'freed by the new caps');
  assert.equal(reads, 2);
  assert.equal(callBlock(), null);
});
