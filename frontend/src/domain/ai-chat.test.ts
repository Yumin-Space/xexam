// AI Chat's conversation state (v0.8): the reducer over a streamed answer, Stop, a polled chat, Retry, the status line, and
// the composer's Enter rule with input methods (Safari's event order, keyCode 229), Shift+Enter, touch screens and
// Ctrl/Cmd+Enter. Round 2: the "thinking" event, free mode (the switch) and the request
// bodies with the reply language. Made-up text only. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  applyEvent, atEnd, beginEdit, canEdit, canRegenerate, chatOf, clearsComposer, composerLayout, composerLines, composerMaxLines, composerStatus, emptyChat, fmtAiTime, footerOf, ignoreSendClick, isBusyElsewhere, isTerminal, isThinking, localFail, MIN_MSGS_PX,
  localEdit, localRegenerate, modelArg, chatMenu, chatShown, pickerView, chooseValue, regenerateLabelKey, editValid, versionInfo, TEMP_ID,
  effortArg, keepEffort, strengthView, slotRoute, groupChats, filterChats, entryTarget, drawerBack, localDayKey, latestChat, chatAddress, DRAWER_ENTRY, isDrawerEntry, staleDrawerEntry,
  localSend, localStop, locksDocument, noAccessKey, pollResult, questionOf, readOnlyReason, retryBody, sendBody, shouldSend, showsEmptyHint, showsTwoPanes, startOf, STOP_GUARD_MS, suggestBody,
  type ChatView, type KeyLike, type ViewMessage,
} from './ai-chat.ts';
import { parseModelRef } from '../api/model-ref.ts';
import type { AiChatDetail, AiMessage, AiModelList, AiSettings } from '../types/api.ts';
import type { ChatDefault } from './ai-chat.ts';

const NOW = 1_800_000_000;

const msg = (o: Partial<AiMessage> & Pick<AiMessage, 'id' | 'role'>): AiMessage => ({
  content: '', status: 'done', error: null, truncated: false, model: null, createdAt: NOW, free: false, ...o,
});

const start = (o: Partial<{ chatId: number; chatTitle: string; userMessageId: number; assistantMessageId: number; model: string; newChat: boolean; retry: boolean }> = {}) => ({
  event: 'start',
  data: { chatId: 11, chatTitle: 'Photosynthesis basics', userMessageId: 21, assistantMessageId: 22, model: 'demo-chat-small', newChat: true, retry: false, ...o },
});

function run(s: ChatView, ...events: Array<{ event: string; data: unknown }>): ChatView {
  return events.reduce((acc, ev) => applyEvent(acc, ev, NOW), s);
}

test('a new chat: start adopts the id and title, adds the question and the answer; deltas append; done ends it', () => {
  let s = localSend(emptyChat(), 'What does a leaf need to make sugar?');
  assert.deepEqual(s.pending, { text: 'What does a leaf need to make sugar?', retry: false });
  s = run(s, start());
  assert.equal(s.chatId, 11);
  assert.equal(s.title, 'Photosynthesis basics');
  assert.equal(s.pending, null);
  assert.equal(s.streamingId, 22);
  assert.equal(s.running, true);
  assert.deepEqual(s.messages.map((m) => [m.id, m.role, m.status, m.content]), [
    [21, 'user', 'done', 'What does a leaf need to make sugar?'],
    [22, 'assistant', 'streaming', ''],
  ]);
  s = run(s, { event: 'delta', data: { text: 'Light, ' } }, { event: 'delta', data: { text: 'water and air.' } });
  assert.equal(s.messages[1]?.content, 'Light, water and air.');
  s = run(s, { event: 'done', data: { status: 'done', tokensIn: 12, tokensOut: 5, truncated: false } });
  assert.equal(s.streamingId, null);
  assert.equal(s.running, false);
  assert.equal(s.messages[1]?.status, 'done');
  assert.equal(s.messages[1]?.model, 'demo-chat-small');
  assert.equal(footerOf(s.messages[1] as AiMessage), null);
});

test('stopped and error end the answer and keep the text; truncated is marked', () => {
  const base = run(localSend(emptyChat(), 'q'), start(), { event: 'delta', data: { text: 'Part of it' } });
  const stopped = run(base, { event: 'stopped', data: { status: 'stopped' } });
  assert.equal(stopped.messages[1]?.status, 'stopped');
  assert.equal(stopped.messages[1]?.content, 'Part of it');
  assert.deepEqual(footerOf(stopped.messages[1] as AiMessage), { kind: 'stopped' });

  const failed = run(base, { event: 'error', data: { status: 'error', code: 'ai.up.timeout', error: '…' } });
  assert.equal(failed.messages[1]?.status, 'error');
  assert.equal(failed.messages[1]?.error, 'ai.up.timeout');
  assert.deepEqual(footerOf(failed.messages[1] as AiMessage), { kind: 'error', code: 'ai.up.timeout' });

  const empty = run(localSend(emptyChat(), 'q'), start(), { event: 'error', data: { status: 'error', code: 'ai.up.auth' } });
  assert.deepEqual(footerOf(empty.messages[1] as AiMessage), { kind: 'noAnswer', code: 'ai.up.auth' });

  const cut = run(base, { event: 'done', data: { status: 'done', truncated: true } });
  assert.equal(cut.messages[1]?.truncated, true);
  assert.deepEqual(footerOf(cut.messages[1] as AiMessage), { kind: 'truncated' });
  assert.deepEqual(footerOf(base.messages[1] as AiMessage), { kind: 'writing' });

  assert.ok(isTerminal('done') && isTerminal('stopped') && isTerminal('error'));
  assert.ok(!isTerminal('delta') && !isTerminal('start'));
});

test('events after the end, unknown events and malformed data change nothing', () => {
  const done = run(localSend(emptyChat(), 'q'), start(), { event: 'done', data: { status: 'done' } });
  assert.deepEqual(run(done, { event: 'delta', data: { text: 'late' } }), done);
  assert.deepEqual(run(done, { event: 'something-new', data: {} }), done);
  assert.deepEqual(run(emptyChat(), { event: 'start', data: { chatId: 'x' } }), emptyChat());
  assert.equal(startOf(null), null);
});

test('a second message in an open chat keeps the history', () => {
  const first = run(localSend(emptyChat(), 'one'), start(), { event: 'done', data: { status: 'done' } });
  const second = run(localSend(first, 'two'), start({ userMessageId: 23, assistantMessageId: 24, newChat: false, chatTitle: '' }));
  assert.equal(second.title, 'Photosynthesis basics', 'an empty title keeps the old one');
  assert.deepEqual(second.messages.map((m) => m.id), [21, 22, 23, 24]);
});

test('Stop marks the answer stopped at once; Stop before the start only forgets the pending send', () => {
  const writing = run(localSend(emptyChat(), 'q'), start(), { event: 'delta', data: { text: 'So far' } });
  const s = localStop(writing);
  assert.equal(s.messages[1]?.status, 'stopped');
  assert.equal(s.messages[1]?.content, 'So far');
  assert.equal(s.streamingId, null);
  assert.equal(s.running, false);
  const before = localStop(localSend(emptyChat(), 'q'));
  assert.equal(before.pending, null);
  assert.deepEqual(before.messages, []);
  assert.equal(localFail(localSend(emptyChat(), 'q')).pending, null);
});

