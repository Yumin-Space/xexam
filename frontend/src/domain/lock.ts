// What User management shows about a user's sign-in throttle (the backend's "lock" of GET /api/users; see the escalating login
// lock in docs/en/security.md). Pure: the page turns it into tags, the visible note under the row and the Unlock action.

import type { MsgKey } from '../i18n/index.ts';
import type { UserLock } from '../types/api.ts';

export interface LockView {
  /** 'fresh': round 1 and not locked (nothing to show); 'locked': a lock runs; 'tries': past round 1, no lock right now */
  state: 'fresh' | 'locked' | 'tries';
  /** Failures left before the next lock (state 'tries' only, else 0) */
  triesLeft: number;
  /** The state counts sign-ins from browsers where the user signed in before (else from every other browser) */
  knownBrowser: boolean;
  /**
   * Offer Unlock: whenever the state is not fresh, so an administrator can give the five tries back before the next lock
   * too (owner, v0.6.5)
   */
  unlock: boolean;
}

/**
 * The view of `lock` when `left` seconds of its lock remain on this page's countdown (0 = no lock running any more). A lock
 * that ended on the countdown while the list was not read again yet shows as fresh: the next round only becomes known when
 * the list is read again (the page does that when a lock ends).
 */
export function lockView(lock: UserLock | null, left: number): LockView {
  const knownBrowser = !!lock?.knownBrowser;
  if (lock && left > 0) return { state: 'locked', triesLeft: 0, knownBrowser, unlock: true };
  if (lock && lock.retryAfter === 0 && lock.round > 1) {
    return { state: 'tries', triesLeft: Math.max(0, lock.triesLeft), knownBrowser, unlock: true };
  }
  return { state: 'fresh', triesLeft: 0, knownBrowser: false, unlock: false };
}

/**
 * `until` (`YYYY-MM-DD HH:MM:SS`, the server's local time) shown to the minute, rounded **up**: the time shown is never before
 * the real end of the lock (06:46:34 shows as 06:47). Anything else is cut to its first 16 characters.
 */
export function untilMinute(until: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})/.exec(until);
  if (!m || m[6] === '00') return until.slice(0, 16);
  const [y, mo, day, hh, mi] = m.slice(1, 6).map(Number) as [number, number, number, number, number];
  // Date.UTC only does the carry (minute → hour → day → month → year); no time zone is involved
  const d = new Date(Date.UTC(y, mo - 1, day, hh, mi + 1));
  const two = (n: number): string => String(n).padStart(2, '0');
  return `${d.getUTCFullYear()}-${two(d.getUTCMonth() + 1)}-${two(d.getUTCDate())} ${two(d.getUTCHours())}:${two(d.getUTCMinutes())}`;
}

/** One sentence of the note: a catalog key, its parameters, and `n` for tn() (plural forms) when set */
export interface NotePart {
  key: MsgKey;
  params: Record<string, string | number>;
  n?: number;
}

/** What the note needs to know about the user besides the lock */
export interface NoteUser {
  name: string;
  /** The account can sign in now (not disabled) */
  enabled: boolean;
  /**
   * The user may have browsers that count as known: they signed in before and no new password was set for them since (an
   * administrator's reset changes the password, and the known-browser cookie belongs to the password)
   */
  mayHaveKnownBrowsers: boolean;
}

/**
 * The sentences that explain a lock or the tries left, shown as visible text under the row (a tooltip alone cannot be reached
 * on a phone or with the keyboard). Empty for a fresh state. The note never claims that some browsers "still work" when that
 * is not sure: not for an account that is disabled, never signed in or must choose a new password, nor when the other browsers
 * are locked too (`otherUntil`).
 */
export function lockNote(lock: UserLock | null, view: LockView, who: NoteUser): NotePart[] {
  if (!lock || view.state === 'fresh') return [];
  const name = who.name;
  const sure = who.enabled && who.mayHaveKnownBrowsers;
  if (view.state === 'locked') {
    const at = untilMinute(lock.until ?? '');
    if (lock.otherUntil) {
      const other = untilMinute(lock.otherUntil);
      const both: NotePart = other === at
        ? { key: 'users.lock.bothSame', params: { name, at } }
        : { key: 'users.lock.both', params: { name, known: view.knownBrowser ? at : other, other: view.knownBrowser ? other : at } };
      return [both, { key: 'users.lock.unlock', params: {} }];
    }
    const parts: NotePart[] = [
      { key: view.knownBrowser ? 'users.lock.known' : 'users.lock.other', params: { name, at } },
      { key: 'users.lock.streak', params: {}, n: lock.round },
    ];
    if (view.knownBrowser) parts.push({ key: 'users.lock.othersSeparate', params: {} });
    else if (sure) parts.push({ key: 'users.lock.knownStillWork', params: { name } });
    parts.push({ key: 'users.lock.unlock', params: {} });
    return parts;
  }
  const parts: NotePart[] = [
    { key: view.knownBrowser ? 'users.tries.known' : 'users.tries.other', params: { name }, n: view.triesLeft },
  ];
  if (!view.knownBrowser && sure) parts.push({ key: 'users.tries.knownUnaffected', params: { name } });
  // A disabled account cannot sign in, so only Unlock gives the tries back
  if (!who.enabled) parts.push({ key: 'users.tries.resetUnlock', params: {} });
  else parts.push({ key: view.knownBrowser ? 'users.tries.resetKnown' : 'users.tries.resetOther', params: {} });
  return parts;
}
