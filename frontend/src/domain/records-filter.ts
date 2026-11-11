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