const detail = (running: boolean, messages: AiMessage[], id = 11): AiChatDetail => ({
  chat: { id, title: 'Photosynthesis basics', createdAt: NOW, updatedAt: NOW, messageCount: messages.length, running },
  messages,
});

test('a polled chat replaces the messages; an answer stopped here stays stopped; another chat is ignored', () => {
  const writing = run(localSend(emptyChat(), 'q'), start(), { event: 'delta', data: { text: 'A' } });
  // the server has written more meanwhile
  let s = pollResult(writing, detail(true, [msg({ id: 21, role: 'user', content: 'q' }), msg({ id: 22, role: 'assistant', content: 'A longer text', status: 'streaming' })]));
  assert.equal(s.messages[1]?.content, 'A longer text');
  assert.equal(s.streamingId, 22);
  assert.equal(s.running, true);
  // finished
  s = pollResult(s, detail(false, [msg({ id: 21, role: 'user', content: 'q' }), msg({ id: 22, role: 'assistant', content: 'The whole text' })]));
  assert.equal(s.streamingId, null);
  assert.equal(s.running, false);
  assert.equal(s.messages[1]?.status, 'done');

  const stopped = localStop(writing);
  const polled = pollResult(stopped, detail(true, [msg({ id: 21, role: 'user' }), msg({ id: 22, role: 'assistant', content: 'A b', status: 'streaming' })]));
  assert.equal(polled.messages[1]?.status, 'stopped');
  assert.equal(polled.messages[1]?.content, 'A b');

  assert.equal(pollResult(writing, detail(false, [], 99)), writing);
  // opening a chat whose answer is running
  const opened = chatOf(detail(true, [msg({ id: 1, role: 'user' }), msg({ id: 2, role: 'assistant', status: 'streaming' })]));
  assert.equal(opened.streamingId, 2);
  assert.equal(opened.running, true);
});

test('Retry: only the last answer after a question, when it failed or was stopped; start with retry replaces it', () => {
  const failed = run(localSend(emptyChat(), 'q'), start(), { event: 'delta', data: { text: 'Half' } },
    { event: 'error', data: { status: 'error', code: 'ai.up.overloaded' } });
  const answer = (v: ChatView): ViewMessage => v.messages[1] as ViewMessage;
  assert.equal(canRegenerate(failed, answer(failed)), true);
  assert.equal(regenerateLabelKey(failed, answer(failed)), 'ai.retry', 'a failed last answer keeps the name Retry');
  const done = run(localSend(emptyChat(), 'q'), start(), { event: 'done', data: { status: 'done' } });
  assert.equal(canRegenerate(done, answer(done)), true, 'a finished answer can be regenerated too');
  assert.equal(regenerateLabelKey(done, answer(done)), 'ai.regenerate');
  const halted = localStop(run(localSend(emptyChat(), 'q'), start()));
  assert.equal(canRegenerate(halted, answer(halted)), true);
  assert.equal(regenerateLabelKey(halted, answer(halted)), 'ai.retry');
  assert.equal(canRegenerate(localSend(failed, '', true), answer(failed)), false, 'not while a request runs');

  const again = run(localSend(failed, '', true), start({ retry: true, newChat: false, userMessageId: 21 }));
  assert.equal(again.messages.length, 2, 'no new message');
  assert.deepEqual([again.messages[1]?.id, again.messages[1]?.status, again.messages[1]?.content, again.messages[1]?.error], [22, 'streaming', '', null]);
  assert.equal(again.streamingId, 22);
  assert.equal(questionOf(again, 22)?.id, 21);
  assert.equal(questionOf(again, 21), null);
});

test('the composer is emptied by the start of a send, never by the start of a retry (a draft typed meanwhile stays)', () => {
  // a send: its text came from the composer
  const sent = localSend(emptyChat(), 'What does a leaf need to make sugar?');
  assert.equal(clearsComposer(sent, start()), true);
  // the answer was stopped, the user typed the next question (not sent), then pressed Retry
  const stopped = localStop(run(localSend(emptyChat(), 'q'), start(), { event: 'delta', data: { text: 'Part' } }));
  const retrying = localSend(stopped, '', true);
  const retryStart = start({ retry: true, newChat: false, userMessageId: 21 });
  assert.equal(clearsComposer(retrying, retryStart), false, 'the draft in the composer stays after the retry starts');
  assert.equal(clearsComposer(stopped, retryStart), false, 'a start marked retry never clears, even without a pending retry');
  assert.equal(clearsComposer(retrying, start()), false, 'a pending retry never clears');
  // other events and malformed starts never clear
  assert.equal(clearsComposer(sent, { event: 'delta', data: { text: 'x' } }), false);
  assert.equal(clearsComposer(sent, { event: 'done', data: { status: 'done' } }), false);
  assert.equal(clearsComposer(sent, { event: 'start', data: { chatId: 'x' } }), false);
});

test('a double-click on Send does not stop the answer it started; a later click or Enter / Space on Stop stops it', () => {
  // the second click of a double-click (CDP clickCount 2, 120 ms later), on Send or on Stop
  assert.equal(ignoreSendClick(2, true, 120), true);
  assert.equal(ignoreSendClick(2, false, 0), true);
  assert.equal(ignoreSendClick(3, true, 5000), true);
  // a single click right after Send turned into Stop
  assert.equal(ignoreSendClick(1, true, 120), true);
  assert.equal(ignoreSendClick(1, true, STOP_GUARD_MS - 1), true);
  // a deliberate Stop: a click, or the keyboard (click count 0), after the guard
  assert.equal(ignoreSendClick(1, true, STOP_GUARD_MS), false);
  assert.equal(ignoreSendClick(0, true, 2000), false);
  // Send itself is never ignored on a single click or the keyboard
  assert.equal(ignoreSendClick(1, false, 0), false);
  assert.equal(ignoreSendClick(0, false, 0), false);
});

const key = (o: Partial<KeyLike> = {}): KeyLike => ({
  key: 'Enter', shiftKey: false, ctrlKey: false, metaKey: false, altKey: false, isComposing: false, keyCode: 13, ...o,
});
const fine = { composing: false, finePointer: true };

test('Enter sends with a mouse; Shift+Enter and Enter on a touch screen make a new line; other keys never send', () => {
  assert.equal(shouldSend(key(), fine), true);
  assert.equal(shouldSend(key({ shiftKey: true }), fine), false);
  assert.equal(shouldSend(key(), { composing: false, finePointer: false }), false);
  assert.equal(shouldSend(key({ ctrlKey: true }), { composing: false, finePointer: false }), true, 'Ctrl+Enter sends on touch screens too');
  assert.equal(shouldSend(key({ metaKey: true }), fine), true, 'Cmd+Enter');
  assert.equal(shouldSend(key({ key: 'a', keyCode: 65 }), fine), false);
});

test('never while an input method composes: isComposing, keyCode 229, and Safari (compositionend before the keydown)', () => {
  assert.equal(shouldSend(key({ isComposing: true }), fine), false);
  assert.equal(shouldSend(key({ keyCode: 229 }), fine), false);
  // Safari: compositionend already fired (isComposing false), but the page's flag is cleared one tick later
  assert.equal(shouldSend(key(), { composing: true, finePointer: true }), false);
  assert.equal(shouldSend(key({ ctrlKey: true, isComposing: true }), fine), false, 'Ctrl+Enter is checked too');
  assert.equal(shouldSend(key({ metaKey: true, keyCode: 229 }), fine), false);
});

