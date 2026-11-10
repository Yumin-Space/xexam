// A sign-in lock's wait as text in the current UI language, rounded up (see ../domain/wait.ts):
// waitText for sentences ("Try again in 5 minutes"), waitShort for the compact status tag of User management ("5 min").
import { roundWait } from '../domain/wait.ts';
import { tn } from './index.ts';

export function waitText(seconds: number): string {
  const w = roundWait(seconds);
  return tn(w.unit === 'minute' ? 'auth.wait.minutes' : w.unit === 'hour' ? 'auth.wait.hours' : 'auth.wait.days', w.n);
}

export function waitShort(seconds: number): string {
  const w = roundWait(seconds);
  return tn(w.unit === 'minute' ? 'users.lock.minutes' : w.unit === 'hour' ? 'users.lock.hours' : 'users.lock.days', w.n);
}
