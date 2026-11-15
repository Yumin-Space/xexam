// v1.6 thinking strength: the known-default lookup (exact, after normalization) and the level kept on a family change.
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { knownDefault, levelForFamily } from './effort-defaults.ts';

const TABLE: Array<[string, string]> = [
  ['claude-opus-5-5', 'medium'], ['claude-haiku-5-5', 'medium'], ['claude-sonnet-5-5', 'high'], ['claude-opus-5', 'high'],
  ['claude-sonnet-5', 'high'], ['claude-opus-4-8', 'high'], ['claude-opus-4-7', 'high'], ['claude-opus-4-6', 'high'],
  ['claude-sonnet-4-6', 'high'], ['gpt-6.1-sol', 'medium'], ['gpt-6.1-terra', 'medium'], ['gpt-6-sol', 'medium'],
  ['gpt-6-luna', 'medium'], ['gpt-5.6', 'medium'], ['gpt-5.5', 'medium'], ['gpt-5.4', 'none'], ['gpt-5.2', 'none'],
  ['gpt-5.1', 'none'], ['gemini-4-hydrogen', 'high'], ['gemini-3.1-pro-preview', 'high'], ['gemini-3-flash-preview', 'high'],
  ['gemini-3-pro-preview', 'high'], ['gemini-4-carbon', 'medium'], ['gemini-4-argon', 'medium'], ['gemini-3.8-flash', 'medium'],
  ['gemini-3.6-flash', 'medium'], ['gemini-3.5-flash-lite', 'minimal'], ['gemini-3.1-flash-lite', 'minimal'],
  ['gemini-2.5-flash-lite', 'none'],
];

test('every table entry resolves', () => {
  for (const [m, l] of TABLE) assert.equal(knownDefault(m), l, m);
});

test('routing prefixes, versions, dates and dots normalize', () => {
  assert.equal(knownDefault('us.anthropic.claude-opus-5-5-v1:0'), 'medium');
  assert.equal(knownDefault('anthropic/claude-sonnet-4.6'), 'high');
  assert.equal(knownDefault('claude-opus-5-20260101'), 'high');
  assert.equal(knownDefault('models/gemini-3-flash-preview'), 'high');
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

test('DeepSeek models default to high; levels are none / low / high / max', () => {
  for (const m of ['deepseek-v4', 'deepseek-reasoner', 'deepseek/deepseek-chat', 'us.deepseek.r1-v1:0']) assert.equal(knownDefault(m), 'high', m);
  const ds = ['none', 'low', 'high', 'max'];
  assert.equal(levelForFamily('none', ds), 'none');
  assert.equal(levelForFamily('max', ds), 'max');
  assert.equal(levelForFamily('medium', ds), 'high');
  assert.equal(levelForFamily('xhigh', ds), 'high');
});

test('Gemma models default to True; the level list is true / false', () => {
  for (const m of ['gemma-4-31b-it', '@cf/google/gemma-4-26b-a4b-it', 'google/gemma-3-27b']) assert.equal(knownDefault(m), 'true', m);
  const g = ['true', 'false'];
  assert.equal(levelForFamily('false', g), 'false');
  assert.equal(levelForFamily('true', g), 'true');
  assert.equal(levelForFamily('high', g), 'true');
  assert.equal(levelForFamily(null, g), 'true');
});