test('chat times: the time only today, the date and the time otherwise', () => {
  // (minute 7: in every time zone of the test machine "now" and a minute earlier are on the same local day)
  const now = Date.UTC(2026, 10, 11, 15, 7, 30);
  const today = fmtAiTime(Math.floor(now / 1000) - 60, 'en', now);
  const earlier = fmtAiTime(Math.floor(now / 1000) - 40 * 86400, 'en', now);
  assert.ok(/\d{1,2}:\d{2}/.test(today), today);
  assert.ok(!/[A-Za-z]{3}/.test(today.replace(/AM|PM/i, '')), `no month today: ${today}`);
  assert.ok(/[A-Za-z]{3}/.test(earlier), `a month name: ${earlier}`);
  assert.ok(fmtAiTime(Math.floor(now / 1000), 'ja', now).length > 0);
});

// ---------------------------------------------------------------- round 2

const none: ViewMessage = msg({ id: 0, role: 'assistant' });

test('thinking: "Thinking…" under the answer until its first text, then "Writing…"; a late or repeated one is ignored', () => {
  let s = run(localSend(emptyChat(), 'Why is the sky blue?'), start());
  const answer = (v: ChatView) => v.messages.find((m) => m.id === 22) ?? none;
  assert.deepEqual(footerOf(answer(s)), { kind: 'writing' });
  s = run(s, { event: 'thinking', data: {} });
  assert.equal(answer(s).thinking, true);
  assert.deepEqual(footerOf(answer(s)), { kind: 'thinking' });
  // a second one changes nothing (the same object back)
  assert.equal(run(s, { event: 'thinking', data: {} }), s);
  s = run(s, { event: 'delta', data: { text: 'Light ' } });
  assert.equal(answer(s).thinking, false);
  assert.deepEqual(footerOf(answer(s)), { kind: 'writing' });
  // after text: ignored
  assert.equal(run(s, { event: 'thinking', data: {} }), s);
  s = run(s, { event: 'done', data: {} });
  assert.equal(answer(s).thinking, false);
  assert.equal(footerOf(answer(s)), null);
  // no answer being written: nothing to mark
  assert.equal(run(emptyChat(), { event: 'thinking', data: {} }).messages.length, 0);
  // thinking, then stopped before any text: "Stopped", not "Thinking…"
  const stopped = run(localSend(emptyChat(), 'q'), start(), { event: 'thinking', data: {} }, { event: 'stopped', data: {} });
  assert.deepEqual(footerOf(answer(stopped)), { kind: 'stopped' });
  // a polled chat shows the usual "Writing…" (the server does not keep the thinking state)
  const polled = pollResult(run(localSend(emptyChat(), 'q'), start(), { event: 'thinking', data: {} }),
    detail(true, [msg({ id: 21, role: 'user', content: 'q' }), msg({ id: 22, role: 'assistant', status: 'streaming' })]));
  assert.deepEqual(footerOf(answer(polled)), { kind: 'writing' });
});

test('free mode is no property of a chat: neither the start event nor an opened or polled chat carries one', () => {
  assert.ok(!('free' in emptyChat()));
  assert.ok(!('free' in run(localSend(emptyChat(), 'Hello'), start())));
  assert.ok(!('free' in chatOf(detail(false, []))));
  // only the messages carry the mode of their turn
  assert.equal(startOf(start().data)!.free, false);
});

test('the send body: chat, text and the reply language only; free mode lives on the server', () => {
  for (const lang of ['en', 'zh', 'ja'] as const) {
    assert.deepEqual(sendBody({ chatId: null, text: 'Hi', lang }), { chatId: null, text: 'Hi', lang });
  }
  assert.equal(JSON.stringify(sendBody({ chatId: 7, text: 'Hi', lang: 'zh' })), '{"chatId":7,"text":"Hi","lang":"zh"}');
});

test('the retry and suggest bodies carry the reply language', () => {
  assert.deepEqual(retryBody(7, 'ja'), { chatId: 7, lang: 'ja' });
  assert.deepEqual(suggestBody(9, 'zh'), { messageId: 9, lang: 'zh' });
});
test('the start event carries the mode into the new messages and a retried pair', () => {
  const s = run(localSend(emptyChat(), 'q'), start({ free: true } as never));
  assert.deepEqual(s.messages.map((x) => x.free), [true, true]);
  const failed = chatOf(detail(false, [msg({ id: 21, role: 'user' }), msg({ id: 22, role: 'assistant', status: 'error', error: 'ai.upstream' })]));
  const r = run(failed, start({ retry: true, newChat: false, free: true } as never));
  assert.deepEqual(r.messages.map((x) => x.free), [true, true]);
});

test('the list row says Thinking while the open answer thinks and has no text; Writing otherwise', () => {
  let s = run(localSend(emptyChat(), 'q'), start());
  assert.equal(isThinking(s), false);
  s = run(s, { event: 'thinking', data: {} });
  assert.equal(isThinking(s), true);
  s = run(s, { event: 'delta', data: { text: 'Leaves' } });
  assert.equal(isThinking(s), false);
  assert.equal(isThinking(emptyChat()), false);
});

test('the empty-chat hint shows only for a chat with nothing in it, nothing being sent and no answer running', () => {
  assert.equal(showsEmptyHint(emptyChat()), true);
  assert.equal(showsEmptyHint(localSend(emptyChat(), 'q')), false);
  assert.equal(showsEmptyHint(run(localSend(emptyChat(), 'q'), start())), false);
  assert.equal(showsEmptyHint(chatOf(detail(true, []))), false);
  assert.equal(showsEmptyHint(chatOf(detail(false, []))), true);
});

test('who may not chat and has nothing to read: the page names the cause, never the container', () => {
  assert.equal(noAccessKey({ enabled: true, suspended: true }), 'ai.noAccess.suspended');
  assert.equal(noAccessKey({ enabled: false, suspended: true }), 'ai.noAccess.suspended');
  assert.equal(noAccessKey({ enabled: false, suspended: false }), 'ai.noAccess.disabled');
  assert.equal(noAccessKey({ enabled: true, suspended: false }), 'ai.noAccess.noFn');
});

test('the read-only banner reason: paused, account without the function, or the feature not available', () => {
  assert.equal(readOnlyReason({ enabled: true, allowed: false, suspended: true }), 'suspended');
  assert.equal(readOnlyReason({ enabled: true, allowed: false, suspended: false }), 'noFunction');
  assert.equal(readOnlyReason({ enabled: false, allowed: false, suspended: false }), 'unavailable');
  assert.equal(readOnlyReason({ enabled: false, allowed: true, suspended: false }), 'unavailable');
});

// ---------------------------------------------------------------- v0.9: study chats keep their kind

test('v0.9: an AI Chat conversation is kind "chat"; a chat read from the server keeps its kind', () => {
  assert.equal(emptyChat().kind, 'chat');
  const plain = chatOf(detail(false, []));
  assert.equal(plain.kind, 'chat');
  const study: AiChatDetail = { ...detail(false, []), chat: { ...detail(false, []).chat, kind: 'practice', recordId: null } };
  assert.equal(chatOf(study).kind, 'practice');
  assert.equal(pollResult(emptyChat(), study).kind, 'practice');
});

