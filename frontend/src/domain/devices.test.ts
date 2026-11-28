import assert from 'node:assert/strict';
import test from 'node:test';
import type { Device } from '../types/api.ts';
import { deviceAction, deviceName, deviceStateKey, signOutTargets, sortDevices } from './devices.ts';

const dev = (o: Partial<Device>): Device => ({
  id: 1, current: false, state: 'active', ua: null, app: false, createdAt: '2026-01-01 00:00:00', signinAt: null, signinIp: null,
  signinLocation: null, activeAt: '2026-01-01 00:00:00', activeIp: null, activeLocation: null, push: false, ...o,
});

test('deviceName: browser and system, the unknown text, the app suffix', () => {
  const ua = 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Mobile/15E148 Safari/604.1';
  assert.equal(deviceName({ ua, app: false }, 'App', '?'), 'Safari · iOS');
  assert.equal(deviceName({ ua, app: true }, 'Home Screen app', '?'), 'Safari · iOS · Home Screen app');
  assert.equal(deviceName({ ua: null, app: false }, 'App', 'Unknown device'), 'Unknown device');
  assert.equal(deviceName({ ua: '  ', app: true }, 'App', 'Unknown device'), 'Unknown device · App');
});

test('deviceStateKey covers the three states', () => {
  assert.equal(deviceStateKey(dev({ state: 'active' })), 'devices.state.active');
  assert.equal(deviceStateKey(dev({ state: 'expired' })), 'devices.state.expired');
  assert.equal(deviceStateKey(dev({ state: 'signedOut' })), 'devices.state.signedOut');
});

test('sortDevices: current first, then last active (newest first), then id (newest first); input untouched', () => {
  const list = [
    dev({ id: 1, activeAt: '2026-01-01 10:00:00' }),
    dev({ id: 2, activeAt: '2026-01-03 10:00:00' }),
    dev({ id: 3, activeAt: '2026-01-03 10:00:00' }),
    dev({ id: 4, activeAt: '2025-12-01 10:00:00', current: true }),
  ];
  const before = list.map((d) => d.id);
  assert.deepEqual(sortDevices(list).map((d) => d.id), [4, 3, 2, 1]);
  assert.deepEqual(list.map((d) => d.id), before);
  assert.deepEqual(sortDevices([]), []);
});

test('deviceAction: nothing for the current or a signed-out device, Stop for expired, sign-out for active', () => {
  assert.equal(deviceAction(dev({ current: true })), 'none');
  assert.equal(deviceAction(dev({ state: 'signedOut' })), 'none');
  assert.equal(deviceAction(dev({ state: 'expired' })), 'stop');
  assert.equal(deviceAction(dev({ state: 'active' })), 'signOut');
  assert.equal(deviceAction(dev({ state: 'expired', current: true })), 'none');
});

test('signOutTargets keeps only the rows a bulk sign-out reaches', () => {
  const list = [dev({ id: 1, current: true }), dev({ id: 2 }), dev({ id: 3, state: 'expired' }), dev({ id: 4, state: 'signedOut' })];
  assert.deepEqual(signOutTargets(list).map((d) => d.id), [2, 3]);
});
