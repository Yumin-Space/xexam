import assert from 'node:assert/strict';
import { test } from 'node:test';
import { clickSort, matchesQuery, pageList, pageTable, withQuery, type TableState } from './paged-table.ts';

type R = { n: string; c: number };
const rows: R[] = Array.from({ length: 23 }, (_, i) => ({ n: `user${String(i + 1).padStart(2, '0')}`, c: (i * 7) % 11 }));
const opts = {
  name: (r: R) => r.n,
  cmp: { user: (a: R, b: R) => a.n.localeCompare(b.n), calls: (a: R, b: R) => a.c - b.c },
};
const st = (o: Partial<TableState> = {}): TableState => ({ query: '', key: 'user', dir: 'asc', page: 1, ...o });

test('ten per page, last page partial, page clamped', () => {
  assert.equal(pageTable(rows, st(), opts).items.length, 10);
  const last = pageTable(rows, st({ page: 3 }), opts);
  assert.deepEqual([last.items.length, last.from, last.to, last.pages], [3, 21, 23, 3]);
  assert.equal(pageTable(rows, st({ page: 99 }), opts).page, 3);
});

test('search runs before paging; empty result has one page', () => {
  const r = pageTable(rows, st({ query: 'USER1' }), opts);
  assert.equal(r.total, 10);
  const none = pageTable(rows, st({ query: 'zzz' }), opts);
  assert.deepEqual([none.total, none.pages, none.page, none.from, none.to], [0, 1, 1, 0, 0]);
  assert.ok(matchesQuery('Alice Bob', 'bo al'));
  assert.ok(matchesQuery('x', '   '));
});

test('sort runs before paging, desc reverses, ties by name', () => {
  const d = pageTable(rows, st({ key: 'calls', dir: 'desc' }), opts);
  assert.equal(d.items[0]!.c, 10);
  const cs = d.items.map((r) => r.c);
  assert.deepEqual(cs, [...cs].sort((a, b) => b - a));
  const asc = pageTable(rows, st({ dir: 'desc' }), opts);
  assert.equal(asc.items[0]!.n, 'user23');
});

test('click sort and search go back to page 1', () => {
  const s = st({ page: 3 });
  assert.deepEqual(clickSort(s, 'calls', false), { query: '', key: 'calls', dir: 'desc', page: 1 });
  assert.deepEqual(clickSort(s, 'user', true), { query: '', key: 'user', dir: 'desc', page: 1 });
  assert.deepEqual(clickSort(clickSort(s, 'calls', false), 'calls', false), { query: '', key: 'calls', dir: 'asc', page: 1 });
  assert.equal(withQuery(s, 'a').page, 1);
});

test('page list', () => {
  assert.deepEqual(pageList(1, 1), [1]);
  assert.deepEqual(pageList(1, 5), [1, 2, 'gap', 5]);
  assert.deepEqual(pageList(3, 5), [1, 2, 3, 4, 5]);
  assert.deepEqual(pageList(5, 12), [1, 'gap', 4, 5, 6, 'gap', 12]);
  assert.deepEqual(pageList(5, 12, 0), [1, 'gap', 5, 'gap', 12]);
});
