import assert from 'node:assert/strict';
import { test } from 'node:test';
import { USER_TABS, hashOfUserTab, userTabOfHash } from './users-tabs.ts';

test('the user management tabs live in the hash; a missing or unknown value is the first tab', () => {
  assert.deepEqual([...USER_TABS], ['users', 'signin']);
  assert.equal(userTabOfHash('#/users'), 'users');
  assert.equal(userTabOfHash('#/users?tab=signin'), 'signin');
  assert.equal(userTabOfHash('#/users?tab=users'), 'users');
  assert.equal(userTabOfHash('#/users?tab=other'), 'users');
  assert.equal(userTabOfHash('#/users?tab='), 'users');
  assert.equal(userTabOfHash('#/users?x=1&tab=signin'), 'signin');
  assert.equal(userTabOfHash(''), 'users');
  for (const k of USER_TABS) assert.equal(userTabOfHash(hashOfUserTab(k)), k);
  assert.equal(hashOfUserTab('signin'), '#/users?tab=signin');
});
