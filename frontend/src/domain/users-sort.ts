// User list order and the "active now" dot (v1.3). Pure.
import type { UserRow } from '../types/api.ts';

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
