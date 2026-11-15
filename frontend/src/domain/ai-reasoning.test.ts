// v1.3: the reasoning block of AI Chat: the `reasoning` / `reasoningDone` events, the view rules (live, open, seconds), the
// status line while reasoning streams, polled chats, and the four catalog strings. Made-up text only.
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  applyEvent, emptyChat, footerOf, isThinking, pollResult, reasoningSeconds, reasoningView, type ChatView, type ViewMessage,
} from './ai-chat.ts';
import ai from '../i18n/catalog/ai.ts';
import type { AiChatDetail, AiMessage } from '../types/api.ts';

const NOW = 1_800_000_000;
const START = { chatId: 1, chatTitle: 'T', userMessageId: 10, assistantMessageId: 11, model: 'm', newChat: true, free: false, mode: 'send' };
const msg = (o: Partial<AiMessage> & Pick<AiMessage, 'id' | 'role'>): AiMessage => ({
  content: '', status: 'done', error: null, truncated: false, model: null, createdAt: NOW, free: false, ...o,
});
const run = (s: ChatView, ...evs: Array<[string, unknown]>): ChatView => evs.reduce((acc, [event, data]) => applyEvent(acc, { event, data }, NOW), s);
const started = (): ChatView => run(emptyChat(), ['start', START]);
const answer = (s: ChatView): ViewMessage => s.messages.find((m) => m.id === 11) as ViewMessage;

test('reasoning frames append to the streaming answer while it has no text', () => {
  const s = run(started(), ['thinking', {}], ['reasoning', { text: 'a' }], ['reasoning', { text: 'b' }]);
  assert.equal(answer(s).reasoning, 'ab');
  assert.equal(isThinking(s), true);
});

test('reasoning frames are ignored after a delta, without a stream, and for empty or non-string text', () => {
  const s = run(started(), ['reasoning', { text: 'a' }], ['delta', { text: 'Hi' }], ['reasoning', { text: 'late' }]);
  assert.equal(answer(s).reasoning, 'a');
  const e = emptyChat();
  assert.equal(applyEvent(e, { event: 'reasoning', data: { text: 'x' } }, NOW), e);
  const b = started();
  for (const text of ['', 5, null, undefined, {}]) assert.equal(applyEvent(b, { event: 'reasoning', data: { text } }, NOW), b);
  assert.equal(applyEvent(b, { event: 'reasoning', data: null }, NOW), b);
});

test('reasoningDone sets the duration and the cut flag; a bad ms is ignored', () => {
  let s = run(started(), ['reasoning', { text: 'a' }], ['reasoningDone', { ms: 2400, truncated: true }]);
  assert.equal(answer(s).reasoningMs, 2400);
  assert.equal(answer(s).reasoningTruncated, true);
  for (const ms of ['5', -1, Number.NaN, null]) {
    s = run(s, ['reasoningDone', { ms }]);
    assert.equal(answer(s).reasoningMs, 2400);
    assert.equal(answer(s).reasoningTruncated, false);
  }
});

test('an in-place retry start clears the reasoning; a regenerate start gives a new answer without it', () => {
  const failed: ChatView = {
    ...emptyChat(), chatId: 1, messages: [msg({ id: 10, role: 'user', content: 'Q' }),
      msg({ id: 11, role: 'assistant', status: 'error', error: 'ai.internal', reasoning: 'old', reasoningMs: 900, reasoningTruncated: true })],
  };
  const r = run(failed, ['start', { ...START, retry: true, newChat: false }]);
  assert.equal(answer(r).reasoning, null);
  assert.equal(answer(r).reasoningMs, null);
  assert.equal(answer(r).reasoningTruncated, false);
  const g = run(failed, ['start', { ...START, assistantMessageId: 12, newChat: false, mode: 'regenerate', parentId: 10 }]);
  const fresh = g.messages.find((m) => m.id === 12);
  assert.equal(fresh?.reasoning, undefined);
  assert.equal(fresh?.reasoningMs, undefined);
});

