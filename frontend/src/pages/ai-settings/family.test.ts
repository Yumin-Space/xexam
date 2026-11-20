import assert from 'node:assert/strict';
import { test } from 'node:test';
import { followFamily, inferFamily, savedFamily } from './family.ts';

test('family follows the provider, format and address', () => {
  assert.equal(inferFamily('anthropic', 'openai', 'https://x.example/v1'), 'anthropic');
  assert.equal(inferFamily('gateway', 'anthropic', 'https://gw.example/anthropic'), 'anthropic');
  assert.equal(inferFamily('gateway', 'responses', 'https://gw.example'), 'responses');
  assert.equal(inferFamily('gateway', 'openai', 'https://gw.example/v1'), 'openai');
  assert.equal(inferFamily('openai', 'openai', 'https://generativelanguage.example/v1beta/openai'), 'google');
  assert.equal(inferFamily('gateway', 'openai', 'https://gw.example/google-ai/v1'), 'google');
  assert.equal(inferFamily('openai', 'openai', 'https://api.example/v1'), 'openai');
});

test('followFamily keeps a hand-chosen family and re-infers otherwise', () => {
  assert.deepEqual(followFamily({ value: 'google', byHand: true }, 'anthropic', 'anthropic', 'https://a.example'), { value: 'google', byHand: true });
  assert.deepEqual(followFamily({ value: 'openai', byHand: false }, 'gateway', 'responses', 'https://a.example'), { value: 'responses', byHand: false });
});

test('a saved connection follows the format again unless its family differs from the inferred one', () => {
  const same = savedFamily('openai', 'gateway', 'openai', 'https://gw.example/v1');
  assert.deepEqual(same, { value: 'openai', byHand: false });
  // a provider switch to Anthropic now moves the family
  assert.deepEqual(followFamily(same, 'anthropic', 'openai', 'https://gw.example/v1'), { value: 'anthropic', byHand: false });
  // the upgrade read Gemini model names on an OpenAI wire: kept
  const read = savedFamily('google', 'gateway', 'openai', 'https://gw.example/v1');
  assert.deepEqual(read, { value: 'google', byHand: true });
  assert.deepEqual(followFamily(read, 'gateway', 'anthropic', 'https://gw.example/v1'), { value: 'google', byHand: true });
});
