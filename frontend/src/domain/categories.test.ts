// Bank categories: grouping order, the tri-state of a category box and moving one place in a list (v3.1).
import assert from 'node:assert/strict';
import test from 'node:test';
import type { BankCategory } from '../types/api.ts';
import { groupBanks, moveInList, triState } from './categories.ts';

const bank = (id: number, categoryId: number | null, sort = 0): { id: number; categoryId: number | null; sort: number } => ({ id, categoryId, sort });
const cat = (id: number, sort: number): BankCategory => ({ id, name: `C${id}`, sort });
const ids = (banks: Array<{ id: number }>): number[] => banks.map((b) => b.id);

test('groupBanks gives one plain group when no category exists', () => {
  const r = groupBanks([bank(3, null), bank(1, null)], []);
  assert.equal(r.hasCategories, false);
  assert.equal(r.groups.length, 1);
  assert.equal(r.groups[0]!.category, null);
  assert.deepEqual(ids(r.groups[0]!.banks), [3, 1]);
});

test('groupBanks orders categories by (sort, id) and appends Uncategorised only when not empty', () => {
  const r = groupBanks([bank(1, 20), bank(2, 10), bank(3, null)], [cat(20, 1), cat(10, 1), cat(30, 0)]);
  assert.equal(r.hasCategories, true);
  assert.deepEqual(r.groups.map((g) => g.category?.id ?? null), [30, 10, 20, null]);
  assert.deepEqual(r.groups.map((g) => ids(g.banks)), [[], [2], [1], [3]]);
  const none = groupBanks([bank(1, 10)], [cat(10, 0)]);
  assert.deepEqual(none.groups.map((g) => g.category?.id), [10]);
});

test('groupBanks puts unplaced banks (sort 0) after the placed ones, by id', () => {
  const r = groupBanks([bank(5, 1, 0), bank(2, 1, 2), bank(9, 1, 1), bank(3, 1, 0)], [cat(1, 0)]);
  assert.deepEqual(ids(r.groups[0]!.banks), [9, 2, 3, 5]);
});

test('groupBanks treats a bank of an unknown category as uncategorised and keeps extra fields', () => {
  const r = groupBanks([{ ...bank(1, 99), name: 'x' }], [cat(1, 0)]);
  assert.equal(r.groups[1]!.category, null);
  assert.equal(r.groups[1]!.banks[0]!.name, 'x');
});

test('triState reports all, some and none', () => {
  assert.equal(triState(['a', 'b'], new Set(['a', 'b', 'z'])), 'all');
  assert.equal(triState(['a', 'b'], new Set(['a'])), 'some');
  assert.equal(triState(['a', 'b'], new Set(['z'])), 'none');
  assert.equal(triState([], new Set(['a'])), 'none');
});

test('moveInList moves one place and stops at the ends', () => {
  assert.deepEqual(moveInList([1, 2, 3], 2, -1), [2, 1, 3]);
  assert.deepEqual(moveInList([1, 2, 3], 2, 1), [1, 3, 2]);
  assert.deepEqual(moveInList([1, 2, 3], 1, -1), [1, 2, 3]);
  assert.deepEqual(moveInList([1, 2, 3], 3, 1), [1, 2, 3]);
  assert.deepEqual(moveInList([1, 2, 3], 9, 1), [1, 2, 3]);
});

test('moveInList does not change its input', () => {
  const a = [1, 2];
  moveInList(a, 1, 1);
  assert.deepEqual(a, [1, 2]);
});
