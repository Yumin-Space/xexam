import assert from 'node:assert/strict';
import { test } from 'node:test';
import { effortView, rowPaused } from './effort.ts';
import type { AiSettings } from '../../types/api.ts';

const lists = { anthropic: ['low', 'medium', 'high'], openai: ['low', 'medium', 'high'], google: ['LOW', 'HIGH'], responses: ['high', 'max'] };
const mk = (eff: { family: 'google'; level: string } | null): Pick<AiSettings, 'slots' | 'effortLevels'> => ({
  effortLevels: lists,
  slots: { chat: { modelId: null, using: null, effortEffective: eff } } as unknown as AiSettings['slots'],
});

test('the row shows the effective level of its family', () => {
  assert.deepEqual(effortView(mk({ family: 'google', level: 'HIGH' }), 'chat'), { family: 'google', level: 'HIGH', levels: ['LOW', 'HIGH'] });
});

test('a row that does not resolve has no view', () => {
  assert.equal(effortView(mk(null), 'chat'), null);
  assert.equal(effortView({ ...mk(null), effortLevels: undefined }, 'chat'), null);
});

test('master switch pauses every row', () => {
  assert.equal(rowPaused({ enabled: false }), true);
  assert.equal(rowPaused({ enabled: true }), false);
});
