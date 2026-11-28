// Browser storage key names: the per-user keys carry the user id, so another user on the same browser never reads them.
import assert from 'node:assert/strict';
import test from 'node:test';
import { guideTestKey, paths, pushConfirmKey, sessionKey, settingsKey, sheetOpenKey } from './storage-keys.ts';

test('sheetOpenKey: one key per user, in the xexam.u<id> family', () => {
  assert.equal(sheetOpenKey(7), 'xexam.u7.sheetOpen.v1');
  assert.notEqual(sheetOpenKey(7), sheetOpenKey(8));
  assert.notEqual(sheetOpenKey(7), sessionKey(7));
  assert.notEqual(sheetOpenKey(7), settingsKey(7));
});

test('v3.1 keys: the guide is per browser, its test and the push confirmation are per user', () => {
  assert.equal(paths.guide, 'xexam.guide.v1');
  assert.equal(guideTestKey(7), 'xexam.u7.guideTest.v1');
  assert.equal(pushConfirmKey(7), 'xexam.u7.pushConfirmed.v1');
  assert.notEqual(guideTestKey(7), guideTestKey(8));
  assert.notEqual(pushConfirmKey(7), pushConfirmKey(8));
  assert.notEqual(guideTestKey(7), pushConfirmKey(7));
  assert.notEqual(guideTestKey(7), sessionKey(7));
});