test('v0.9: a follow-up in a study chat keeps the kind (the start event names it or says nothing); an unknown kind is ignored', () => {
  const open = chatOf({ ...detail(false, [msg({ id: 21, role: 'user' }), msg({ id: 22, role: 'assistant', content: 'A' })]),
    chat: { id: 11, title: '2026-11-12 21:58', createdAt: NOW, updatedAt: NOW, messageCount: 2, running: false, kind: 'exam' } });
  assert.equal(open.kind, 'exam');
  const quiet = run(localSend(open, 'and why?'), start({ chatId: 11, userMessageId: 23, assistantMessageId: 24, newChat: false }));
  assert.equal(quiet.kind, 'exam');
  const named = run(localSend(open, 'and why?'), { event: 'start', data: { ...start({ chatId: 11, userMessageId: 23, assistantMessageId: 24, newChat: false }).data, chatKind: 'exam' } });
  assert.equal(named.kind, 'exam');
  const odd = run(localSend(open, 'and why?'), { event: 'start', data: { ...start({ chatId: 11, userMessageId: 23, assistantMessageId: 24, newChat: false }).data, chatKind: 'nonsense' } });
  assert.equal(odd.kind, 'exam');
  // a new AI Chat conversation that the server calls "chat"
  const fresh = run(localSend(emptyChat(), 'hi'), { event: 'start', data: { ...start().data, chatKind: 'chat' } });
  assert.equal(fresh.kind, 'chat');
  assert.equal(startOf({ ...start().data, chatKind: 'summary' })?.chatKind, 'summary');
  assert.equal(startOf({ ...start().data, chatKind: 'assess' })?.chatKind, 'assess');
  assert.equal(startOf({ ...start().data, chatKind: 'bogus' })?.chatKind, null);
  assert.equal(startOf(start().data)?.chatKind, null);
});

test('v0.9: Retry is offered only under an AI Chat answer, never under a hint, explanation or summary', () => {
  const failed = (kind: 'chat' | 'hint' | 'explain' | 'summary') => chatOf(detail(false, [
    msg({ id: 31, role: 'user', content: 'q', kind }),
    msg({ id: 32, role: 'assistant', status: 'error', error: 'ai.up.timeout', kind }),
  ]));
  const ok = failed('chat');
  assert.equal(canRegenerate(ok, ok.messages[1] as ViewMessage), true);
  for (const k of ['hint', 'explain', 'summary'] as const) {
    const v = failed(k);
    assert.equal(canRegenerate(v, v.messages[1] as ViewMessage), false);
    assert.equal(canEdit(v, v.messages[0] as ViewMessage), false);
  }
});

test('v1.0: the phone screen locks the document up to 900 px and shows two panes from 720 px', () => {
  assert.equal(locksDocument(390), true);
  assert.equal(locksDocument(900), true);
  assert.equal(locksDocument(901), false);
  assert.equal(showsTwoPanes(719), false);
  assert.equal(showsTwoPanes(720), true);
  assert.equal(showsTwoPanes(900), true);
});

test('v1.0: the list is "at the end" within 24 px', () => {
  assert.equal(atEnd({ scrollHeight: 1000, scrollTop: 600, clientHeight: 400 }), true);
  assert.equal(atEnd({ scrollHeight: 1000, scrollTop: 576, clientHeight: 400 }), true);
  assert.equal(atEnd({ scrollHeight: 1000, scrollTop: 575, clientHeight: 400 }), false);
  // a list shorter than its box is always at its end
  assert.equal(atEnd({ scrollHeight: 200, scrollTop: 0, clientHeight: 400 }), true);
  assert.equal(atEnd({ scrollHeight: 1000, scrollTop: 500, clientHeight: 400 }, 100), true);
});

test('v1.0: the composer status slot shows one thing, errors first, then the cap, then "busy elsewhere"', () => {
  assert.equal(composerStatus({ sendError: 'x', capped: true, busyElsewhere: true }), 'error');
  assert.equal(composerStatus({ sendError: null, capped: true, busyElsewhere: true }), 'cap');
  assert.equal(composerStatus({ sendError: null, capped: false, busyElsewhere: true }), 'busy');
  assert.equal(composerStatus({ sendError: '', capped: false, busyElsewhere: false }), null);
});

test('v1.0: another chat is busy only when the running answer is not this chat and not sent from here', () => {
  assert.equal(isBusyElsewhere(null, 3, false), false);
  assert.equal(isBusyElsewhere({ chatId: 3 }, 3, false), false);
  assert.equal(isBusyElsewhere({ chatId: 4 }, 3, false), true);
  assert.equal(isBusyElsewhere({ chatId: 4 }, null, false), true);
  assert.equal(isBusyElsewhere({ chatId: 4 }, 3, true), false);
});

test('v1.0: the composer grows to 4 lines on touch or in a short window, else 8', () => {
  assert.equal(composerMaxLines(true, 1000), 8);
  assert.equal(composerMaxLines(false, 699), 8);
  assert.equal(composerMaxLines(false, 700), 8);
});

test('composerLines follows the room left (keyboard, short landscape)', () => {
  assert.equal(composerLines(true, 844, 800, 24), 8);
  assert.equal(composerLines(true, 844, 400, 24), 6);
  assert.equal(composerLines(true, 740, 407, 24), 6);
  assert.equal(composerLines(true, 390, 215, 24), 3);
  assert.equal(composerLines(true, 390, 50, 24), 1);
  assert.equal(composerLines(false, 1000, 1000, 22), 8);
});

test('composerLines keeps the conversation room: the text room caps the lines, at least 1', () => {
  // 390x844 with the keyboard: 153 px free above a one-line composer, 44 px input with 18 px padding and border
  assert.equal(composerLines(true, 844, 464, 24, 153 + 44 - 18 - MIN_MSGS_PX), 3);
  // 360x740 with the keyboard: 96 px free, the input stays at one line
  assert.equal(composerLines(true, 740, 407, 24, 96 + 44 - 18 - MIN_MSGS_PX), 1);
  assert.equal(composerLines(true, 844, 844, 24, -50), 1);
  assert.equal(composerLines(true, 844, 844, 24, 1000), 8);
  assert.equal(composerLines(true, 844, 844, 24, Number.POSITIVE_INFINITY), 8);
});

test('readOnlyReason (v1.1): the chat feature unticked, and the cause named by `why`', () => {
  const base = { enabled: true, allowed: true, suspended: false };
  assert.equal(readOnlyReason({ ...base, features: { chat: false } }), 'featureOff');
  assert.equal(readOnlyReason({ ...base, features: { chat: true } }), 'unavailable');
  assert.equal(readOnlyReason({ ...base, suspended: true, features: { chat: false } }), 'suspended');
  assert.equal(readOnlyReason({ enabled: false, allowed: false, suspended: false, why: 'off' }), 'off');
  assert.equal(readOnlyReason({ enabled: false, allowed: false, suspended: false, why: 'link' }), 'down');
  assert.equal(readOnlyReason({ enabled: false, allowed: false, suspended: false, why: null }), 'unavailable');
  assert.equal(readOnlyReason({ enabled: true, allowed: false, suspended: false, why: null }), 'noFunction');
});

// ---------------------------------------------------------------- v1.3: branches, regenerate, edit, versions, the model menu

/** A path of three turns: q1 (10) a1 (11) q2 (12) a2 (13); a1 has versions 11 / 20, q1 has versions 10 / 30 */
const tree = (): ChatView => chatOf(detail(false, [
  msg({ id: 10, role: 'user', content: 'q1', parentId: null, versions: [10, 30] }),
  msg({ id: 11, role: 'assistant', content: 'a1', parentId: 10, versions: [11, 20] }),
  msg({ id: 12, role: 'user', content: 'q2', parentId: 11 }),
  msg({ id: 13, role: 'assistant', content: 'a2', parentId: 12 }),
]));

