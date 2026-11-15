// User management's AI+ rules (v0.8 round 2; contracts §2.8), without DOM so they run under `node --test`.
// The per-user "AI+ service" switch is shown, and can be used, only while AI+ is available: ai-service is running and
// connected to the main service (a report younger than 60 s) and an administrator turned the AI+ service on in AI
// settings. `aiAvailable` comes with every answer of GET /api/users and POST /api/users/*. Otherwise the rows have no AI+
// line and the New user form no AI+ box; the stored switches stay as they are. A page drawn before AI+ went away that
// still tries is refused by the backend (409, code `ai.serviceOff`): no success message then, only the refusal.
import type { Role } from '../types/api.ts';
import { untilMinute } from './lock.ts';

/** The code (and the backend's Chinese text) of the refusal while AI+ is not available */
export const AI_SERVICE_OFF_CODE = 'ai.serviceOff';
export const MSG_AI_SERVICE_OFF = '请先在 AI 设置里开启 AI+ 功能。';

/** The AI+ line of a row: none while AI+ is not available; a switch for a plain user; a note for an administrator */
export type AiLineKind = 'none' | 'switch' | 'admin';

export function aiLineKind(aiAvailable: boolean, role: Role): AiLineKind {
  if (!aiAvailable) return 'none';
  return role === 'admin' ? 'admin' : 'switch';
}

/** Whether a plain user's AI+ service switch (row) or box (New user form) is drawn */
export const aiSwitchShown = (aiAvailable: boolean, role: Role): boolean => aiLineKind(aiAvailable, role) === 'switch';

/** The `ai` of a create body: true only when the AI+ box is drawn (AI+ available, role user) and ticked */
export const createAi = (aiAvailable: boolean, role: Role, ticked: boolean): boolean => aiSwitchShown(aiAvailable, role) && ticked;

/** The refusal "Turn on the AI+ service in AI settings first" (an ApiError, or anything with that code or text) */
export function isServiceOff(e: unknown): boolean {
  if (typeof e !== 'object' || e === null) return false;
  const x = e as { code?: unknown; serverMessage?: unknown };
  return x.code === AI_SERVICE_OFF_CODE || x.serverMessage === MSG_AI_SERVICE_OFF;
}

/**
 * The end of a sign-in lock as the row's tag shows it (v1.0, US-10): `HH:MM`, or `MM-DD HH:MM` when the lock ends on another
 * day than it started (it lasts at most a day). `until` is the server's `YYYY-MM-DD HH:MM:SS`; `retryAfter` the seconds the
 * lock had left when the list arrived. Only calendar arithmetic is used, no time zone is involved.
 */
export function lockUntilLabel(until: string, retryAfter: number): string {
  const at = untilMinute(until);
  const m = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2})/.exec(at);
  if (!m) return at;
  const [y, mo, d, hh, mi] = m.slice(1, 6).map(Number) as [number, number, number, number, number];
  const start = new Date(Date.UTC(y, mo - 1, d, hh, mi) - Math.max(0, retryAfter) * 1000);
  const sameDay = start.getUTCFullYear() === y && start.getUTCMonth() === mo - 1 && start.getUTCDate() === d;
  return sameDay ? `${m[4]}:${m[5]}` : `${m[2]}-${m[3]} ${m[4]}:${m[5]}`;
}

/** `YYYY-MM-DD HH:MM[:SS]` with the full year and without the seconds ("2026-11-12 01:46"), for the one-line facts of a row */
export const fullAt = (at: string): string => at.slice(0, 16);

/** More users than this get a search field (US-11) */
export const SEARCH_FROM = 10;

/** The users whose name contains `query` (case-insensitive, trimmed); everyone for an empty query */
export function filterUsers<T extends { username: string }>(list: readonly T[], query: string): T[] {
  const q = query.trim().toLowerCase();
  return q ? list.filter((u) => u.username.toLowerCase().includes(q)) : [...list];
}

/** The kinds of a row's fact lines (v1.4), in the order they are drawn */
export type FactKind = 'lastLogin' | 'loginIp' | 'loginPlace' | 'lastActive' | 'activeIp' | 'activePlace' | 'activeDevice' | 'records';

/** The facts a row can show (a subset of `UserRow`) */
export interface FactSource {
  lastActiveAt?: string | null | undefined;
  lastLoginIp?: string | null | undefined;
  lastActiveIp?: string | null | undefined;
  lastActiveUa?: string | null | undefined;
}

/**
 * Which fact lines a user row draws, one fact per line: last sign-in, its IP and place, last active, its IP and place, the
 * last active device, the record count. The IP / place lines need the eye open and a recorded IP; a never-active user has no
 * activity lines; the device line ignores the eye (a device is not an address) and needs a user agent.
 */
export function factPlan(u: FactSource, eyeOpen: boolean): FactKind[] {
  const plan: FactKind[] = ['lastLogin'];
  if (eyeOpen && u.lastLoginIp) plan.push('loginIp', 'loginPlace');
  plan.push('lastActive');
  if (u.lastActiveAt) {
    if (eyeOpen && u.lastActiveIp) plan.push('activeIp', 'activePlace');
    if ((u.lastActiveUa ?? '').trim() !== '') plan.push('activeDevice');
  }
  plan.push('records');
  return plan;
}
