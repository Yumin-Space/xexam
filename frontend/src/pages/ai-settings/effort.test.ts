import assert from 'node:assert/strict';
import { test } from 'node:test';
import { effortPost, effortView, rowPaused } from './effort.ts';
import type { AiSettings } from '../../types/api.ts';

const lists = { anthropic: ['low', 'medium', 'high'], openai: ['low', 'medium', 'high'], google: ['LOW', 'HIGH'], responses: ['high', 'max'] };
const mk = (stored: { family: string; level: string } | null, using = true): Pick<AiSettings, 'slots' | 'effortLevels' | 'pool' | 'connections'> => ({
  effortLevels: lists,
  pool: [{ id: 7, connectionId: 3 }] as unknown as AiSettings['pool'],
  connections: [{ id: 3, effortFamily: 'google' }] as unknown as AiSettings['connections'],
  slots: { chat: { modelId: null, using: using ? { modelId: 7, fallback: 'none' } : null, effort: stored, effortEffective: null } } as unknown as AiSettings['slots'],
});

test('a stored level of the connection family is selected', () => {
  assert.deepEqual(effortView(mk({ family: 'google', level: 'HIGH' }), 'chat'), { family: 'google', level: 'HIGH', levels: ['LOW', 'HIGH'] });
});

test('an untouched row shows Default (empty value), not a model default', () => {
  assert.equal(effortView(mk(null), 'chat')?.level, '');
});

test('a stored level of another family counts as Default', () => {
  assert.equal(effortView(mk({ family: 'anthropic', level: 'high' }), 'chat')?.level, '');
  assert.equal(effortView(mk({ family: 'google', level: 'MEDIUM' }), 'chat')?.level, '');
});

test('Default posts null, a level posts itself', () => {
  assert.equal(effortPost(''), null);
  assert.equal(effortPost('LOW'), 'LOW');
});

test('a row that does not resolve has no view', () => {
  assert.equal(effortView(mk(null, false), 'chat'), null);
  assert.equal(effortView({ ...mk(null), effortLevels: undefined }, 'chat'), null);
});

test('master switch pauses every row', () => {
  assert.equal(rowPaused({ enabled: false }), true);
  assert.equal(rowPaused({ enabled: true }), false);
});

test('the server family wins and works when the row has no using (connection switched off)', () => {
  const s = mk(null, false);
  (s.slots.chat as { effortFamily?: string }).effortFamily = 'google';
  assert.deepEqual(effortView(s, 'chat'), { family: 'google', level: '', levels: ['LOW', 'HIGH'] });
});
