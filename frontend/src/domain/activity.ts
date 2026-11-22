// Activity log helpers (v1.3; v1.4 address column): the short device label, the one-line detail summary, the filter values and the
// "administrator sign-in ends soon" check. Pure: no DOM, no network (tested in activity.test.ts).
import type { ActivityGroup } from '../types/api.ts';
import { serverMs } from './users-sort.ts';

/** The groups of the action filter, in the order of the menu */
export const ACTIVITY_GROUPS: readonly ActivityGroup[] = ['auth', 'users', 'banks', 'study', 'settings', 'visit', 'ai', 'system'];
/** Auto-refresh period of the activity tab */
export const ACTIVITY_REFRESH_MS = 10_000;
/** The admin sign-in warning starts this long before the hard end */
export const HARD_WARN_MS = 10 * 60 * 1000;

/** `Chrome · Windows` style label from a User-Agent; unknown agents show their first 40 characters; null when there is none */
export function deviceLabel(ua: string | null | undefined): string | null {
  const s = (ua ?? '').trim();
  if (!s) return null;
  let browser: string | null = null;
  if (/\bEdg(e|A|iOS)?\//.test(s)) browser = 'Edge';
  else if (/\bOPR\/|\bOpera\b/.test(s)) browser = 'Opera';
  else if (/\bFirefox\/|\bFxiOS\//.test(s)) browser = 'Firefox';
  else if (/\bChrome\/|\bCriOS\//.test(s)) browser = 'Chrome';
  else if (/\bSafari\//.test(s)) browser = 'Safari';
  let os: string | null = null;
  if (/Windows/.test(s)) os = 'Windows';
  else if (/Android/.test(s)) os = 'Android';
  else if (/iPhone|iPad|iPod/.test(s)) os = 'iOS';
  else if (/Mac OS X|Macintosh/.test(s)) os = 'macOS';
  else if (/CrOS/.test(s)) os = 'ChromeOS';
  else if (/Linux|X11/.test(s)) os = 'Linux';
  const parts = [browser, os].filter((p): p is string => p != null);
  return parts.length ? parts.join(' · ') : s.slice(0, 40);
}

/**
 * The address column of an activity row (v1.4): eye open = `IP · place · device` (a missing IP or device shows "—", a missing
 * place is left out); eye closed = the device alone (it is not an address), "—" when unknown
 */
export function addressText(ip: string | null, place: string | null, device: string | null, eyeOpen: boolean): string {
  if (!eyeOpen) return device ?? '—';
  return [ip ?? '—', place, device ?? '—'].filter((x): x is string => x != null && x !== '').join(' · ');
}

const scalar = (v: unknown): string | null =>
  typeof v === 'string' || typeof v === 'number' || typeof v === 'boolean' ? String(v) : null;

/** A one-line summary of an entry's `detail` (names, hosts, counts, codes only; the server already filtered it) */
export function detailText(detail: Record<string, unknown> | null | undefined): string {
  if (!detail || typeof detail !== 'object') return '';
  const from = scalar(detail.oldHost);
  const to = scalar(detail.newHost);
  if (from && to) return `${from} → ${to}`;
  const parts: string[] = [];
  for (const [k, v] of Object.entries(detail)) {
    const text = Array.isArray(v) ? v.map(scalar).filter((x): x is string => x != null).join(', ') : scalar(v);
    if (text) parts.push(`${k}: ${text}`);
  }
  const out = parts.join('; ');
  return out.length > 160 ? `${out.slice(0, 159)}…` : out;
}

/** The user filter value of the select: '' = everyone, 'system', 'anon' or a user id as text */
export function userFilter(v: string): number | 'system' | 'anon' | undefined {
  if (v === 'system' || v === 'anon') return v;
  const n = Number(v);
  return v !== '' && Number.isInteger(n) && n > 0 ? n : undefined;
}

/** The action select value: `g:<group>` or `a:<code>` (a code is only used by the visit entry, `page.view`) */
export function actionFilter(v: string): { group?: string; action?: string } {
  if (v.startsWith('g:')) return { group: v.slice(2) };
  if (v.startsWith('a:')) return { action: v.slice(2) };
  return {};
}

/** Whole minutes (rounded up) left until `hardEndsAt` when that is within 10 minutes and not over; else null */
export function hardSoonMinutes(hardEndsAt: string | null | undefined, nowMs: number): number | null {
  const end = serverMs(hardEndsAt);
  if (end == null) return null;
  const left = end - nowMs;
  if (left <= 0 || left > HARD_WARN_MS) return null;
  return Math.max(1, Math.ceil(left / 60_000));
}

/** Entries per page of the activity list (the entries are paged on the client, within what has been read) */
export const ACTIVITY_PAGE_SIZE = 20;

/** One page of `total` loaded entries: the page is clamped into 1..pages (an empty list is one empty page); from / to are 1-based, 0 when empty */
export function activityPage(total: number, page: number, size = ACTIVITY_PAGE_SIZE): { page: number; pages: number; start: number; end: number; from: number; to: number } {
  const pages = Math.max(1, Math.ceil(total / size));
  const p = Math.min(Math.max(1, Math.trunc(page) || 1), pages);
  const start = (p - 1) * size;
  const end = Math.min(total, start + size);
  return { page: p, pages, start, end, from: total ? start + 1 : 0, to: end };
}

/** New entries (read with `after`) in front of the list, no duplicates, newest first */
export function mergeNewer<T extends { id: number }>(list: readonly T[], fresh: readonly T[]): T[] {
  const have = new Set(list.map((e) => e.id));
  return [...fresh.filter((e) => !have.has(e.id)), ...list].sort((a, b) => b.id - a.id);
}

const PRIVATE_IP = /^(10\.|127\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.|::1$|f[cd][0-9a-f]{2}:)/i;

/** The IPs of a page of entries are all one private / loopback address (at least 3): the proxy's, not the visitors' */
export function looksLikeProxy(ips: ReadonlyArray<string | null>): boolean {
  const seen = ips.filter((x): x is string => !!x);
  return seen.length >= 3 && seen.every((x) => x === seen[0]) && PRIVATE_IP.test(seen[0] ?? '');
}
