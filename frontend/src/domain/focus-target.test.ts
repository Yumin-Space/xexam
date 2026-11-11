import test from 'node:test';
import assert from 'node:assert/strict';
import { pickFocusTarget } from './ai-popover.ts';

test('a lost focus goes to the header button when there is one', () => {
  assert.equal(pickFocusTarget('stop', 'title'), 'stop');
});

test('a lost focus goes to the title when the header has no usable button', () => {
  assert.equal(pickFocusTarget(null, 'title'), 'title');
  assert.equal(pickFocusTarget(undefined, undefined), null);
});
