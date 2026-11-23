// What an administrator reads of ONE user's study records (v2.7, #/users/<uid>/records[/<id>]). Everything here is separate
// from the own data on purpose: record ids are global, so a target record in S.recordCache would show up as the
// administrator's own record, and S.records / S.progress feed the Home dashboard and the notebook. Nothing here is ever
// written into them. The caches are forgotten when the user-records routes are left or another user is opened.
import { fetchUserRecord, fetchUserRecords } from '../api/users.ts';
import { NO_FILTERS, type RecordFilters } from '../domain/records-filter.ts';
import type { ReviewFilter } from '../components/review.ts';
import type { UserRecordDetail, UserRecordsResponse } from '../types/api.ts';

/** The list page's own UI state (the own page keeps one of these too, in pages/records.ts) */
export interface ListUi {
  filters: RecordFilters;
  /** Current page of the numbered list, 1-based */
  page: number;
  /** Scroll position of the list when a record was opened from it (0 = nothing to restore) */
  listY: number;
  filtersOpen: boolean;
  /** "Wrong only / not completed / all" on the detail page; null = the record's default */
  detailFilter: ReviewFilter | null;
}

export const newListUi = (): ListUi => ({ filters: { ...NO_FILTERS }, page: 1, listY: 0, filtersOpen: false, detailFilter: null });

interface Target {
  uid: number | null;
  list: UserRecordsResponse | null;
  details: Map<string, UserRecordDetail>;
  ui: ListUi;
}

export const UR: Target = { uid: null, list: null, details: new Map(), ui: newListUi() };

const detailKey = (uid: number, rid: number): string => `${uid}:${rid}`;

/** Start over for another user (or for the same one after leaving the view) */
export function resetUserRecords(uid: number | null): void {
  UR.uid = uid;
  UR.list = null;
  UR.details.clear();
  UR.ui = newListUi();
}

/** The view is about `uid`: the same user keeps everything, another one starts empty */
export function enterUserRecords(uid: number): void {
  if (UR.uid !== uid) resetUserRecords(uid);
}

export const clearUserRecords = (): void => resetUserRecords(null);

export async function loadUserRecords(uid: number): Promise<UserRecordsResponse> {
  enterUserRecords(uid);
  if (UR.list) return UR.list;
  const r = await fetchUserRecords(uid);
  // Another user may have been opened while this was on its way: drop the late answer
  if (UR.uid === uid) UR.list = r;
  return r;
}

export async function loadUserRecord(uid: number, rid: number): Promise<UserRecordDetail> {
  enterUserRecords(uid);
  const hit = UR.details.get(detailKey(uid, rid));
  if (hit) return hit;
  const d = await fetchUserRecord(uid, rid);
  if (UR.uid === uid) UR.details.set(detailKey(uid, rid), d);
  return d;
}

export const cachedUserRecord = (uid: number, rid: number): UserRecordDetail | undefined =>
  UR.uid === uid ? UR.details.get(detailKey(uid, rid)) : undefined;
