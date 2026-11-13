// Feature permissions and the visibility rule of every AI control (v1.1, FP-3, M-7, A-OWNER-15).
import assert from 'node:assert/strict';
import test from 'node:test';
import type { AiStatus } from '../state/ai.ts';
import { ALL_ON, FEATURE_GROUPS, FEATURE_KEYS, aiControlView, offList, readFeatures, sameFeatures, type ControlContext } from './ai-features.ts';

const ready = (o: Record<string, unknown> = {}): AiStatus => ({
  state: 'ready', enabled: true, allowed: true, suspended: false, hasData: false, running: null, retentionDays: 90, memoryLimit: 50,
  memoryMaxTokens: 200, maxMessageChars: 16000, keyProblem: false, configured: true, freeMode: false, ...o,
}) as AiStatus;

const ctx = (o: Partial<ControlContext> = {}): ControlContext => ({ role: 'user', service: true, canManageBanks: true, status: ready(), ...o });

test('six keys in FP-1 order, grouped as the table shows them', () => {
  assert.deepEqual([...FEATURE_KEYS], ['chat', 'hint', 'explain', 'summary', 'generate', 'import']);
  assert.deepEqual([...FEATURE_GROUPS.main, ...FEATURE_GROUPS.banks], [...FEATURE_KEYS]);
});

test('readFeatures: missing or malformed is all on, a missing import key is on, only false switches off', () => {
  assert.deepEqual(readFeatures(undefined), ALL_ON);
  assert.deepEqual(readFeatures(null), ALL_ON);
  assert.deepEqual(readFeatures([]), ALL_ON);
  assert.deepEqual(readFeatures('x'), ALL_ON);
  assert.deepEqual(readFeatures({ chat: true, hint: false, explain: true, summary: true, generate: true }), { ...ALL_ON, hint: false });
  assert.deepEqual(readFeatures({ chat: 0, hint: 'no' }), ALL_ON);
  assert.deepEqual(offList(readFeatures({ generate: false, chat: false, import: false })), ['chat', 'generate', 'import']);
  assert.ok(sameFeatures(ALL_ON, readFeatures({})));
});

test('hidden: no AI+ feature, and bank controls without the bank-management permission', () => {
  assert.equal(aiControlView('chat', false, ctx({ service: false })).view, 'hidden');
  assert.equal(aiControlView('generate', true, ctx({ canManageBanks: false })).view, 'hidden');
  assert.equal(aiControlView('import', true, ctx({ canManageBanks: false, status: ready({ suspended: true }) })).view, 'hidden');
  // a non-bank control does not need the permission
  assert.equal(aiControlView('chat', false, ctx({ canManageBanks: false })).view, 'ok');
});

test('paused wins over a feature switch; the controls stay visible', () => {
  const st = ready({ suspended: true, features: { chat: false } });
  assert.equal(aiControlView('chat', false, ctx({ status: st })).view, 'paused');
  assert.equal(aiControlView('generate', true, ctx({ status: st })).view, 'paused');
});

test('unavailable reasons follow the status', () => {
  assert.deepEqual(aiControlView('chat', false, ctx({ status: { state: 'loading' } })), { view: 'unavailable', reason: 'loading' });
  assert.deepEqual(aiControlView('chat', false, ctx({ status: { state: 'unavailable' } })), { view: 'unavailable', reason: 'down' });
  assert.deepEqual(aiControlView('chat', false, ctx({ status: ready({ enabled: false, why: 'off' }) })), { view: 'unavailable', reason: 'off' });
  assert.deepEqual(aiControlView('chat', false, ctx({ status: ready({ enabled: false, why: 'link' }) })), { view: 'unavailable', reason: 'down' });
  assert.deepEqual(aiControlView('chat', false, ctx({ status: ready({ enabled: false }) })), { view: 'unavailable', reason: 'unavailable' });
});

test('feature off: greyed with the permission reason; usable only when granted, not paused and ticked', () => {
  const st = ready({ features: { chat: true, hint: false, explain: true, summary: true, generate: true } });
  assert.equal(aiControlView('hint', false, ctx({ status: st })).view, 'featureOff');
  assert.equal(aiControlView('explain', false, ctx({ status: st })).view, 'ok');
  // the sixth key is missing (older ai-service): import counts as ticked
  assert.equal(aiControlView('import', true, ctx({ status: st })).view, 'ok');
  assert.equal(aiControlView('import', true, ctx({ status: ready({ features: { import: false } }) })).view, 'featureOff');
});

test('administrators have every feature, and see nothing while AI+ is not available', () => {
  const admin = (status: AiStatus): ControlContext => ctx({ role: 'admin', canManageBanks: false, status });
  assert.equal(aiControlView('generate', true, admin(ready({ features: { generate: false } }))).view, 'ok');
  assert.equal(aiControlView('generate', true, admin(ready({ enabled: false }))).view, 'hidden');
  assert.equal(aiControlView('chat', false, admin({ state: 'loading' })).view, 'hidden');
});
