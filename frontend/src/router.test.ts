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
