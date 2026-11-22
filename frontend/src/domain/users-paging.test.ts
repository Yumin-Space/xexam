import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { UserRow } from '../types/api.ts';
import { USERS_PAGE_SIZE, USERS_PAGE_SIZE_PHONE, pageOfUser, pageUsers, viewQuery, viewSort, type UsersView } from './users-sort.ts';

const many = (n: number): UserRow[] =>
  Array.from({ length: n }, (_, i) => ({ id: i + 1, username: `user${String(i + 1).padStart(3, '0')}@example.com`, displayName: i % 2 ? 'Odd' : null } as UserRow));
const view = (page: number, query = ''): UsersView => ({ query, sort: 'name', page });

test('10 users per page (5 on phones), the last page holds the rest', () => {
  const list = many(45);
  assert.equal(USERS_PAGE_SIZE, 10);
  assert.equal(USERS_PAGE_SIZE_PHONE, 5);
  const p1 = pageUsers(list, view(1));
  assert.deepEqual([p1.items.length, p1.pages, p1.from, p1.to, p1.total], [10, 5, 1, 10, 45]);
  const p5 = pageUsers(list, view(5));
  assert.deepEqual([p5.items.length, p5.from, p5.to], [5, 41, 45]);
  assert.equal(p5.items[0]?.id, 41);
  // the phone size
  const ph = pageUsers(list, view(9), USERS_PAGE_SIZE_PHONE);
  assert.deepEqual([ph.items.length, ph.pages, ph.from, ph.to], [5, 9, 41, 45]);
});

test('search works across all users, not only the shown page', () => {
  const r = pageUsers(many(45), view(1, 'user044'));
  assert.deepEqual(r.items.map((u) => u.id), [44]);
  assert.equal(r.pages, 1);
  assert.equal(pageUsers(many(45), view(1, 'odd')).total, 22);
});

test('a new search or order goes back to page 1', () => {
  assert.equal(viewQuery(view(3), 'x').page, 1);
  assert.equal(viewSort(view(3), 'login').page, 1);
  assert.equal(viewSort(view(3), 'login').sort, 'login');
});

test('the page is clamped when users are gone or the number is odd', () => {
  assert.equal(pageUsers(many(11), view(2)).page, 2);
  assert.equal(pageUsers(many(10), view(2)).page, 1);
  assert.equal(pageUsers([], view(5)).page, 1);
  assert.equal(pageUsers(many(45), view(0)).page, 1);
  assert.equal(pageUsers(many(45), view(Number.NaN)).page, 1);
  assert.equal(pageUsers([], view(1)).from, 0);
});

test('the page of one user follows the search and the order', () => {
  const list = many(45);
  assert.equal(pageOfUser(list, view(1), 1), 1);
  assert.equal(pageOfUser(list, view(1), 15), 2);
  assert.equal(pageOfUser(list, view(1), 45), 5);
  assert.equal(pageOfUser(list, view(1), 45, USERS_PAGE_SIZE_PHONE), 9);
  assert.equal(pageOfUser(list, view(1, 'user045'), 45), 1);
  assert.equal(pageOfUser(list, view(1), 999), 1);
});
