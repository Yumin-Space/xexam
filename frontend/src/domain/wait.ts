// How long a sign-in lock still runs, rounded up to whole minutes, hours or days (the backend's "retryAfter" seconds; see the
// escalating login lock in docs/en/security.md). Pure: the texts come from the catalogs (auth.wait.* and users.lock.*).

export type WaitUnit = 'minute' | 'hour' | 'day';

export interface Wait {
  n: number;
  unit: WaitUnit;
}

/**
 * `seconds` rounded up: minutes below one hour, hours below one day, else days. A wait never shows as less than it is
 * ("try again in 1 minute" after 61 seconds would be wrong): 61 s → 2 minutes, 59 min 1 s → 1 hour, 23 h 1 s → 1 day.
 */
export function roundWait(seconds: number): Wait {
  const s = Math.max(1, Math.ceil(Number.isFinite(seconds) ? seconds : 1));
  const minutes = Math.ceil(s / 60);
  if (minutes < 60) return { n: minutes, unit: 'minute' };
  const hours = Math.ceil(s / 3600);
  if (hours < 24) return { n: hours, unit: 'hour' };
  return { n: Math.ceil(s / 86400), unit: 'day' };
}

/** Seconds left until `deadline` (milliseconds on the same clock as `now`), never negative. */
export function secondsLeft(deadline: number, now: number): number {
  return Math.max(0, (deadline - now) / 1000);
}