test('versionInfo: null with one version, else the position and the neighbours', () => {
  assert.equal(versionInfo(msg({ id: 1, role: 'user' })), null);
  assert.equal(versionInfo(msg({ id: 1, role: 'user', versions: [1] })), null);
  assert.equal(versionInfo(msg({ id: 5, role: 'user', versions: [1, 2] })), null, 'a list that does not hold the message');
  assert.deepEqual(versionInfo(msg({ id: 11, role: 'assistant', versions: [11, 20, 25] })), { index: 1, total: 3, prev: null, next: 20 });
  assert.deepEqual(versionInfo(msg({ id: 20, role: 'assistant', versions: [11, 20, 25] })), { index: 2, total: 3, prev: 11, next: 25 });
  assert.deepEqual(versionInfo(msg({ id: 25, role: 'assistant', versions: [11, 20, 25] })), { index: 3, total: 3, prev: 20, next: null });
});

test('questionOf follows parentId; without a parent field it falls back to the row before', () => {
  const s = tree();
  assert.equal(questionOf(s, 11)?.id, 10);
  assert.equal(questionOf(s, 13)?.id, 12);
  assert.equal(questionOf(s, 10), null);
  const odd = chatOf(detail(false, [msg({ id: 1, role: 'user', parentId: null }), msg({ id: 2, role: 'user' }), msg({ id: 3, role: 'assistant', parentId: 1 })]));
  assert.equal(questionOf(odd, 3)?.id, 1, 'by parent, not by the row before');
});

test('canRegenerate / canEdit: AI Chat messages only, never while something runs or starts', () => {
  const s = tree();
  const m = (id: number): ViewMessage => s.messages.find((x) => x.id === id) as ViewMessage;
  assert.equal(canRegenerate(s, m(11)), true);
  assert.equal(canRegenerate(s, m(10)), false, 'a question has no Regenerate');
  assert.equal(canEdit(s, m(12)), true);
  assert.equal(canEdit(s, m(13)), false);
  const busy = { ...s, running: true };
  assert.equal(canRegenerate(busy, m(11)), false);
  assert.equal(canEdit(busy, m(12)), false);
  assert.equal(canEdit(localSend(s, 'x'), m(12)), false);
  assert.equal(canRegenerate({ ...s, messages: s.messages.map((x) => (x.id === 10 ? { ...x, kind: 'hint' as const } : x)) }, m(11)), false, 'under a hint question');
});

const startOf3 = (o: Record<string, unknown>) => ({ event: 'start', data: { chatId: 11, chatTitle: 'T', model: 'demo-chat-small', ...o } });

test('start of a regenerate: the old answer and its tail are cut, the new version follows the question', () => {
  const s = localRegenerate(tree(), 11);
  assert.equal(s.messages.length, 1, 'optimistic cut: only the first question is left');
  const r = applyEvent(s, startOf3({ mode: 'regenerate', userMessageId: 10, assistantMessageId: 20, parentId: 10, versions: [11, 20], connection: 'main' }), NOW);
  assert.deepEqual(r.messages.map((m) => m.id), [10, 20]);
  assert.deepEqual(r.messages[1]?.versions, [11, 20]);
  assert.equal(r.messages[1]?.parentId, 10);
  assert.equal(r.messages[1]?.connection, 'main');
  assert.equal(r.streamingId, 20);
  assert.equal(r.pending, null);
  // without the optimistic cut (a poll came in meanwhile) the frame cuts as well
  const r2 = applyEvent(tree(), startOf3({ mode: 'regenerate', userMessageId: 10, assistantMessageId: 20, parentId: 10, versions: [11, 20] }), NOW);
  assert.deepEqual(r2.messages.map((m) => m.id), [10, 20]);
  // a regenerate never clears the composer
  assert.equal(clearsComposer(s, startOf3({ mode: 'regenerate', userMessageId: 10, assistantMessageId: 20, parentId: 10 })), false);
});

test('start of an edit: the question and its tail are replaced by the new question and answer', () => {
  const s = localEdit(tree(), 12, 'q2 edited');
  assert.deepEqual(s.messages.map((m) => m.id), [10, 11, TEMP_ID]);
  assert.equal(s.messages[2]?.content, 'q2 edited');
  const r = applyEvent(s, startOf3({ mode: 'edit', userMessageId: 40, assistantMessageId: 41, userParentId: 11, parentId: 40, userVersions: [12, 40] }), NOW);
  assert.deepEqual(r.messages.map((m) => m.id), [10, 11, 40, 41]);
  assert.equal(r.messages[2]?.content, 'q2 edited');
  assert.deepEqual(r.messages[2]?.versions, [12, 40]);
  assert.equal(r.messages[2]?.parentId, 11);
  assert.equal(r.messages[3]?.parentId, 40);
  assert.equal(clearsComposer(s, startOf3({ mode: 'edit', userMessageId: 40, assistantMessageId: 41, userParentId: 11 })), true);
  // editing the first message: two roots, the path starts again
  const root = applyEvent(localEdit(tree(), 10, 'q1 edited'), startOf3({ mode: 'edit', userMessageId: 50, assistantMessageId: 51, userParentId: null, versions: [], userVersions: [10, 30, 50] }), NOW);
  assert.deepEqual(root.messages.map((m) => m.id), [50, 51]);
  assert.equal(root.messages[0]?.parentId, null);
});

test('a regenerate or an edit that fails before it starts restores the list', () => {
  const t0 = tree();
  assert.deepEqual(localFail(localRegenerate(t0, 13)).messages, t0.messages);
  assert.deepEqual(localFail(localEdit(t0, 10, 'x')).messages, t0.messages);
  assert.equal(localFail(localEdit(t0, 10, 'x')).pending, null);
  // a plain send has nothing to restore
  assert.deepEqual(localFail(localSend(t0, 'x')).messages, t0.messages);
});

test('a send after a switch appends under the last message of the shown branch', () => {
  const r = applyEvent(localSend(tree(), 'next'), startOf3({ userMessageId: 60, assistantMessageId: 61 }), NOW);
  assert.equal(r.messages[4]?.parentId, 13);
  assert.equal(r.messages[5]?.parentId, 60);
});

test('a poll during a regenerate keeps the pending request and takes the server path', () => {
  const cut = localRegenerate(tree(), 11);
  const polled = pollResult(cut, detail(true, [
    msg({ id: 10, role: 'user', parentId: null }),
    msg({ id: 20, role: 'assistant', parentId: 10, status: 'streaming', versions: [11, 20] }),
  ]));
  assert.equal(polled.streamingId, 20);
  assert.equal(polled.pending?.retry, true);
  assert.deepEqual(polled.messages.map((m) => m.id), [10, 20]);
});

test('beginEdit / editValid: the draft is kept, and the edit ends when its question left the path', () => {
  const s = tree();
  const e = beginEdit(s, s.messages[2] as ViewMessage, 'half typed');
  assert.deepEqual(e, { messageId: 12, draft: 'half typed' });
  assert.equal(beginEdit(s, s.messages[3] as ViewMessage, ''), null, 'an answer is not edited');
  assert.equal(beginEdit({ ...s, running: true }, s.messages[2] as ViewMessage, ''), null);
  assert.equal(editValid(s, e as NonNullable<typeof e>), true);
  const moved = chatOf(detail(false, [msg({ id: 10, role: 'user', parentId: null }), msg({ id: 20, role: 'assistant', parentId: 10 })]));
  assert.equal(editValid(moved, e as NonNullable<typeof e>), false);
});

