import assert from 'node:assert/strict';
import test from 'node:test';
import { lockNote, lockView, untilMinute } from './lock.ts';

import type { UserLock } from '../types/api.ts';

const lock = (o: Partial<UserLock> = {}): UserLock => ({
  until: null, retryAfter: 0, round: 2, triesLeft: 3, knownBrowser: false, ...o,
});

test('a fresh account shows nothing and offers no Unlock', () => {
  assert.deepEqual(lockView(null, 0), { state: 'fresh', triesLeft: 0, knownBrowser: false, unlock: false });
});

test('a running lock offers Unlock and says whose browsers it counts', () => {
  assert.deepEqual(lockView(lock({ retryAfter: 60, round: 1, triesLeft: 0 }), 42),
    { state: 'locked', triesLeft: 0, knownBrowser: false, unlock: true });
  assert.equal(lockView(lock({ retryAfter: 300, knownBrowser: true }), 300).knownBrowser, true);
});

test('past round 1 without a running lock: tries left, and Unlock is offered before the next lock too', () => {
  assert.deepEqual(lockView(lock({ triesLeft: 2 }), 0), { state: 'tries', triesLeft: 2, knownBrowser: false, unlock: true });
  assert.equal(lockView(lock({ triesLeft: 1, knownBrowser: true }), 0).knownBrowser, true);
});

test('a lock that ended on the countdown shows as fresh until the list is read again', () => {
  assert.deepEqual(lockView(lock({ retryAfter: 60, round: 1, triesLeft: 0 }), 0),
    { state: 'fresh', triesLeft: 0, knownBrowser: false, unlock: false });
});

// ---------------------------------------------------------------- the visible note (v0.6.5 re-review)

const who = (o: Partial<{ name: string; enabled: boolean; mayHaveKnownBrowsers: boolean }> = {}) => ({
  name: 'bob', enabled: true, mayHaveKnownBrowsers: true, ...o,
});
const keys = (parts: { key: string }[]) => parts.map((p) => p.key);

test('the end of a lock is shown to the minute, rounded up, never before the real end', () => {
  assert.equal(untilMinute('2026-11-11 06:46:34'), '2026-11-11 06:47');
  assert.equal(untilMinute('2026-11-11 06:46:00'), '2026-11-11 06:46');
  assert.equal(untilMinute('2026-12-31 23:59:01'), '2027-01-01 00:00');
  assert.equal(untilMinute('2028-02-28 23:59:30'), '2028-02-29 00:00');
  assert.equal(untilMinute('2026-11-11 06:46'), '2026-11-11 06:46');
  assert.equal(untilMinute(''), '');
});

test('a fresh state has no note', () => {
  assert.deepEqual(lockNote(null, lockView(null, 0), who()), []);
});

test('a lock of the other browsers says the known browsers still work only when that is sure', () => {
  const l = lock({ until: '2026-11-11 06:46:34', retryAfter: 60, round: 2, triesLeft: 0 });
  const v = lockView(l, 60);
  const parts = lockNote(l, v, who());
  assert.deepEqual(keys(parts), ['users.lock.other', 'users.lock.streak', 'users.lock.knownStillWork', 'users.lock.unlock']);
  assert.deepEqual(parts[0]?.params, { name: 'bob', at: '2026-11-11 06:47' });
  assert.equal(parts[1]?.n, 2);
  // never signed in (or a new password set by an administrator since), or disabled: no such claim
  for (const w of [who({ mayHaveKnownBrowsers: false }), who({ enabled: false })]) {
    assert.deepEqual(keys(lockNote(l, v, w)), ['users.lock.other', 'users.lock.streak', 'users.lock.unlock']);
  }
});

test('a lock of the known browsers says the others are counted separately', () => {
  const l = lock({ until: '2026-11-11 06:46:00', retryAfter: 60, round: 1, triesLeft: 0, knownBrowser: true });
  assert.deepEqual(keys(lockNote(l, lockView(l, 60), who())),
    ['users.lock.known', 'users.lock.streak', 'users.lock.othersSeparate', 'users.lock.unlock']);
});

test('both locked: the note says every browser is locked, with each end', () => {
  const l = lock({ until: '2026-11-11 06:50:10', retryAfter: 300, round: 2, triesLeft: 0, otherUntil: '2026-11-11 06:46:00', otherRetryAfter: 60 });
  const parts = lockNote(l, lockView(l, 300), who());
  assert.deepEqual(keys(parts), ['users.lock.both', 'users.lock.unlock']);
  // the shown (longer) lock is the other browsers'; the known browsers' one is otherUntil
  assert.deepEqual(parts[0]?.params, { name: 'bob', known: '2026-11-11 06:46', other: '2026-11-11 06:51' });
  const k = lock({ ...l, knownBrowser: true });
  assert.deepEqual(lockNote(k, lockView(k, 300), who())[0]?.params, { name: 'bob', known: '2026-11-11 06:51', other: '2026-11-11 06:46' });
  // both ending in the same minute: one time
  const same = lock({ ...l, otherUntil: '2026-11-11 06:50:05' });
  assert.deepEqual(lockNote(same, lockView(same, 300), who()),
    [{ key: 'users.lock.bothSame', params: { name: 'bob', at: '2026-11-11 06:51' } }, { key: 'users.lock.unlock', params: {} }]);
});

test('tries left: which browsers, and what gives the tries back', () => {
  const l = lock({ triesLeft: 2 });
  const v = lockView(l, 0);
  const parts = lockNote(l, v, who());
  assert.deepEqual(keys(parts), ['users.tries.other', 'users.tries.knownUnaffected', 'users.tries.resetOther']);
  assert.equal(parts[0]?.n, 2);
  assert.deepEqual(keys(lockNote(l, v, who({ mayHaveKnownBrowsers: false }))), ['users.tries.other', 'users.tries.resetOther']);
  // a disabled account cannot sign in: only Unlock gives the tries back
  assert.deepEqual(keys(lockNote(l, v, who({ enabled: false }))), ['users.tries.other', 'users.tries.resetUnlock']);
  const k = lock({ triesLeft: 1, knownBrowser: true });
  assert.deepEqual(keys(lockNote(k, lockView(k, 0), who())), ['users.tries.known', 'users.tries.resetKnown']);
});
