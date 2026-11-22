import assert from 'node:assert/strict';
import { test } from 'node:test';
import {
  AI_SERVICE_OFF_CODE, MSG_AI_SERVICE_OFF, SEARCH_FROM, aiLineKind, aiSwitchShown, createAi, factPlan, filterUsers, isServiceOff, lockUntilLabel, fullAt,
} from './user-admin.ts';

test('aiSwitchShown: only for a plain user while AI+ is available', () => {
  assert.equal(aiSwitchShown(true, 'user'), true);
  assert.equal(aiSwitchShown(true, 'admin'), false);
  assert.equal(aiSwitchShown(false, 'user'), false);
  assert.equal(aiSwitchShown(false, 'admin'), false);
});

test('aiLineKind: no AI+ line at all while AI+ is not available', () => {
  assert.equal(aiLineKind(false, 'user'), 'none');
  assert.equal(aiLineKind(false, 'admin'), 'none');
  assert.equal(aiLineKind(true, 'user'), 'switch');
  assert.equal(aiLineKind(true, 'admin'), 'admin');
});

test('createAi: "ai": true only when the box is drawn and ticked', () => {
  assert.equal(createAi(true, 'user', true), true);
  assert.equal(createAi(true, 'user', false), false);
  // An administrator always has AI while AI+ is available: nothing is stored for them
  assert.equal(createAi(true, 'admin', true), false);
  // AI+ not available: the box is not drawn, so a box ticked earlier is not sent
  assert.equal(createAi(false, 'user', true), false);
  assert.equal(createAi(false, 'admin', true), false);
});

test('isServiceOff: the backend refusal by code or by its exact text', () => {
  assert.equal(AI_SERVICE_OFF_CODE, 'ai.serviceOff');
  // The backend's MSG_AI_SERVICE_OFF (contracts §2.5), byte for byte
  assert.equal(MSG_AI_SERVICE_OFF, '请先在 AI 设置里开启 AI+ 功能。');
  assert.equal(isServiceOff({ code: 'ai.serviceOff', serverMessage: MSG_AI_SERVICE_OFF }), true);
  assert.equal(isServiceOff({ code: 'ai.serviceOff' }), true);
  assert.equal(isServiceOff({ serverMessage: MSG_AI_SERVICE_OFF }), true);
  assert.equal(isServiceOff({ code: 'ai.disabled' }), false);
  assert.equal(isServiceOff({ serverMessage: '用户名已被使用。' }), false);
  assert.equal(isServiceOff(null), false);
  assert.equal(isServiceOff('ai.serviceOff'), false);
});

test('lockUntilLabel: the time of day, with the date only when the lock ends on another day than it started', () => {
  // 600 s left at 01:44 -> ends 01:54 the same day
  assert.equal(lockUntilLabel('2026-11-12 01:54:00', 600), '01:54');
  // rounded up to the minute
  assert.equal(lockUntilLabel('2026-11-12 01:54:10', 600), '01:55');
  // 4 hours left at 22:00 -> ends 02:00 the next day
  assert.equal(lockUntilLabel('2026-11-13 02:00:00', 4 * 3600), '11-13 02:00');
  // a month end carries correctly
  assert.equal(lockUntilLabel('2026-12-01 00:30:00', 3600), '12-01 00:30');
  assert.equal(lockUntilLabel('garbage', 5), 'garbage');
});

test('fullAt: always the full year, no seconds', () => {
  assert.equal(fullAt('2026-11-12 01:46:09'), '2026-11-12 01:46');
  assert.equal(fullAt('2026-11-12 01:46'), '2026-11-12 01:46');
});

test('filterUsers: case-insensitive part of the name; empty query keeps everyone', () => {
  const list = [{ username: 'Alex' }, { username: 'riley' }, { username: 'alexa.b' }];
  assert.deepEqual(filterUsers(list, ' ALE ').map((u) => u.username), ['Alex', 'alexa.b']);
  assert.equal(filterUsers(list, '').length, 3);
  assert.deepEqual(filterUsers([{ username: 'a@x.test', displayName: 'Sunny' }, { username: 'b@x.test' }], 'sun').map((u) => u.username), ['a@x.test']);
  assert.equal(filterUsers(list, 'zzz').length, 0);
  assert.equal(SEARCH_FROM, 10);
});

test('factPlan: the order of the fact lines, eye open', () => {
  const u = { lastActiveAt: '2026-11-15 11:02', lastLoginIp: '203.0.113.5', lastActiveIp: '198.51.100.7', lastActiveUa: 'Example/1.0' };
  assert.deepEqual(factPlan(u, true), ['lastLogin', 'loginIp', 'loginPlace', 'lastActive', 'activeIp', 'activePlace', 'activeDevice', 'records']);
});

test('factPlan: eye closed keeps the device line, drops every IP / place line', () => {
  const u = { lastActiveAt: '2026-11-15 11:02', lastLoginIp: '203.0.113.5', lastActiveIp: '198.51.100.7', lastActiveUa: 'Example/1.0' };
  assert.deepEqual(factPlan(u, false), ['lastLogin', 'lastActive', 'activeDevice', 'records']);
  // the answer carries no IP while the eye is closed: the same plan
  assert.deepEqual(factPlan({ lastActiveAt: u.lastActiveAt, lastLoginIp: null, lastActiveIp: null, lastActiveUa: 'Example/1.0' }, true), ['lastLogin', 'lastActive', 'activeDevice', 'records']);
});

test('factPlan: a never-active user has no activity lines; no device = no device line', () => {
  assert.deepEqual(factPlan({ lastActiveAt: null, lastActiveIp: '198.51.100.7', lastActiveUa: 'Example/1.0' }, true), ['lastLogin', 'lastActive', 'records']);
  assert.deepEqual(factPlan({ lastActiveAt: '2026-11-15 11:02', lastActiveIp: '198.51.100.7', lastActiveUa: ' ' }, true), ['lastLogin', 'lastActive', 'activeIp', 'activePlace', 'records']);
  assert.deepEqual(factPlan({ lastActiveAt: '2026-11-15 11:02', lastLoginIp: '203.0.113.5' }, true), ['lastLogin', 'loginIp', 'loginPlace', 'lastActive', 'records']);
});
