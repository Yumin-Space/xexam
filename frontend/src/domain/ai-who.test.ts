// v1.3 sprint 2: what the line above a message names. Administrators see "connection · model", everyone else only the "AI"
// mark. Made-up names only. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import test from 'node:test';
import { applyEvent, chatShowsModel, emptyChat, startOf, whoParts } from './ai-chat.ts';

const MODEL = 'demo-chat-small';
const CONN = 'Demo link';
const NOW = 1_700_000_000;

test('a question is always "you"', () => {
  for (const admin of [true, false]) {
    assert.deepEqual(whoParts({ role: 'user', model: MODEL, connection: CONN }, admin), { mark: 'you', connection: null, model: null });
  }
});

test('a non-administrator never sees the model or connection, even in stale data', () => {
  assert.deepEqual(whoParts({ role: 'assistant', model: MODEL, connection: CONN }, false), { mark: 'ai', connection: null, model: null });
});

test('an answer without a model is the "AI" mark for everyone', () => {
  for (const admin of [true, false]) {
    for (const model of [undefined, null, '']) {
      assert.equal(whoParts({ role: 'assistant', model }, admin).mark, 'ai');
    }
  }
});

test('an administrator sees connection and model', () => {
  assert.deepEqual(whoParts({ role: 'assistant', model: MODEL, connection: CONN }, true), { mark: null, connection: CONN, model: MODEL });
});

test('an administrator with a model and no connection keeps the model only', () => {
  for (const connection of [undefined, null, '']) {
    assert.deepEqual(whoParts({ role: 'assistant', model: MODEL, connection }, true), { mark: null, connection: null, model: MODEL });
  }
});

test('a non-administrator start frame yields no model and no connection', () => {
  const data = {
    chatId: 11, chatTitle: 'T', userMessageId: 21, assistantMessageId: 22, newChat: true, retry: false, free: false,
    chatKind: 'chat', mode: 'send', parentId: null, userParentId: null,
  };
  const d = startOf(data)!;
  assert.equal(d.model, '');
  assert.equal(d.connection, undefined);
  const s = applyEvent(emptyChat(), { event: 'start', data }, NOW);
  const msg = s.messages[s.messages.length - 1]!;
  assert.ok(!msg.model);
  assert.equal(msg.connection, undefined);
  assert.equal(whoParts(msg, false).mark, 'ai');
});

test('an administrator start frame names both', () => {
  const data = {
    chatId: 11, chatTitle: 'T', userMessageId: 21, assistantMessageId: 22, newChat: true, retry: false, free: false,
    model: MODEL, connection: CONN,
  };
  const s = applyEvent(emptyChat(), { event: 'start', data }, NOW);
  const msg = s.messages[s.messages.length - 1]!;
  assert.deepEqual(whoParts(msg, true), { mark: null, connection: CONN, model: MODEL });
});

test('v1.4: the model is named only for an administrator in the free mode (where it can be chosen)', () => {
  assert.equal(chatShowsModel(true, true), true);
  assert.equal(chatShowsModel(true, false), false, 'an administrator in the normal mode sees only "AI"');
  assert.equal(chatShowsModel(false, true), false);
  assert.equal(chatShowsModel(false, false), false);
  const answer = { role: 'assistant' as const, model: MODEL, connection: CONN };
  assert.deepEqual(whoParts(answer, chatShowsModel(true, false)), { mark: 'ai', connection: null, model: null });
  assert.deepEqual(whoParts(answer, chatShowsModel(true, true)), { mark: null, connection: CONN, model: MODEL });
});
