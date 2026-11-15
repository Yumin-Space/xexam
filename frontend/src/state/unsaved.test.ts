import assert from 'node:assert/strict';
import test from 'node:test';
import { nextUnsaved } from './unsaved.ts';

test('unsaved -> saving -> saved, or back to unsaved on a failure', () => {
  assert.equal(nextUnsaved('unsaved', 'retry'), 'saving');
  assert.equal(nextUnsaved('saving', 'ok'), 'saved');
  assert.equal(nextUnsaved('saving', 'fail'), 'unsaved');
});

test('other events change nothing (a double tap, a late answer)', () => {
  assert.equal(nextUnsaved('saving', 'retry'), 'saving');
  assert.equal(nextUnsaved('unsaved', 'ok'), 'unsaved');
  assert.equal(nextUnsaved('unsaved', 'fail'), 'unsaved');
  assert.equal(nextUnsaved('saved', 'retry'), 'saved');
  assert.equal(nextUnsaved('saved', 'fail'), 'saved');
});
