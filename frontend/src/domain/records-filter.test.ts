// The study-record filters: question bank, type (exam / practice), date range, accuracy band; and "is anything filtered".
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  NO_FILTERS, RECORDS_PAGE, filterCount, focusAfterDelete, isFiltered, isKindFilter, matchesRecord, nextLimit, pageRows, type RecordFilters,
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

test('paging: 50 rows first, then 50 more, never past the end (RC-6)', () => {
  const rows = Array.from({ length: 120 }, (_, i) => i);
  assert.equal(RECORDS_PAGE, 50);
  const a = pageRows(rows, RECORDS_PAGE);
  assert.equal(a.shown.length, 50);
  assert.equal(a.rest, 70);
  const l2 = nextLimit(RECORDS_PAGE, rows.length);
  assert.equal(l2, 100);
  assert.equal(pageRows(rows, l2).rest, 20);
  const l3 = nextLimit(l2, rows.length);
  assert.equal(l3, 120);
  assert.equal(pageRows(rows, l3).rest, 0);
  assert.deepEqual(pageRows([1, 2], RECORDS_PAGE), { shown: [1, 2], rest: 0 });
  assert.deepEqual(pageRows([], RECORDS_PAGE), { shown: [], rest: 0 });
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