test('reasoningView: no block for users, absent, null or blank reasoning', () => {
  const base = { status: 'done' as const, content: 'A' };
  assert.equal(reasoningView({ ...base, role: 'user', reasoning: 'x' }), null);
  assert.equal(reasoningView({ ...base, role: 'assistant' }), null);
  assert.equal(reasoningView({ ...base, role: 'assistant', reasoning: null }), null);
  assert.equal(reasoningView({ ...base, role: 'assistant', reasoning: ' \n\t ' }), null);
});

test('reasoningView: live is open unless the reader closed it; finished is closed unless the reader opened it', () => {
  const live = { role: 'assistant' as const, status: 'streaming' as const, content: '', reasoning: '\n  step one', reasoningMs: null };
  assert.deepEqual(reasoningView(live), { text: 'step one', live: true, open: true, seconds: null, truncated: false });
  assert.equal(reasoningView(live, false)?.open, false);
  assert.equal(reasoningView(live, true)?.open, true);
  const done = { ...live, content: 'Hi', reasoningMs: 1500, reasoning: ' step one \n', reasoningTruncated: true };
  assert.deepEqual(reasoningView(done), { text: 'step one', live: false, open: false, seconds: 2, truncated: true });
  assert.equal(reasoningView(done, true)?.open, true);
  assert.equal(reasoningView(done, false)?.open, false);
  assert.equal(reasoningView({ ...live, status: 'done' })?.live, false);
  assert.equal(reasoningView({ ...live, reasoningMs: 700 })?.live, false);
});

test('reasoningSeconds rounds and never shows less than 1', () => {
  for (const [ms, n] of [[0, 1], [400, 1], [1499, 1], [1500, 2], [61000, 61]] as const) assert.equal(reasoningSeconds(ms), n);
});

test('footerOf is null while reasoning text streams without answer text, and unchanged otherwise', () => {
  const m = { role: 'assistant' as const, status: 'streaming' as const, content: '', error: null, truncated: false, thinking: true };
  assert.deepEqual(footerOf(m), { kind: 'thinking' });
  assert.equal(footerOf({ ...m, reasoning: 'x' }), null);
  assert.notEqual(footerOf({ ...m, reasoning: '  ' }), null);
  assert.deepEqual(footerOf({ ...m, reasoning: 'x', content: 'Hi' }), { kind: 'writing' });
  assert.deepEqual(footerOf({ ...m, status: 'stopped', reasoning: 'x' }), { kind: 'stopped' });
});

test('pollResult keeps the reasoning fields of the server', () => {
  const d = {
    chat: { id: 1, title: 'T', running: false, kind: 'chat', pick: null },
    messages: [msg({ id: 11, role: 'assistant', content: 'A', reasoning: 'why', reasoningMs: 3200, reasoningTruncated: false })],
  } as unknown as AiChatDetail;
  const s = pollResult({ ...emptyChat(), chatId: 1 }, d);
  assert.equal(s.messages[0]?.reasoning, 'why');
  assert.equal(s.messages[0]?.reasoningMs, 3200);
  assert.equal(s.messages[0]?.reasoningTruncated, false);
});

test('an unknown event changes nothing', () => {
  const s = started();
  assert.equal(applyEvent(s, { event: 'reasoningX', data: { text: 'a' } }, NOW), s);
});

test('the catalog has the four reasoning strings in en, zh and ja', () => {
  const want = {
    en: ['Thinking…', 'Thought for {n} s', 'Reasoning', 'The reasoning was too long; only its beginning was kept.'],
    zh: ['思考中…', '已思考 {n} 秒', '思考过程', '思考内容过长，只保留了开头部分。'],
    ja: ['思考中…', '{n} 秒間思考', '思考過程', '思考内容が長すぎるため、先頭部分のみ保存しました。'],
  } as const;
  const keys = ['ai.reasoning.live', 'ai.reasoning.done', 'ai.reasoning.label', 'ai.reasoning.truncated'];
  const cat = ai as unknown as Record<string, Record<string, string>>;
  for (const lang of ['en', 'zh', 'ja'] as const) keys.forEach((k, i) => assert.equal(cat[lang]?.[k], want[lang][i], `${lang} ${k}`));
});
