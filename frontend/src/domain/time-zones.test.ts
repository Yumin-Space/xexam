import assert from 'node:assert/strict';
import test from 'node:test';
import { allZones, deviceZone, offsetLabel, zoneOffsetMinutes, zoneOptions } from './time-zones.ts';

test('offsetLabel formats whole and half hours, both signs', () => {
  assert.equal(offsetLabel(0), 'UTC+00:00');
  assert.equal(offsetLabel(330), 'UTC+05:30');
  assert.equal(offsetLabel(-210), 'UTC-03:30');
  assert.equal(offsetLabel(-720), 'UTC-12:00');
});

test('zoneOptions: sorted by offset then name, UTC always present, unknown zones dropped, no duplicates', () => {
  const offs: Record<string, number | null> = { 'Etc/GMT-9': 540, 'Europe/Berlin': 60, 'Europe/Paris': 60, 'America/New_York': -300, 'Mars/Base': null };
  const out = zoneOptions(['Etc/GMT-9', 'Europe/Paris', 'Mars/Base', 'Europe/Berlin', 'America/New_York', 'Europe/Berlin'], (z) => offs[z] ?? null);
  assert.deepEqual(out.map((o) => o.value), ['America/New_York', 'UTC', 'Europe/Berlin', 'Europe/Paris', 'Etc/GMT-9']);
  assert.equal(out[0]?.label, '(UTC-05:00) America/New York');
  assert.equal(out[1]?.label, '(UTC+00:00) UTC');
  assert.equal(out[4]?.label, '(UTC+09:00) Etc/GMT-9');
});

test('zoneOptions: UTC is there even for an empty list', () => {
  assert.deepEqual(zoneOptions([], () => null), [{ value: 'UTC', label: '(UTC+00:00) UTC' }]);
});

test('zoneOffsetMinutes: UTC is 0, a fixed zone is exact, nonsense is null', () => {
  const at = new Date(Date.UTC(2026, 0, 15, 12, 0, 0));
  assert.equal(zoneOffsetMinutes('UTC', at), 0);
  assert.equal(zoneOffsetMinutes('Etc/GMT-9', at), 540);
  assert.equal(zoneOffsetMinutes('Asia/Kolkata', at), 330);
  assert.equal(zoneOffsetMinutes('Not/AZone', at), null);
});

test('zoneOffsetMinutes follows daylight saving time', () => {
  assert.equal(zoneOffsetMinutes('America/New_York', new Date(Date.UTC(2026, 0, 15, 12))), -300);
  assert.equal(zoneOffsetMinutes('America/New_York', new Date(Date.UTC(2026, 6, 15, 12))), -240);
});

test('allZones is never empty and keeps UTC in the picker; deviceZone is a string or null', () => {
  const zones = allZones();
  assert.ok(zones.length >= 1);
  const z = deviceZone();
  assert.ok(z === null || typeof z === 'string');
  assert.ok(zoneOptions(zones, (n) => zoneOffsetMinutes(n)).some((o) => o.value === 'UTC'));
});