const models: AiModelList = {
  connections: [
    { id: 1, name: 'main', provider: 'openai', status: 'ok', listed: true, error: null, models: [
      { id: 'demo-chat-small', context: null, poolId: 1, status: 'ok' }, { id: 'demo-chat-large', context: null, poolId: null, status: null },
      { id: 'demo-bad', context: null, poolId: 3, status: 'failed' }] },
    { id: 2, name: 'spare', provider: 'anthropic', status: 'broken', listed: false, error: 'ai.up.auth', models: [{ id: 'demo-x', context: null, poolId: null, status: null }] },
    { id: 3, name: 'edge', provider: 'gateway', status: 'ok', listed: true, error: null, models: [
      { id: 'demo:colon', context: null, poolId: 7, status: 'untested' }, { id: 'demo-listed-only', context: null, poolId: null, status: null }] },
    { id: 4, name: 'empty', provider: 'openai', status: 'ok', listed: true, error: null, models: [{ id: 'demo-only-bad', context: null, poolId: 9, status: 'failed' }] },
    { id: 5, name: 'garden', provider: 'openai', status: 'ok', enabled: false, listed: false, error: null, models: [] },
  ],
};
const DEF: ChatDefault = { connectionId: 1, connectionName: 'main', model: 'demo-chat-small' };

test('chatMenu: working enabled connections with usable models, in the listing order; failed models, broken, off and empty connections are left out; the default is marked', () => {
  const m = chatMenu(models, null, DEF);
  assert.equal(m.loaded, true);
  assert.equal(m.off, null);
  assert.deepEqual(m.conns.map((g) => [g.name, g.models.map((o) => o.label)]), [['main', ['demo-chat-small', 'demo-chat-large']], ['edge', ['demo:colon', 'demo-listed-only']]]);
  assert.deepEqual(m.conns[1]?.models[0], { value: '3:demo:colon', label: 'demo:colon', isDefault: false }, 'a model id with a colon stays whole');
  assert.deepEqual(m.conns[0]?.models.map((o) => o.isDefault), [true, false]);
  assert.equal(chatMenu(models, null, null).conns[0]?.models[0]?.isDefault, false);
  // an unlisted pick on a working connection comes first in its models; a listed pick is not repeated
  const bad = chatMenu(models, { connectionId: 1, model: 'demo-bad' }, DEF);
  assert.deepEqual(bad.conns[0]?.models.map((o) => o.label), ['demo-bad', 'demo-chat-small', 'demo-chat-large']);
  assert.equal(chatMenu(models, { connectionId: 1, model: 'demo-unlisted' }, DEF).conns[0]?.models[0]?.value, '1:demo-unlisted');
  assert.equal(chatMenu(models, { connectionId: 3, model: 'demo:colon' }, DEF).conns[1]?.models.length, 2);
  assert.equal(chatMenu(models, { connectionId: 1, model: 'demo-chat-small' }, DEF).conns[0]?.models.length, 2);
  // a pick on an off connection is reported with its name; a pick on a broken or missing connection is neither listed nor off
  assert.deepEqual(chatMenu(models, { connectionId: 5, model: 'demo-x' }, DEF).off, { name: 'garden' });
  for (const pick of [{ connectionId: 2, model: 'demo-x' }, { connectionId: 9, model: 'gone' }]) {
    assert.deepEqual(chatMenu(models, pick, DEF), chatMenu(models, null, DEF), JSON.stringify(pick));
  }
  // a connection without the `enabled` field counts as on
  const bare: AiModelList = { connections: [{ id: 4, name: 'bare', provider: 'openai', status: 'ok', listed: true, error: null, models: [{ id: 'demo-typed', context: null, poolId: null, status: null }] }] };
  assert.equal(chatMenu(bare, null, null).conns.length, 1);
  assert.deepEqual(chatMenu({ connections: [] }, { connectionId: 4, model: 'demo-typed' }, null), { loaded: true, conns: [], off: null });
  // not loaded: nothing offered, nothing off
  assert.deepEqual(chatMenu(null, { connectionId: 9, model: 'gone' }, DEF), { loaded: false, conns: [], off: null });
});

test('chatMenu: a gateway that lists its whole catalogue shows all of it under each connection, minus the failed models', () => {
  const catalogue = Array.from({ length: 40 }, (_, i) => ({ id: `demo-catalogue-${i}`, context: null, poolId: null, status: null }));
  const list: AiModelList = {
    connections: [
      { id: 5, name: 'gate', provider: 'gateway', status: 'ok', listed: true, error: null, models: [
        ...catalogue.slice(0, 20), { id: 'demo-added', context: 64000, poolId: 12, status: 'ok' }, ...catalogue.slice(20)] },
      { id: 6, name: 'other', provider: 'gateway', status: 'ok', listed: true, error: null, models: [
        ...catalogue, { id: 'demo-added', context: null, poolId: null, status: null }, { id: 'demo-other', context: null, poolId: 13, status: 'ok' },
        { id: 'demo-failed', context: null, poolId: 14, status: 'failed' }] },
    ],
  };
  const m = chatMenu(list, null, null);
  const ids = catalogue.map((c) => c.id);
  assert.deepEqual(m.conns.map((g) => [g.id, g.name, g.models.map((o) => o.label)]), [
    [5, 'gate', [...ids.slice(0, 20), 'demo-added', ...ids.slice(20)]],
    [6, 'other', [...ids, 'demo-added', 'demo-other']],
  ]);
  assert.equal(m.conns[0]?.models.length, 41);
  assert.equal(m.conns[1]?.models.length, 42);
  assert.deepEqual(m.conns[1]?.models[0], { value: '6:demo-catalogue-0', label: 'demo-catalogue-0', isDefault: false });
});

test('modelArg: nothing changed sends no model, the default clears the pick, a model is sent as a reference', () => {
  const stored = { connectionId: 1, model: 'demo-chat-small' };
  assert.equal(modelArg('1:demo-chat-small', stored, parseModelRef), undefined);
  assert.equal(modelArg('', null, parseModelRef), undefined);
  assert.equal(modelArg('', stored, parseModelRef), null);
  assert.deepEqual(modelArg('3:demo:colon', stored, parseModelRef), { connectionId: 3, model: 'demo:colon' });
  assert.equal(modelArg('garbage', null, parseModelRef), undefined);
});

test('chatShown: a pick the menu does not offer (broken or removed connection) shows as the default and is still sent as unchanged', () => {
  for (const pick of [{ connectionId: 2, model: 'demo-x' }, { connectionId: 9, model: 'gone' }]) {
    const text = `${pick.connectionId}:${pick.model}`;
    assert.equal(chatShown(text, chatMenu(models, pick, DEF)), '', text);
    // the picker shows the default, yet nothing was changed: the server falls back and clears the pick itself
    assert.equal(modelArg(text, pick, parseModelRef), undefined, text);
  }
  // offered values are shown as they are: the default, a listed model, the unlisted pick, an unsent change
  const bad = { connectionId: 1, model: 'demo-bad' };
  assert.equal(chatShown('', chatMenu(models, bad, DEF)), '');
  assert.equal(chatShown('1:demo-bad', chatMenu(models, bad, DEF)), '1:demo-bad');
  assert.equal(chatShown('3:demo:colon', chatMenu(models, bad, DEF)), '3:demo:colon');
  // a pick on an off connection is kept (the call fails with ai.connectionOff; nothing is dropped silently)
  const off = { connectionId: 5, model: 'demo-x' };
  assert.equal(chatShown('5:demo-x', chatMenu(models, off, DEF)), '5:demo-x');
  // the list is not loaded yet: the pick is kept
  assert.equal(chatShown('9:gone', chatMenu(null, { connectionId: 9, model: 'gone' }, DEF)), '9:gone');
});

