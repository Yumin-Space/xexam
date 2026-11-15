import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { UserRow } from '../types/api.ts';
import { ONLINE_MS, isOnline, serverMs, sortUsers } from './users-sort.ts';

const u = (id: number, username: string, lastLoginAt: string | null, lastActiveAt: string | null): UserRow =>
  ({ id, username, lastLoginAt, lastActiveAt } as UserRow);

const list = [
  u(1, 'bob', '2026-01-02 10:00', '2026-01-03 09:00:00'),
  u(2, 'Ann', null, null),
  u(3, 'cy', '2026-01-05 10:00', '2026-01-01 09:00:00'),
  u(4, 'dee', null, null),
];

test('sort by name ignores case', () => {
  assert.deepEqual(sortUsers(list, 'name').map((x) => x.id), [2, 1, 3, 4]);
});

test('sort by login and activity: newest first, never last, ties by name', () => {
  assert.deepEqual(sortUsers(list, 'login').map((x) => x.id), [3, 1, 2, 4]);
  assert.deepEqual(sortUsers(list, 'active').map((x) => x.id), [1, 3, 2, 4]);
});

test('sorting does not change the input', () => {
  const copy = list.map((x) => x.id);
  sortUsers(list, 'active');
  assert.deepEqual(list.map((x) => x.id), copy);
});

test('serverMs parses minutes and seconds, rejects junk', () => {
  assert.equal(serverMs('2026-01-01 00:01'), Date.UTC(2026, 0, 1, 0, 1));
  assert.equal(serverMs('2026-01-01 00:01:30'), Date.UTC(2026, 0, 1, 0, 1, 30));
  assert.equal(serverMs('x'), null);
  assert.equal(serverMs(null), null);
});

test('isOnline: within five minutes only', () => {
  const now = Date.UTC(2026, 0, 1, 12, 0, 0);
  assert.equal(isOnline('2026-01-01 11:56:00', now), true);
  assert.equal(isOnline('2026-01-01 11:54:00', now), false);
  assert.equal(isOnline(null, now), false);
  assert.equal(ONLINE_MS, 300000);
});
