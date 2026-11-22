// v2.6 display-name helpers. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { cleanDisplayName, displayNameProblem, looksLikeEmail, shownName, signInNameOk } from './user-name.ts';

describe('display name', () => {
  it('shows the display name, else the login', () => {
    assert.equal(shownName({ username: 'a@b.co', displayName: '小王' }), '小王');
    assert.equal(shownName({ username: 'a@b.co', displayName: null }), 'a@b.co');
    assert.equal(shownName({ username: 'a@b.co', displayName: '  ' }), 'a@b.co');
  });
  it('cleans and checks like the server', () => {
    assert.equal(cleanDisplayName('  Wang   Xiao  '), 'Wang Xiao');
    assert.equal(displayNameProblem('   '), 'empty');
    assert.equal(displayNameProblem('a@b'), 'bad');
    for (const bad of ['a\u200bb', 'a\u202eb', '\ufeffa', 'a\u00adb', 'ab', 'ab']) assert.equal(displayNameProblem(bad), 'bad');
    assert.equal(displayNameProblem('一二三四五六七八九十一二三四五六七八九十'), null);
    assert.equal(displayNameProblem('一二三四五六七八九十一二三四五六七八九十一'), 'bad');
    assert.equal(displayNameProblem('😀'.repeat(20)), null);
  });
  it('recognises e-mail shaped logins', () => {
    assert.ok(looksLikeEmail('sam@example.com'));
    assert.ok(!looksLikeEmail('admin'));
    assert.ok(!looksLikeEmail('a b@c.d'));
  });
  it('signs in with an e-mail address only (or the default admin of a fresh install)', () => {
    assert.ok(signInNameOk('sam@example.com'));
    assert.ok(signInNameOk(' Admin '));
    assert.ok(!signInNameOk('小王'));
    assert.ok(!signInNameOk('sam'));
  });
});
