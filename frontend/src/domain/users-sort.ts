// User list order and the "active now" dot (v1.3). Pure.
import type { UserRow } from '../types/api.ts';
import { filterUsers } from './user-admin.ts';

export type UserSort = 'name' | 'login' | 'active';
export const USER_SORTS: readonly UserSort[] = ['name', 'login', 'active'];
/** A user whose last request is at most this old counts as active now */
export const ONLINE_MS = 5 * 60 * 1000;

/** Server time `YYYY-MM-DD HH:MM[:SS]` (the server clock is UTC) to epoch milliseconds; null if it does not parse */
export function serverMs(at: string | null | undefined): number | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})[ T](\d{2}):(\d{2})(?::(\d{2}))?/.exec(at ?? '');
  if (!m) return null;
  const [y, mo, d, hh, mi, ss] = [m[1], m[2], m[3], m[4], m[5], m[6] ?? '0'].map(Number) as [number, number, number, number, number, number];
  return Date.UTC(y, mo - 1, d, hh, mi, ss);
}

export function isOnline(lastActiveAt: string | null | undefined, nowMs: number): boolean {
  const at = serverMs(lastActiveAt);
  return at != null && nowMs - at <= ONLINE_MS && at - nowMs <= ONLINE_MS;
}

const byName = (a: UserRow, b: UserRow): number => a.username.localeCompare(b.username, undefined, { sensitivity: 'base' }) || a.id - b.id;

/** A new array: name A-Z, or newest first by last sign-in / last activity (never-seen users last, then by name) */
export function sortUsers(list: readonly UserRow[], by: UserSort): UserRow[] {
  const out = [...list];
  if (by === 'name') return out.sort(byName);
  const key = (u: UserRow): number => serverMs(by === 'login' ? u.lastLoginAt : u.lastActiveAt) ?? -Infinity;
  return out.sort((a, b) => {
    const ka = key(a);
    const kb = key(b);
    if (ka === kb) return byName(a, b);
    return ka < kb ? 1 : -1;
  });
}

/** User management shows this many users per page (v2.6) */
export const USERS_PAGE_SIZE = 10;
/** On phones a user card is tall (about two fit on a screen), so a page holds fewer (owner) */
export const USERS_PAGE_SIZE_PHONE = 5;

/** What the list shows: the search text, the order and the 1-based page */
export interface UsersView {
  query: string;
  sort: UserSort;
  page: number;
}

/** Typing a search or picking another order goes back to page 1 */
export const viewQuery = (v: UsersView, query: string): UsersView => ({ ...v, query, page: 1 });
export const viewSort = (v: UsersView, sort: UserSort): UsersView => ({ ...v, sort, page: 1 });

export interface UsersPage {
  items: UserRow[];
  /** Users after the search (before paging) */
  total: number;
  pages: number;
  /** The page actually shown (clamped into 1..pages) */
  page: number;
  from: number;
  to: number;
}

/** Search across all users, then sort, then cut one page; a page past the end (users deleted) is clamped */
export function pageUsers(list: readonly UserRow[], view: UsersView, size = USERS_PAGE_SIZE): UsersPage {
  const sorted = sortUsers(filterUsers(list, view.query), view.sort);
  const pages = Math.max(1, Math.ceil(sorted.length / size));
  const page = Math.min(Math.max(1, Math.floor(view.page) || 1), pages);
  const start = (page - 1) * size;
  const items = sorted.slice(start, start + size);
  return { items, total: sorted.length, pages, page, from: items.length ? start + 1 : 0, to: start + items.length };
}

/** The page that holds user `id` under this search and order (1 when it is not in the list) */
export function pageOfUser(list: readonly UserRow[], view: UsersView, id: number, size = USERS_PAGE_SIZE): number {
  const at = sortUsers(filterUsers(list, view.query), view.sort).findIndex((x) => x.id === id);
  return at < 0 ? 1 : Math.floor(at / size) + 1;
}

/** Route names that belong to User management: the list page, search and order are kept while moving between them (v2.7) */
export const inUsersArea = (name: string): boolean => name === 'users' || name === 'userRecords' || name === 'userRecord';
