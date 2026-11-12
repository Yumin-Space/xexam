// The filters of the study-record list (#/records): question bank, type (exam / practice, v0.6.5), date range and
// accuracy band. Pure, so the matching rules are tested on their own (records-filter.test.ts); the page keeps the current
// values and the overview numbers and bars are computed from the records that match.
import type { RecordSummary } from '../types/api.ts';
import { kindOf } from './attempt.ts';

export type ScoreBand = 'any' | 'low' | 'mid' | 'high';
/** Which records: all of them, only exams, or only practice */
export type KindFilter = 'all' | 'exam' | 'practice';

export interface RecordFilters {
  /** 'all' or a bank id as a string */
  bank: string;
  kind: KindFilter;
  /** "YYYY-MM-DD" or '' */
  from: string;
  to: string;
  score: ScoreBand;
}

export const NO_FILTERS: Readonly<RecordFilters> = { bank: 'all', kind: 'all', from: '', to: '', score: 'any' };

export function isKindFilter(v: unknown): v is KindFilter {
  return v === 'all' || v === 'exam' || v === 'practice';
}

/** Whether any filter narrows the list */
export function isFiltered(f: Readonly<RecordFilters>): boolean {
  return f.bank !== 'all' || f.kind !== 'all' || !!f.from || !!f.to || f.score !== 'any';
}

/** How many of the five filters narrow the list (the "筛选 (n)" count of the phone filter bar, RC-1) */
export function filterCount(f: Readonly<RecordFilters>): number {
  // "From" and "To" are two fields of the grid, so each counts on its own
  return (f.bank !== 'all' ? 1 : 0) + (f.kind !== 'all' ? 1 : 0) + (f.from ? 1 : 0) + (f.to ? 1 : 0) + (f.score !== 'any' ? 1 : 0);
}

type Row = Pick<RecordSummary, 'banks' | 'at' | 'percent' | 'kind'>;

export function matchesRecord(r: Row, f: Readonly<RecordFilters>): boolean {
  if (f.bank !== 'all' && !r.banks.some((b) => String(b.id) === f.bank)) return false;
  if (f.kind !== 'all' && kindOf(r) !== f.kind) return false;
  const day = r.at.slice(0, 10);
  if (f.from && day < f.from) return false;
  if (f.to && day > f.to) return false;
  switch (f.score) {
    case 'low':
      return r.percent < 60;
    case 'mid':
      return r.percent >= 60 && r.percent < 80;
    case 'high':
      return r.percent >= 80;
    default:
      return true;
  }
}

// ---------------------------------------------------------------- paging (v1.0, RC-6)

/** How many records the list shows at first, and how many each "show more" adds */
export const RECORDS_PAGE = 50;

/** The first `limit` rows and how many are left out */
export function pageRows<T>(rows: readonly T[], limit: number): { shown: T[]; rest: number } {
  const n = Math.max(0, Math.floor(limit));
  return { shown: rows.slice(0, n), rest: Math.max(0, rows.length - n) };
}

/** The limit after one "show more" tap (never beyond what exists, so the label count stays exact) */
export function nextLimit(limit: number, total: number): number {
  return Math.min(Math.max(total, 0), Math.max(0, limit) + RECORDS_PAGE);
}

/**
 * Where the focus goes after a record of the shown rows was deleted (RC-11): the next row, else the previous one, else
 * null (the page puts it on the list heading). `ids` are the shown rows' ids in display order.
 */
export function focusAfterDelete(ids: readonly number[], deleted: number): number | null {
  const i = ids.indexOf(deleted);
  if (i < 0) return null;
  const next = ids[i + 1];
  if (next !== undefined) return next;
  const prev = ids[i - 1];
  return prev !== undefined ? prev : null;
}
