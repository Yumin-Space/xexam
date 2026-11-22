import assert from 'node:assert/strict';
import test from 'node:test';
import { visitToReport } from './visits.ts';

test('nothing is reported unless the server asked for visits', () => {
  assert.equal(visitToReport(false, 'home', 'banks'), null);
  assert.equal(visitToReport(false, null, 'home'), null);
});

test('a route-name change is reported, the first page too', () => {
  assert.equal(visitToReport(true, null, 'home'), 'home');
  assert.equal(visitToReport(true, 'home', 'records'), 'records');
  assert.equal(visitToReport(true, 'records', 'record'), 'record');
});

test('the same name (another record, a redraw) and an unknown name are not reported', () => {
  assert.equal(visitToReport(true, 'record', 'record'), null);
  assert.equal(visitToReport(true, 'home', 'nowhere'), null);
  assert.equal(visitToReport(true, 'home', 'study'), 'study');
});

test('the profile page is reported like the other pages (v2.6)', () => {
  assert.equal(visitToReport(true, 'settings', 'profile'), 'profile');
});
