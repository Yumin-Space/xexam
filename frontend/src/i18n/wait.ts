// A sign-in lock's wait as text in the current UI language, rounded up (see ../domain/wait.ts):
// waitText for sentences ("Try again in 5 minutes"), waitShort for the compact status tag of User management ("5 min").
import { roundWait } from '../domain/wait.ts';
import { localeTag, tn } from './index.ts';

export function waitText(seconds: number): string {
  const w = roundWait(seconds);
  return tn(w.unit === 'minute' ? 'auth.wait.minutes' : w.unit === 'hour' ? 'auth.wait.hours' : 'auth.wait.days', w.n);
}

export function waitShort(seconds: number): string {
  const w = roundWait(seconds);
  return tn(w.unit === 'minute' ? 'users.lock.minutes' : w.unit === 'hour' ? 'users.lock.hours' : 'users.lock.days', w.n);
}

/**
 * A moment as the clock time of the reader (HH:MM in the Settings-language locale and the browser's time zone), with the date
 * (M/D) in front when it is not the same calendar day as `nowSec` (v1.0 AI request caps: "about 5 minutes (14:05)").
 */
export function clockText(atSec: number, nowSec: number): string {
  const at = new Date(atSec * 1000);
  const now = new Date(nowSec * 1000);
  const time = new Intl.DateTimeFormat(localeTag(), { hour: '2-digit', minute: '2-digit', hour12: false }).format(at);
  const sameDay = at.getFullYear() === now.getFullYear() && at.getMonth() === now.getMonth() && at.getDate() === now.getDate();
  return sameDay ? time : `${at.getMonth() + 1}/${at.getDate()} ${time}`;
}
