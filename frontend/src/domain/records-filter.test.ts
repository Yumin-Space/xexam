// The study-record filters: question bank, type (exam / practice), date range, accuracy band; and "is anything filtered".
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  NO_FILTERS, filterCount, focusAfterDelete, isFiltered, isKindFilter, matchesRecord, pageRecords, recordsPageSize, type RecordFilters,
} from './records-filter.ts';
import type { RecordSummary } from '../types/api.ts';

type Row = Pick<RecordSummary, 'banks' | 'at' | 'percent' | 'kind'>;
const rec = (at: string, percent: number, kind: Row['kind'], bankIds: number[] = [1]): Row => ({
  at, percent, kind, banks: bankIds.map((id) => ({ id, name: `Demo ${id}`, deleted: false })),
});
const rows: Row[] = [
  rec('2026-11-01 09:00:00', 50, 'exam'),
  rec('2026-11-02 10:00:00', 70, 'practice', [2]),
  rec('2026-11-03 11:00:00', 90, 'practice', [1, 2]),
  rec('2026-11-04 12:00:00', 85, undefined), // a record from before v0.6.5: an exam
];
const pick = (f: Partial<RecordFilters>): number[] => rows.flatMap((r, i) => (matchesRecord(r, { ...NO_FILTERS, ...f }) ? [i] : []));

test('no filter matches everything and is not "filtered"', () => {
  assert.deepEqual(pick({}), [0, 1, 2, 3]);
  assert.equal(isFiltered(NO_FILTERS), false);
});

test('type filter: exams (old records without a kind included) or practice', () => {
  assert.deepEqual(pick({ kind: 'exam' }), [0, 3]);
  assert.deepEqual(pick({ kind: 'practice' }), [1, 2]);
  assert.equal(isFiltered({ ...NO_FILTERS, kind: 'practice' }), true);
});

test('type filter combines with bank, dates and accuracy', () => {
  assert.deepEqual(pick({ kind: 'practice', bank: '1' }), [2]);
  assert.deepEqual(pick({ kind: 'practice', score: 'high' }), [2]);
  assert.deepEqual(pick({ kind: 'exam', from: '2026-11-02' }), [3]);
  assert.deepEqual(pick({ to: '2026-11-02' }), [0, 1]);
  assert.deepEqual(pick({ score: 'low' }), [0]);
  assert.deepEqual(pick({ score: 'mid' }), [1]);
});

test('isKindFilter accepts only all / exam / practice', () => {
  for (const v of ['all', 'exam', 'practice']) assert.equal(isKindFilter(v), true);
  for (const v of ['', 'Exam', 'practise', null, 1]) assert.equal(isKindFilter(v), false);
});

test('paging: 10 rows per phone page, 20 elsewhere, the page is clamped (v2.7)', () => {
  const rows = Array.from({ length: 45 }, (_, i) => i);
  assert.equal(recordsPageSize(true), 10);
  assert.equal(recordsPageSize(false), 20);
  const a = pageRecords(rows, 1, 20);
  assert.equal(a.items.length, 20);
  assert.deepEqual([a.page, a.pages, a.from, a.to, a.total], [1, 3, 1, 20, 45]);
  const c = pageRecords(rows, 3, 20);
  assert.deepEqual([c.items.length, c.from, c.to], [5, 41, 45]);
  // Past the end (the last row of the last page was deleted): the last page
  assert.equal(pageRecords(rows, 9, 20).page, 3);
  assert.equal(pageRecords(rows, 0, 20).page, 1);
  assert.equal(pageRecords(rows, Number.NaN, 20).page, 1);
  const e = pageRecords([], 4, 10);
  assert.deepEqual([e.items.length, e.page, e.pages, e.from, e.to], [0, 1, 1, 0, 0]);
  assert.equal(pageRecords(rows, 1, 10).pages, 5);
});

test('focus after a delete: the next row, else the previous, else nothing (RC-11)', () => {
  assert.equal(focusAfterDelete([5, 6, 7], 5), 6);
  assert.equal(focusAfterDelete([5, 6, 7], 6), 7);
  assert.equal(focusAfterDelete([5, 6, 7], 7), 6);
  assert.equal(focusAfterDelete([5], 5), null);
  assert.equal(focusAfterDelete([5, 6], 9), null);
});

test('filter count: one per active field, none by default (RC-1)', () => {
  assert.equal(filterCount(NO_FILTERS), 0);
  assert.equal(filterCount({ ...NO_FILTERS, kind: 'exam' }), 1);
  assert.equal(filterCount({ bank: '3', kind: 'practice', from: '2026-11-01', to: '2026-11-30', score: 'low' }), 5);
  assert.equal(filterCount({ ...NO_FILTERS, from: '2026-11-01' }), 1);
});
