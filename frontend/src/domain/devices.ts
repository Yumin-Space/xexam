// Signed-in devices (v3.1): the pure rules behind the device list of Settings and User management. No DOM, no network.
import type { Device } from '../types/api.ts';
import { deviceLabel } from './activity.ts';

/** `Chrome · Windows` style name of a device (from its User-Agent), plus " · Home Screen app" for an installed app */
export function deviceName(d: Pick<Device, 'ua' | 'app'>, appLabel: string, unknown: string): string {
  const base = deviceLabel(d.ua) ?? unknown;
  return d.app ? `${base} · ${appLabel}` : base;
}

export type DeviceStateKey = 'devices.state.active' | 'devices.state.expired' | 'devices.state.signedOut';

/** The catalog key of the state tag */
export function deviceStateKey(d: Pick<Device, 'state'>): DeviceStateKey {
  return d.state === 'active' ? 'devices.state.active' : d.state === 'expired' ? 'devices.state.expired' : 'devices.state.signedOut';
}

/** Current device first, then the most recently active, then the newest row; the input is not changed */
export function sortDevices(list: readonly Device[]): Device[] {
  return [...list].sort((a, b) => {
    if (a.current !== b.current) return a.current ? -1 : 1;
    if (a.activeAt !== b.activeAt) return a.activeAt < b.activeAt ? 1 : -1;
    return b.id - a.id;
  });
}

/** What the sign-out action of a row is: nothing (the current device, or already signed out), a sign-out, or "Stop" for an expired row (D20) */
export type DeviceAction = 'none' | 'signOut' | 'stop';

export function deviceAction(d: Pick<Device, 'current' | 'state'>): DeviceAction {
  if (d.current || d.state === 'signedOut') return 'none';
  return d.state === 'expired' ? 'stop' : 'signOut';
}

/** The devices a "sign out all" would reach: not the current one, still active or expired */
export function signOutTargets(list: readonly Device[]): Device[] {
  return list.filter((d) => deviceAction(d) !== 'none');
}
