// v1.6 thinking strength: the known-default lookup (exact, after normalization) and the level kept on a family change.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { knownDefault, levelForFamily } from './effort-defaults.ts';

const TABLE: Array<[string, string]> = [
  ['claude-opus-5-5', 'medium'], ['claude-haiku-5-5', 'medium'], ['claude-sonnet-5-5', 'high'], ['claude-opus-5', 'high'],
  ['claude-sonnet-5', 'high'], ['claude-opus-4-8', 'high'], ['claude-opus-4-7', 'high'], ['claude-opus-4-6', 'high'],
  ['claude-sonnet-4-6', 'high'], ['gpt-6.1-sol', 'medium'], ['gpt-6.1-terra', 'medium'], ['gpt-6-sol', 'medium'],
  ['gpt-6-luna', 'medium'], ['gpt-5.6', 'medium'], ['gpt-5.5', 'medium'], ['gpt-5.4', 'none'], ['gpt-5.2', 'none'],
  ['gpt-5.1', 'none'], ['gemini-4-hydrogen', 'HIGH'], ['gemini-3.1-pro-preview', 'HIGH'], ['gemini-3-flash-preview', 'HIGH'],
  ['gemini-3-pro-preview', 'HIGH'], ['gemini-4-carbon', 'MEDIUM'], ['gemini-4-argon', 'MEDIUM'], ['gemini-3.8-flash', 'MEDIUM'],
  ['gemini-3.6-flash', 'MEDIUM'], ['gemini-3.5-flash-lite', 'MINIMAL'], ['gemini-3.1-flash-lite', 'MINIMAL'],
  ['gemini-2.5-flash-lite', 'none'],
];

test('every table entry resolves', () => {
  for (const [m, l] of TABLE) assert.equal(knownDefault(m), l, m);
});

test('routing prefixes, versions, dates and dots normalize', () => {
  assert.equal(knownDefault('us.anthropic.claude-opus-5-5-v1:0'), 'medium');
  assert.equal(knownDefault('anthropic/claude-sonnet-4.6'), 'high');
  assert.equal(knownDefault('claude-opus-5-20260101'), 'high');
  assert.equal(knownDefault('models/gemini-3-flash-preview'), 'HIGH');
  assert.equal(knownDefault('GPT-5.5'), 'medium');
});

test('near misses return null', () => {
  for (const m of ['claude-opus-4-5', 'gpt-5', 'gpt-5.5-mini', 'gpt-6-astra', 'gemini-2.5-pro', 'gemini-2.5-flash', 'claude-opus-5-5-x', '', 'toString'])
    assert.equal(knownDefault(m), null, m);
  assert.equal(knownDefault(null), null);
  assert.equal(knownDefault(undefined), null);
});

test('a family change keeps a valid level and falls back to medium', () => {
  const anth = ['low', 'medium', 'high', 'xhigh', 'max'];
  assert.equal(levelForFamily('high', anth), 'high');
  assert.equal(levelForFamily('none', anth), 'medium');
  assert.equal(levelForFamily(null, anth), 'medium');
});

test('deepseek model ids default to high under the Responses API family; levels are none / low / high / max', () => {
  for (const m of ['deepseek-v4', 'deepseek-reasoner', 'deepseek/deepseek-chat', 'us.deepseek.r1-v1:0']) assert.equal(knownDefault(m), 'high', m);
  const ds = ['none', 'low', 'high', 'max'];
  assert.equal(levelForFamily('none', ds), 'none');
  assert.equal(levelForFamily('max', ds), 'max');
  assert.equal(levelForFamily('medium', ds), 'high');
  assert.equal(levelForFamily('xhigh', ds), 'high');
});

test('Gemma models default to HIGH; the level list is HIGH / MINIMAL', () => {
  for (const m of ['gemma-4-31b-it', '@cf/google/gemma-4-26b-a4b-it', 'google/gemma-3-27b']) assert.equal(knownDefault(m), 'HIGH', m);
  const g = ['HIGH', 'MINIMAL'];
  assert.equal(levelForFamily('MINIMAL', g), 'MINIMAL');
  assert.equal(levelForFamily('HIGH', g), 'HIGH');
  assert.equal(levelForFamily('high', g), 'HIGH');
  assert.equal(levelForFamily('minimal', g), 'MINIMAL');
  assert.equal(levelForFamily('medium', g), 'HIGH');
  assert.equal(levelForFamily('max', g), 'HIGH');
  assert.equal(levelForFamily(null, g), 'HIGH');
});

test('Google levels keep the choice case-insensitively and fall back to MEDIUM', () => {
  const go = ['MINIMAL', 'LOW', 'MEDIUM', 'HIGH', 'XHIGH'];
  assert.equal(levelForFamily('high', go), 'HIGH');
  assert.equal(levelForFamily('xhigh', go), 'XHIGH');
  assert.equal(levelForFamily('LOW', go), 'LOW');
  assert.equal(levelForFamily('max', go), 'MEDIUM');
  assert.equal(levelForFamily('none', go), 'MEDIUM');
  assert.equal(levelForFamily(null, go), 'MEDIUM');
  const oa = ['low', 'medium', 'high', 'xhigh', 'max'];
  assert.equal(levelForFamily('XHIGH', oa), 'xhigh');
  assert.equal(levelForFamily('MINIMAL', oa), 'medium');
});
