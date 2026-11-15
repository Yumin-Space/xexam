// modelRefText / parseModelRef (v1.3): `<connectionId>:<model>`, split at the first colon. Run with `npm test`.
import assert from 'node:assert/strict';
import test from 'node:test';
import { modelRefText, parseModelRef } from './model-ref.ts';

test('modelRefText joins the connection id and the model with a colon', () => {
  assert.equal(modelRefText({ connectionId: 2, model: 'demo-chat-small' }), '2:demo-chat-small');
});

test('parseModelRef splits at the first colon only: model names may contain colons', () => {
  assert.deepEqual(parseModelRef('3:demo:large:v2'), { connectionId: 3, model: 'demo:large:v2' });
  assert.deepEqual(parseModelRef('12:demo-model'), { connectionId: 12, model: 'demo-model' });
});

test('parseModelRef round-trips modelRefText', () => {
  for (const model of ['a', 'demo:x', 'demo/x:y:z', 'with space']) {
    const ref = { connectionId: 7, model };
    assert.deepEqual(parseModelRef(modelRefText(ref)), ref);
  }
});

test('parseModelRef refuses text that is not <positive integer>:<model>', () => {
  for (const bad of ['', 'demo', ':demo', '0:demo', '-1:demo', '1.5:demo', 'x:demo', '2:', '02:demo', '1234567890123456:demo']) {
    assert.equal(parseModelRef(bad), null, bad);
  }
});
