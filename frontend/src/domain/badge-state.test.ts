import assert from 'node:assert/strict';
import { test } from 'node:test';
import { badgeKey, badgeState, badgeTip, ownBadgeState, type BadgeState } from './badge-state.ts';

const key = (b: BadgeState): string => badgeKey(b);

test('badgeState: every combination for a plain user', () => {
  for (const service of [false, true]) {
    for (const suspended of [false, true]) {
      for (const available of [false, true]) {
        const got = key(badgeState({ role: 'user', service, suspended, available }));
        const want = !service ? 'none' : suspended ? 'off-suspended' : !available ? 'off-unavailable' : 'on';
        assert.equal(got, want, `service ${service}, suspended ${suspended}, available ${available}`);
      }
    }
  }
});

test('badgeState: administrators are bright or nothing, never dark', () => {
  for (const service of [false, true]) {
    for (const suspended of [false, true]) {
      assert.equal(key(badgeState({ role: 'admin', service, suspended, available: true })), 'on');
      assert.equal(key(badgeState({ role: 'admin', service, suspended, available: false })), 'none');
    }
  }
});

test('badgeState: the reason of the dark badge is "suspended" before "unavailable"', () => {
  assert.deepEqual(badgeState({ role: 'user', service: true, suspended: true, available: false }), { kind: 'off', reason: 'suspended' });
  assert.deepEqual(badgeState({ role: 'user', service: true, suspended: false, available: false }), { kind: 'off', reason: 'unavailable' });
  assert.deepEqual(badgeState({ role: 'user', service: true, suspended: false, available: true }), { kind: 'on' });
  assert.deepEqual(badgeState({ role: 'user', service: false, suspended: true, available: true }), { kind: 'none' });
});

test('ownBadgeState: the signed-in user from the session and the AI status', () => {
  const user = { role: 'user' as const, aiAllowed: true };
  const admin = { role: 'admin' as const, aiAllowed: true };
  // Nobody signed in, or the first status still being read: no badge (no dark flash before the bright one)
  assert.equal(key(ownBadgeState(null, { state: 'ready', enabled: true, suspended: false })), 'none');
  assert.equal(key(ownBadgeState(user, { state: 'loading' })), 'none');
  assert.equal(key(ownBadgeState(admin, { state: 'loading' })), 'none');
  // The AI service cannot be reached: dark for a user with the service, nothing for an administrator
  assert.equal(key(ownBadgeState(user, { state: 'unavailable' })), 'off-down');
  assert.equal(key(ownBadgeState(admin, { state: 'unavailable' })), 'none');
  // Ready
  assert.equal(key(ownBadgeState(user, { state: 'ready', enabled: true, suspended: false })), 'on');
  assert.equal(key(ownBadgeState(user, { state: 'ready', enabled: false, suspended: false })), 'off-unavailable');
  assert.equal(key(ownBadgeState(user, { state: 'ready', enabled: true, suspended: true })), 'off-suspended');
  assert.equal(key(ownBadgeState(user, { state: 'ready', enabled: false, suspended: true })), 'off-suspended');
  // An older answer without "suspended" counts as not suspended
  assert.equal(key(ownBadgeState(user, { state: 'ready', enabled: true })), 'on');
  // No AI+ service: no badge whatever the status says
  assert.equal(key(ownBadgeState({ role: 'user', aiAllowed: false }, { state: 'ready', enabled: true, suspended: true })), 'none');
  assert.equal(key(ownBadgeState(admin, { state: 'ready', enabled: true, suspended: false })), 'on');
  assert.equal(key(ownBadgeState(admin, { state: 'ready', enabled: false, suspended: false })), 'none');
});

test('ownBadgeState: `why` names the cause of a dark badge (v1.1, BT-2)', () => {
  const user = { role: 'user' as const, aiAllowed: true };
  const st = (why?: 'off' | 'link' | null) => ({ state: 'ready' as const, enabled: false, suspended: false, ...(why !== undefined ? { why } : {}) });
  assert.equal(key(ownBadgeState(user, st('off'))), 'off-off');
  assert.equal(key(ownBadgeState(user, st('link'))), 'off-down');
  assert.equal(key(ownBadgeState(user, st(null))), 'off-unavailable');
  assert.equal(key(ownBadgeState(user, st())), 'off-unavailable');
  // suspended is checked first
  assert.equal(key(ownBadgeState(user, { state: 'ready', enabled: false, suspended: true, why: 'off' })), 'off-suspended');
  assert.equal(key(ownBadgeState(user, { state: 'ready', enabled: true, why: null })), 'on');
});

test('badgeState: an admin view takes the reason from the service state', () => {
  assert.deepEqual(badgeState({ role: 'user', service: true, suspended: false, available: false, unavailableReason: 'off' }), { kind: 'off', reason: 'off' });
  assert.deepEqual(badgeState({ role: 'user', service: true, suspended: false, available: false, unavailableReason: 'down' }), { kind: 'off', reason: 'down' });
  assert.deepEqual(badgeState({ role: 'user', service: true, suspended: true, available: false, unavailableReason: 'down' }), { kind: 'off', reason: 'suspended' });
});

test('badgeTip: every row of the BT-2 table, both viewers', () => {
  const tip = (b: BadgeState, v: 'own' | 'admin', off: Parameters<typeof badgeTip>[2] = []) => badgeTip(b, v, off);
  assert.equal(tip({ kind: 'none' }, 'own'), null);
  assert.equal(tip({ kind: 'none' }, 'admin'), null);
  assert.deepEqual(tip({ kind: 'on' }, 'own'), { key: 'ai.badge.tip.on', off: [] });
  assert.deepEqual(tip({ kind: 'on' }, 'admin'), { key: 'ai.badge.tip.adminOn', off: [] });
  assert.deepEqual(tip({ kind: 'on' }, 'own', ['hint']), { key: 'ai.badge.tip.onSome', off: ['hint'] });
  assert.deepEqual(tip({ kind: 'on' }, 'admin', ['chat', 'generate', 'import']), { key: 'ai.badge.tip.adminOnSome', off: ['chat', 'generate', 'import'] });
  assert.equal(tip({ kind: 'off', reason: 'suspended' }, 'own')?.key, 'ai.badge.tip.suspended');
  assert.equal(tip({ kind: 'off', reason: 'suspended' }, 'admin')?.key, 'ai.badge.tip.adminSuspended');
  assert.equal(tip({ kind: 'off', reason: 'off' }, 'own')?.key, 'ai.badge.tip.off');
  assert.equal(tip({ kind: 'off', reason: 'off' }, 'admin')?.key, 'ai.badge.tip.adminOff');
  assert.equal(tip({ kind: 'off', reason: 'down' }, 'own')?.key, 'ai.badge.tip.down');
  assert.equal(tip({ kind: 'off', reason: 'down' }, 'admin')?.key, 'ai.badge.tip.adminDown');
  assert.equal(tip({ kind: 'off', reason: 'unavailable' }, 'own')?.key, 'ai.badge.unavailable');
  assert.equal(tip({ kind: 'off', reason: 'unavailable' }, 'admin')?.key, 'ai.badge.unavailableAdmin');
  // a dark badge never lists features
  assert.deepEqual(tip({ kind: 'off', reason: 'off' }, 'own', ['hint'])?.off, []);
});
