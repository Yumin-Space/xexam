// The bank list's view of a bank summary: a bank with retired questions only (v0.7: questions from an older Excel layout).
import assert from 'node:assert/strict';
import test from 'node:test';
import { retiredOnly } from './bank-status.ts';

test('retiredOnly: no active question and at least one retired one', () => {
  assert.equal(retiredOnly({ total: 0, inactive: 12 }), true);
  // an empty bank (nothing imported, nothing retired) is not one
  assert.equal(retiredOnly({ total: 0, inactive: 0 }), false);
  // a bank in use, with or without retired questions, is not one
  assert.equal(retiredOnly({ total: 5, inactive: 0 }), false);
  assert.equal(retiredOnly({ total: 5, inactive: 3 }), false);
});
