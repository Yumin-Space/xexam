// What the signed-in user may do with AI (v0.8): 'loading' until the first answer, then 'ready' or (any failure)
// 'unavailable'; at most every 60 s unless forced; never while a new password must be chosen; an answer for the previous
// user is dropped; and when the page must be drawn again. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { beforeEach, test } from 'node:test';
import {
  AI_STATUS_MAX_AGE_MS, aiBadgeOn, aiChatVisible, aiEnabled, aiFreeMode, aiModeShown, aiNeedsRender, aiStatus, aiSuspended, refreshAiStatus, resetAiStatus, setAiFreeMode, statusOf,
  type AiStatus,
} from './ai.ts';

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
