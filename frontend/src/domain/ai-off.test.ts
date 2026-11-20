import assert from 'node:assert/strict';
import { test } from 'node:test';
import { connectionOffKey, featureOffKey } from './ai-off.ts';

test('feature-off key depends on the role', () => {
  assert.equal(featureOffKey(false), 'ai.err.featureOff');
  assert.equal(featureOffKey(true), 'ai.err.featureOffAdmin');
});

test('connection-off key depends on the role', () => {
  assert.equal(connectionOffKey(false), 'ai.err.connectionOff');
  assert.equal(connectionOffKey(true), 'ai.err.connectionOffAdmin');
});