test('pickerView: pending connection first, then the chosen model, then the default while its connection is listed', () => {
  const menu = chatMenu(models, null, DEF);
  assert.deepEqual(pickerView('1:demo-chat-large', 3, menu, DEF), { connId: 3, modelValue: '' }, 'only the API was changed: no model yet');
  assert.deepEqual(pickerView('3:demo:colon', null, menu, DEF), { connId: 3, modelValue: '3:demo:colon' });
  assert.deepEqual(pickerView('', null, menu, DEF), { connId: 1, modelValue: '1:demo-chat-small' }, 'the default is preselected');
  assert.deepEqual(pickerView('', null, menu, null), { connId: null, modelValue: '' });
  // the default's connection is not listed (off or broken): placeholders
  const gone = chatMenu({ connections: models.connections.filter((c) => c.id !== 1) }, null, DEF);
  assert.deepEqual(pickerView('', null, gone, DEF), { connId: null, modelValue: '' });
  // a pick on an off connection shows that connection and model
  assert.deepEqual(pickerView('5:demo-x', null, chatMenu(models, { connectionId: 5, model: 'demo-x' }, DEF), DEF), { connId: 5, modelValue: '5:demo-x' });
  assert.deepEqual(pickerView('garbage', null, menu, DEF), { connId: null, modelValue: '' });
});

test('chooseValue: the model marked default means "follow the default"; any other model is kept as chosen', () => {
  assert.equal(chooseValue('1:demo-chat-small', DEF), '');
  assert.equal(chooseValue('1:demo-chat-large', DEF), '1:demo-chat-large');
  assert.equal(chooseValue('3:demo:colon', DEF), '3:demo:colon');
  assert.equal(chooseValue('1:demo-chat-small', null), '1:demo-chat-small');
  // choosing the default after a custom pick clears the stored pick (null in the request)
  assert.equal(modelArg(chooseValue('1:demo-chat-small', DEF), { connectionId: 3, model: 'demo:colon' }, parseModelRef), null);
});

test('the pick: a send keeps the model it carried as the chat pick; a fallback frame clears it; the body carries it only when set', () => {
  const pick = { connectionId: 1, model: 'demo-chat-large' };
  const sent = applyEvent(localSend(emptyChat(), 'hi', false, pick), startOf3({ userMessageId: 1, assistantMessageId: 2 }), NOW);
  assert.deepEqual(sent.pick, pick);
  const fell = applyEvent(localSend({ ...emptyChat(), pick }, 'hi'), startOf3({ userMessageId: 1, assistantMessageId: 2, modelFallback: true }), NOW);
  assert.equal(fell.pick, null);
  const fellEff = applyEvent(localSend({ ...emptyChat(), pick, pickEffort: 'high' }, 'hi'), startOf3({ userMessageId: 1, assistantMessageId: 2, modelFallback: true }), NOW);
  assert.equal(fellEff.pickEffort, 'high', 'the server keeps the stored level on a fallback');
  const kept = applyEvent(localSend({ ...emptyChat(), pick }, 'hi'), startOf3({ userMessageId: 1, assistantMessageId: 2 }), NOW);
  assert.deepEqual(kept.pick, pick);
  assert.deepEqual(sendBody({ chatId: null, text: 't', lang: 'en' }), { chatId: null, text: 't', lang: 'en' });
  assert.deepEqual(sendBody({ chatId: 1, text: 't', lang: 'en', model: null }), { chatId: 1, text: 't', lang: 'en', model: null });
  assert.deepEqual(chatOf({ chat: { ...detail(false, []).chat, pick }, messages: [] }).pick, pick);
  assert.equal(chatOf(detail(false, [])).pick, null);
});

// ---------------------------------------------------------------- v2.1: the strength step

const LEVELS = {
  anthropic: ['low', 'medium', 'high', 'max'], openai: ['none', 'low', 'medium', 'high'],
  google: ['MINIMAL', 'LOW', 'MEDIUM', 'HIGH'], responses: ['low', 'medium', 'high'],
};
const SET = {
  connections: [{ id: 1, effortFamily: 'anthropic' }, { id: 2, effortFamily: 'google' }],
  pool: [{ id: 10, connectionId: 1, model: 'demo-a' }, { id: 11, connectionId: 2, model: 'demo-g' }],
  slots: { default: { modelId: 10 }, chat: { modelId: null, effort: { family: 'anthropic', level: 'low' }, effortEffective: { family: 'anthropic', level: 'low' } } },
  effortLevels: LEVELS,
} as unknown as AiSettings;

test('strengthView: own pick, else the chat row on its own family, else Default (no model default any more)', () => {
  assert.deepEqual(slotRoute(SET, 'chat'), { connectionId: 1, model: 'demo-a' });
  assert.equal(strengthView(SET, 1, 'demo-a', 'high')?.selected, 'high', 'a valid pick wins');
  assert.equal(strengthView(SET, 1, 'demo-a', '')?.selected, 'low', 'the chat row level is shown');
  assert.equal(strengthView(SET, 1, 'demo-b', '')?.selected, 'low', 'another model of the family: the row level, as the server sends it');
  const none = { ...SET, slots: { ...SET.slots, chat: { modelId: null, effort: null } } } as unknown as AiSettings;
  assert.equal(strengthView(none, 1, 'claude-sonnet-5-5', '')?.selected, 'default', 'nothing stored: Default, whatever the model');
  assert.equal(strengthView(SET, 1, 'demo-a', 'default')?.selected, 'default', 'the own Default beats the row level');
  assert.deepEqual(strengthView(SET, 1, 'demo-a', '')?.options, ['default', 'low', 'medium', 'high', 'max'], 'Default first, then the levels');
  assert.equal(strengthView(SET, 2, 'demo-g', 'max')?.selected, 'default', 'a level of another family is ignored; the row is another family too');
  assert.equal(strengthView(SET, 2, 'demo-g', 'HIGH')?.selected, 'HIGH');
  assert.equal(strengthView(SET, 2, 'demo-g', 'high')?.family, 'google');
  assert.equal(strengthView(SET, 9, 'x', ''), null);
  assert.equal(strengthView(null, 1, 'x', ''), null);
});

test('effortArg / keepEffort', () => {
  assert.equal(effortArg('high', 'high'), undefined);
  assert.equal(effortArg('', null), undefined);
  assert.equal(effortArg('', 'high'), null, 'an explicit reset');
  assert.equal(effortArg('low', null), 'low');
  assert.equal(effortArg('default', null), 'default', 'picking Default is sent as "default"');
  assert.equal(effortArg('default', 'default'), undefined);
  assert.equal(effortArg('default', 'high'), 'default');
  assert.equal(effortArg('high', 'default'), 'high');
  const v = strengthView(SET, 1, 'demo-b', '');
  assert.equal(keepEffort('high', v), 'high');
  assert.equal(keepEffort('HIGH', v), '', 'levels are matched exactly, like the server does');
  assert.equal(keepEffort('none', v), '');
  assert.equal(keepEffort('default', v), 'default', 'Default is kept for every family');
  assert.equal(keepEffort('default', null), 'default');
  assert.equal(keepEffort('high', null), '');
});

