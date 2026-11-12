// Browser storage key names: the per-user keys carry the user id, so another user on the same browser never reads them.
import assert from 'node:assert/strict';
import test from 'node:test';
import { sessionKey, settingsKey, sheetOpenKey } from './storage-keys.ts';

test('sheetOpenKey: one key per user, in the xexam.u<id> family', () => {
  assert.equal(sheetOpenKey(7), 'xexam.u7.sheetOpen.v1');
  assert.notEqual(sheetOpenKey(7), sheetOpenKey(8));
  assert.notEqual(sheetOpenKey(7), sessionKey(7));
  assert.notEqual(sheetOpenKey(7), settingsKey(7));
});
