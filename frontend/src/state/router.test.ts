import assert from 'node:assert/strict';
import test from 'node:test';
import { parseHash, routeHash } from '../router.ts';

test('AI Chat: #/ai/<id> keeps the open chat across a reload', () => {
  assert.deepEqual(parseHash('#/ai'), { name: 'ai' });
  assert.deepEqual(parseHash('#/ai/12'), { name: 'ai', chatId: 12 });
  assert.deepEqual(parseHash('#/ai/abc'), { name: 'ai' });
  assert.deepEqual(parseHash('#/ai/new'), { name: 'ai', fresh: true });
  assert.equal(routeHash({ name: 'ai', fresh: true }), '#/ai/new');
  assert.equal(routeHash({ name: 'ai' }), '#/ai');
  assert.equal(routeHash({ name: 'ai', chatId: 7 }), '#/ai/7');
  assert.equal(routeHash({ name: 'aiSettings' }), '#/ai-settings');
});
