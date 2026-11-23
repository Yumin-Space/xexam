// Hash routing: the dashboard stays at #/ and the exam setup lives at #/study (v2.5).
import assert from 'node:assert/strict';
import test from 'node:test';
import { parseHash, routeHash, sameRoute, sectionOf } from './router.ts';

test('study parses, round-trips and belongs to its own menu section', () => {
  assert.deepEqual(parseHash('#/study'), { name: 'study' });
  assert.equal(routeHash({ name: 'study' }), '#/study');
  assert.equal(sectionOf({ name: 'study' }), 'study');
  assert.equal(sectionOf({ name: 'quiz' }), null);
  assert.ok(sameRoute(parseHash('#/study/'), { name: 'study' }));
});

test('home is the dashboard and unknown paths fall back to it', () => {
  assert.deepEqual(parseHash('#/'), { name: 'home' });
  assert.deepEqual(parseHash(''), { name: 'home' });
  assert.deepEqual(parseHash('#/nowhere'), { name: 'home' });
  assert.equal(routeHash({ name: 'home' }), '#/');
  assert.equal(sectionOf({ name: 'home' }), 'home');
});

test('profile parses, round-trips and has its own menu section (v2.6)', () => {
  assert.deepEqual(parseHash('#/profile'), { name: 'profile' });
  assert.equal(routeHash({ name: 'profile' }), '#/profile');
  assert.equal(sectionOf({ name: 'profile' }), 'profile');
  assert.ok(sameRoute(parseHash('#/profile/'), { name: 'profile' }));
});

test('an administrator reading the records of a user: routes parse, round-trip and stay in the users section (v2.7)', () => {
  assert.deepEqual(parseHash('#/users/7/records'), { name: 'userRecords', user: 7 });
  assert.deepEqual(parseHash('#/users/7/records/42/'), { name: 'userRecord', user: 7, id: 42 });
  assert.equal(routeHash({ name: 'userRecords', user: 7 }), '#/users/7/records');
  assert.equal(routeHash({ name: 'userRecord', user: 7, id: 42 }), '#/users/7/records/42');
  assert.equal(sectionOf({ name: 'userRecords', user: 7 }), 'users');
  assert.equal(sectionOf({ name: 'userRecord', user: 7, id: 42 }), 'users');
  assert.ok(!sameRoute({ name: 'userRecords', user: 7 }, { name: 'userRecords', user: 8 }));
  // Anything malformed is the plain users page
  for (const bad of ['#/users/x/records', '#/users/7', '#/users/7/other', '#/users/7/records/abc', '#/users/7/records/1/2', '#/users/1234567890123456/records']) {
    assert.deepEqual(parseHash(bad), { name: 'users' }, bad);
  }
});