test('use default (brain popover): a stale level of another family is dropped, so no 400 is sent', () => {
  // Anthropic 'high' picked, then back to the Google default route
  const google = strengthView(SET, 2, 'demo-g', 'high');
  const kept = keepEffort('high', google);
  assert.equal(kept, '');
  assert.equal(effortArg(kept, null), undefined, 'nothing is sent');
  assert.equal(effortArg(kept, 'high'), null, 'a stored own choice is reset, never a wrong level');
  assert.equal(keepEffort('default', google), 'default');
});

test('follow AI settings: selected "" shows the inherited level and sends null only when a choice was stored', () => {
  assert.equal(strengthView(SET, 1, 'demo-a', '')?.selected, 'low');
  assert.equal(effortArg('', 'high'), null);
  assert.equal(effortArg('', null), undefined, 'clicking the inherited item while following changes nothing');
});

test('sendBody carries effort only when given; the pick is kept at the start event', () => {
  assert.deepEqual(sendBody({ chatId: 1, text: 'a', lang: 'en' }), { chatId: 1, text: 'a', lang: 'en' });
  assert.deepEqual(sendBody({ chatId: 1, text: 'a', lang: 'en', effort: null }), { chatId: 1, text: 'a', lang: 'en', effort: null });
  const s = localSend(emptyChat(), 'a', false, undefined, 'high');
  assert.equal(s.pending?.effort, 'high');
  assert.equal(s.pending?.model, undefined);
});

test('groupChats: local calendar days, newest first, empty groups left out', () => {
  const now = new Date(2026, 10, 21, 0, 30); // 21 Nov, just after midnight
  const at = (daysAgo: number, h = 12) => Math.floor(new Date(2026, 10, 21 - daysAgo, h).getTime() / 1000);
  const chats = [
    { id: 1, updatedAt: at(10) }, { id: 2, updatedAt: at(0, 0) }, { id: 3, updatedAt: at(1, 23) }, { id: 4, updatedAt: at(7) },
    { id: 5, updatedAt: at(8, 23) }, { id: 6, updatedAt: at(2) }, { id: 7, updatedAt: at(1, 1) },
  ];
  const g = groupChats(chats, now);
  assert.deepEqual(g.map((x) => x.group), ['today', 'yesterday', 'week', 'older']);
  assert.deepEqual(g[0]?.chats.map((c) => c.id), [2]);
  assert.deepEqual(g[1]?.chats.map((c) => c.id), [3, 7], 'newest first');
  assert.deepEqual(g[2]?.chats.map((c) => c.id), [6, 4]);
  assert.deepEqual(g[3]?.chats.map((c) => c.id), [5, 1]);
  assert.deepEqual(groupChats([{ updatedAt: at(10) }], now).map((x) => x.group), ['older'], 'empty groups are left out');
  assert.deepEqual(groupChats([], now), []);
  const iso = groupChats([{ updatedAt: new Date(2026, 10, 21, 0, 10).toISOString() }], now);
  assert.equal(iso[0]?.group, 'today', 'a date text works too');
});

test('filterChats: trimmed, case-insensitive, empty = all', () => {
  const list = [{ t: 'Rust Lifetimes' }, { t: '练习 一' }, { t: 'other' }];
  const title = (c: { t: string }) => c.t;
  assert.equal(filterChats(list, '', title), list);
  assert.equal(filterChats(list, '   ', title), list);
  assert.deepEqual(filterChats(list, '  LIFE ', title).map(title), ['Rust Lifetimes']);
  assert.deepEqual(filterChats(list, '练习', title).map(title), ['练习 一']);
  assert.deepEqual(filterChats(list, 'zzz', title), []);
});

test('entryTarget: the address wins, then a running chat, else a new chat', () => {
  assert.deepEqual(entryTarget({ running: 5, hash: 9 }), { kind: 'chat', id: 9 });
  assert.deepEqual(entryTarget({ running: 5, hash: 'none' }), { kind: 'chat', id: 5 });
  assert.deepEqual(entryTarget({ running: 5, hash: 'new' }), { kind: 'chat', id: 5 });
  assert.deepEqual(entryTarget({ running: null, hash: 'none' }), { kind: 'new' });
  assert.deepEqual(entryTarget({ running: null, hash: 'new' }), { kind: 'new' });
});

test('drawerBack: back closes an open drawer, else leaves', () => {
  assert.equal(drawerBack({ drawerOpen: true }), 'close-drawer');
  assert.equal(drawerBack({ drawerOpen: false }), 'leave');
});

test('localDayKey: one number per local calendar day, consecutive days differ by 1', () => {
  const a = new Date(2026, 5, 10, 0, 5);
  const b = new Date(2026, 5, 10, 23, 55);
  const c = new Date(2026, 5, 11, 0, 5);
  assert.equal(localDayKey(a), localDayKey(b));
  assert.equal(localDayKey(c), localDayKey(a) + 1);
});

test('latestChat: the newest chat by last activity, null for none', () => {
  assert.equal(latestChat([]), null);
  const list = [{ id: 1, updatedAt: 1_700_000_000 }, { id: 2, updatedAt: 1_700_050_000 }, { id: 3, updatedAt: 1_699_000_000 }];
  assert.equal(latestChat(list)?.id, 2);
});

test('chatAddress: a chat, a new draft, and no draft address for a user who cannot write', () => {
  assert.equal(chatAddress(7, true), '#/ai/7');
  assert.equal(chatAddress(7, false), '#/ai/7');
  assert.equal(chatAddress('new', true), '#/ai/new');
  assert.equal(chatAddress('new', false), '#/ai');
  assert.equal(chatAddress(null, true), '#/ai');
});

test('the drawer entry marker: only the drawer own state counts; a closed drawer reached by Forward is stale', () => {
  assert.equal(isDrawerEntry(DRAWER_ENTRY), true);
  assert.equal(isDrawerEntry(null), false);
  assert.equal(isDrawerEntry({}), false);
  assert.equal(isDrawerEntry('x'), false);
  assert.equal(staleDrawerEntry({ drawerOpen: false, ownMove: false, state: DRAWER_ENTRY }), true);
  assert.equal(staleDrawerEntry({ drawerOpen: true, ownMove: false, state: DRAWER_ENTRY }), false);
  assert.equal(staleDrawerEntry({ drawerOpen: false, ownMove: true, state: DRAWER_ENTRY }), false);
  assert.equal(staleDrawerEntry({ drawerOpen: false, ownMove: false, state: null }), false);
});

test('composerLayout: compact for one fitting line, expanded for a newline or a wrapping text', () => {
  assert.equal(composerLayout('', 44, 24, 20), 'compact');
  assert.equal(composerLayout('short', 44, 24, 20), 'compact');
  assert.equal(composerLayout('a\nb', 44, 24, 20), 'expanded');
  assert.equal(composerLayout('a\r\nb', 44, 24, 20), 'expanded');
  assert.equal(composerLayout('x'.repeat(200), 68, 24, 20), 'expanded');
  assert.equal(composerLayout('x', 56, 24, 20), 'compact');
});
