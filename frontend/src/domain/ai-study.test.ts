import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  type Action, type Answer, type Entry, answerOf, boundedSet, emptyEntry, entryView, isAskError, isNothingToSummarise, isOtherLang,
  MAX_ENTRIES, parseSaved, reduce, refreshesStatus, runningOf, sessionPart, studyKey, summaryKey,
} from './ai-study.ts';

const run = (e: Entry, ...actions: Action[]): Entry => actions.reduce(reduce, e);
const ev = (event: string, data: unknown = {}): Action => ({ type: 'event', ev: { event, data }, now: 1000 });
const answer = (o: Partial<Answer> = {}): Answer => ({
  messageId: 1, status: 'done', error: null, truncated: false, content: 'good', lang: 'en', createdAt: 1, finishedAt: 2, sample: null, ...o,
});
const START = ev('start', { chatId: 7, assistantMessageId: 21, lang: 'en' });

describe('stream reducer', () => {
  it('ask -> start -> delta... -> done', () => {
    let e = run(emptyEntry(), { type: 'ask' });
    assert.equal(entryView(e, 'en').phase, 'requesting');
    assert.equal(entryView(e, 'en').canStop, true);
    e = run(e, START, ev('delta', { text: 'Hel' }), ev('delta', { text: 'lo' }));
    assert.equal(e.chatId, 7);
    let v = entryView(e, 'en');
    assert.deepEqual([v.phase, v.text, v.canStop, v.canRegenerate], ['streaming', 'Hello', true, false]);
    e = run(e, ev('done', { truncated: true }));
    v = entryView(e, 'en');
    assert.deepEqual([v.phase, v.text, v.canStop, v.canRegenerate, v.truncated, v.savedAt], ['done', 'Hello', false, true, true, 1000]);
    assert.equal(e.shown?.content, 'Hello');
  });
  it('an empty delta and a delta without a running attempt change nothing', () => {
    const e = emptyEntry();
    assert.equal(run(e, ev('delta', { text: 'x' })), e);
    const s = run(e, START);
    assert.equal(run(s, ev('delta', { text: '' })), s);
  });
  it('stopped keeps the partial text when nothing good exists', () => {
    const e = run(emptyEntry(), { type: 'ask' }, START, ev('delta', { text: 'half' }), ev('stopped'));
    const v = entryView(e, 'en');
    assert.deepEqual([v.phase, v.text, v.canRegenerate], ['stopped', 'half', true]);
    assert.equal(e.shown, null);
  });
  it('an error without a previous answer shows the partial text and the error note', () => {
    const e = run(emptyEntry(), { type: 'ask' }, START, ev('delta', { text: 'part' }), ev('error', { code: 'ai.up.timeout' }));
    const v = entryView(e, 'en');
    assert.equal(v.phase, 'error');
    assert.deepEqual(v.note, { kind: 'error', code: 'ai.up.timeout', status: 0 });
    assert.equal(v.text, 'part');
  });
  it('withdraw clears the text at once and a withheld hint with nothing before it shows the withheld state', () => {
    let e = run(emptyEntry(), { type: 'ask' }, START, ev('delta', { text: 'The answer is B' }));
    e = run(e, ev('error', { code: 'ai.hintLeak', withdraw: true }));
    const v = entryView(e, 'en');
    assert.deepEqual([v.phase, v.text, v.note, v.canRegenerate], ['withheld', '', { kind: 'withheld' }, true]);
  });
  it('withdraw also clears the text for another code', () => {
    const e = run(emptyEntry(), { type: 'ask' }, START, ev('delta', { text: 'x' }), ev('error', { code: 'ai.internal', withdraw: true }));
    assert.equal(e.attempt?.content, '');
  });
  it('an error event without a code is ai.internal', () => {
    const e = run(emptyEntry(), { type: 'ask' }, START, ev('error', {}));
    assert.equal(e.attempt?.error, 'ai.internal');
  });
});

describe('a failed regenerate keeps the last good answer', () => {
  const good = (): Entry => run(emptyEntry(), { type: 'saved', saved: { chatId: 7, latest: answer(), lastGood: null } });

  it('a done regenerate replaces what is shown', () => {
    let e = run(good(), { type: 'ask' }, START);
    assert.equal(entryView(e, 'en').text, '', 'the new text replaces the shown text while it streams');
    e = run(e, ev('delta', { text: 'new' }), ev('done'));
    assert.equal(entryView(e, 'en').text, 'new');
    assert.equal(e.shown?.content, 'new');
  });
  it('a failed regenerate shows the previous answer under the failed-previous note', () => {
    const e = run(good(), { type: 'ask' }, START, ev('delta', { text: 'bad' }), ev('error', { code: 'ai.up.timeout' }));
    const v = entryView(e, 'en');
    assert.deepEqual([v.text, v.phase, v.note], ['good', 'done', { kind: 'failedPrev', reason: 'error', code: 'ai.up.timeout' }]);
  });
  it('a stopped regenerate keeps it too', () => {
    const e = run(good(), { type: 'ask' }, START, ev('delta', { text: 'x' }), { type: 'localStop' });
    const v = entryView(e, 'en');
    assert.deepEqual([v.text, v.note], ['good', { kind: 'failedPrev', reason: 'stopped', code: null }]);
  });
  it('a withheld regenerate keeps it and says so', () => {
    const e = run(good(), { type: 'ask' }, START, ev('delta', { text: 'leak' }), ev('error', { code: 'ai.hintLeak', withdraw: true }));
    const v = entryView(e, 'en');
    assert.deepEqual([v.text, v.note], ['good', { kind: 'withheldPrev' }]);
  });
  it('a request refused before any stream (not allowed) keeps it under the error note', () => {
    const e = run(good(), { type: 'ask' }, { type: 'askFailed', error: { status: 403, code: 'ai.suspended' } });
    const v = entryView(e, 'en');
    assert.deepEqual([v.text, v.phase, v.note, v.canRegenerate], ['good', 'done', { kind: 'error', code: 'ai.suspended', status: 403 }, true]);
  });
  it('the next ask clears the failure', () => {
    const e = run(good(), { type: 'askFailed', error: { status: 0, code: null } }, { type: 'ask' });
    assert.equal(e.failure, null);
  });
});

describe('saved answers', () => {
  it('nothing saved: an empty entry that remembers the chat', () => {
    const e = run(emptyEntry(), { type: 'loading' }, { type: 'saved', saved: { chatId: 3, latest: null, lastGood: null } });
    assert.equal(e.chatId, 3);
    assert.equal(entryView(e, 'en').phase, 'empty');
  });
  it('the chat title (the server title) is remembered from the saved route and from the start event', () => {
    assert.equal(parseSaved({ chatId: 3, chatTitle: '2026-11-12 09:30', latest: null, lastGood: null }).chatTitle, '2026-11-12 09:30');
    const saved = run(emptyEntry(), { type: 'saved', saved: { chatId: 3, chatTitle: '2026-11-12 09:30-练习', latest: answer(), lastGood: null } });
    assert.equal(saved.chatTitle, '2026-11-12 09:30-练习');
    const started = run(emptyEntry(), { type: 'ask' }, { type: 'event', ev: { event: 'start', data: { chatId: 7, chatTitle: 'T1', assistantMessageId: 2 } }, now: 1 });
    assert.equal(started.chatTitle, 'T1');
    // A later saved answer without a title keeps the known one; no chat at all has no title
    assert.equal(run(started, { type: 'saved', saved: { chatId: 7, latest: answer(), lastGood: null } }).chatTitle, 'T1');
    assert.equal(run(started, { type: 'saved', saved: { chatId: null, latest: null, lastGood: null } }).chatTitle, null);
    // Another chat without a title: the old title is not carried over to it
    assert.equal(run(started, { type: 'saved', saved: { chatId: 8, latest: answer(), lastGood: null } }).chatTitle, null);
    assert.equal(run(started, { type: 'event', ev: { event: 'start', data: { chatId: 8, assistantMessageId: 3 } }, now: 2 }).chatTitle, null);
    assert.equal(run(started, { type: 'event', ev: { event: 'start', data: { chatId: 7, assistantMessageId: 3 } }, now: 2 }).chatTitle, 'T1');
  });
  it('latest done is shown at once', () => {
    const e = run(emptyEntry(), { type: 'saved', saved: { chatId: 3, latest: answer({ content: 'saved text' }), lastGood: null } });
    const v = entryView(e, 'en');
    assert.deepEqual([v.phase, v.text, v.savedAt, e.polling], ['done', 'saved text', 2, false]);
  });
  it('latest streaming: its text, polling on, no Regenerate, Stop allowed (the chat is known)', () => {
    const e = run(emptyEntry(), { type: 'saved', saved: { chatId: 3, latest: answer({ status: 'streaming', content: 'so far', finishedAt: null }), lastGood: null } });
    const v = entryView(e, 'en');
    assert.deepEqual([v.phase, v.text, v.canRegenerate, v.canStop, e.polling], ['streaming', 'so far', false, true, true]);
    // the poll sees it finish
    const done = run(e, { type: 'saved', saved: { chatId: 3, latest: answer({ content: 'so far and more' }), lastGood: null } });
    assert.deepEqual([entryView(done, 'en').phase, done.polling, done.shown?.content], ['done', false, 'so far and more']);
  });
  it('latest failed / stopped / withheld with a lastGood: the good one under the matching note', () => {
    const g = answer({ content: 'older' });
    let e = run(emptyEntry(), { type: 'saved', saved: { chatId: 3, latest: answer({ status: 'error', error: 'ai.up.down', content: '' }), lastGood: g } });
    assert.deepEqual(entryView(e, 'en').note, { kind: 'failedPrev', reason: 'error', code: 'ai.up.down' });
    e = run(emptyEntry(), { type: 'saved', saved: { chatId: 3, latest: answer({ status: 'stopped', content: 'p' }), lastGood: g } });
    assert.deepEqual(entryView(e, 'en').note, { kind: 'failedPrev', reason: 'stopped', code: null });
    e = run(emptyEntry(), { type: 'saved', saved: { chatId: 3, latest: answer({ status: 'error', error: 'ai.hintLeak', content: '' }), lastGood: g } });
    assert.deepEqual(entryView(e, 'en').note, { kind: 'withheldPrev' });
    assert.equal(entryView(e, 'en').text, 'older');
  });
  it('latest withheld with nothing good: the withheld state', () => {
    const e = run(emptyEntry(), { type: 'saved', saved: { chatId: 3, latest: answer({ status: 'error', error: 'ai.hintLeak', content: '' }), lastGood: null } });
    assert.equal(entryView(e, 'en').phase, 'withheld');
  });
  it('a lost stream turns into polling while the attempt is streaming', () => {
    const e = run(emptyEntry(), { type: 'ask' }, START, ev('delta', { text: 'a' }), { type: 'lost' });
    assert.equal(e.polling, true);
    assert.equal(entryView(e, 'en').text, 'a');
  });
  it('a loading entry without any answer paints the loading phase', () => {
    assert.equal(entryView(run(emptyEntry(), { type: 'loading' }), 'en').phase, 'loading');
  });
});

describe('busy and request failures', () => {
  it('a 409 busy shows the busy note, no Regenerate, and keeps the shown answer', () => {
    const running = runningOf({ chatId: 2, messageId: 8, chatKind: 'practice', kind: 'hint', questionId: 'q9', questionNo: 3, source: 'practice', recordId: null });
    const e = run(
      run(emptyEntry(), { type: 'saved', saved: { chatId: 7, latest: answer(), lastGood: null } }),
      { type: 'ask' }, { type: 'askFailed', error: { status: 409, code: 'ai.busy', running } },
    );
    const v = entryView(e, 'en');
    assert.deepEqual(v.note, { kind: 'busy', running });
    assert.equal(v.canRegenerate, false);
    assert.equal(v.text, 'good');
  });
  it('a busy answer whose runner is unknown still offers Regenerate (nothing can be stopped)', () => {
    const e = run(emptyEntry(), { type: 'ask' }, { type: 'askFailed', error: { status: 409, code: 'ai.busy', running: null } });
    assert.equal(entryView(e, 'en').canRegenerate, true);
  });
  it('a second busy answer sets the note again', () => {
    const f = { type: 'askFailed', error: { status: 409, code: 'ai.busy', running: null } } as const;
    const e = run(emptyEntry(), { type: 'ask' }, f, { type: 'ask' }, f);
    assert.deepEqual(entryView(e, 'en').note, { kind: 'busy', running: null });
  });
  it('a request error with nothing shown is the error phase with Regenerate', () => {
    const e = run(emptyEntry(), { type: 'ask' }, { type: 'askFailed', error: { status: 0, code: null } });
    const v = entryView(e, 'en');
    assert.deepEqual([v.phase, v.canRegenerate], ['error', true]);
  });
  it('refreshesStatus', () => {
    for (const code of ['ai.disabled', 'ai.notAllowed', 'ai.suspended', 'ai.unavailable']) assert.equal(refreshesStatus({ status: 403, code }), true);
    assert.equal(refreshesStatus({ status: 503, code: 'ai.backendDown' }), true);
    assert.equal(refreshesStatus({ status: 502, code: 'ai.backendDown' }), false);
    assert.equal(refreshesStatus({ status: 409, code: 'ai.busy' }), false);
    assert.equal(refreshesStatus({ status: 0, code: null }), false);
  });
  it('isNothingToSummarise', () => {
    assert.equal(isNothingToSummarise('ai.noWrong'), true);
    assert.equal(isNothingToSummarise('ai.noUsable'), true);
    assert.equal(isNothingToSummarise('ai.noDetail'), true);
    assert.equal(isNothingToSummarise('ai.busy'), false);
    assert.equal(isNothingToSummarise(null), false);
  });
});

describe('other language', () => {
  it('a done answer in another Settings language gets the note; the same language does not', () => {
    const e = run(emptyEntry(), { type: 'saved', saved: { chatId: 1, latest: answer({ lang: 'zh-CN' }), lastGood: null } });
    assert.deepEqual(entryView(e, 'en').note, { kind: 'otherLang', lang: 'zh-CN' });
    assert.equal(entryView(e, 'zh-CN').note, null);
    assert.equal(isOtherLang(answer({ lang: 'ja' }), 'en'), true);
    assert.equal(isOtherLang(answer({ lang: null }), 'en'), false);
    assert.equal(isOtherLang(null, 'en'), false);
  });
  it('a failure note beats the language note', () => {
    const e = run(
      run(emptyEntry(), { type: 'saved', saved: { chatId: 1, latest: answer({ lang: 'ja' }), lastGood: null } }),
      { type: 'ask' }, START, ev('error', { code: 'ai.up.down' }),
    );
    assert.equal(entryView(e, 'en').note?.kind, 'failedPrev');
  });
});

describe('cache keys and bound', () => {
  const base = { mode: 'practice', sessionKey: 'k'.repeat(32), source: 'practice', kind: 'hint', questionId: 'q1' } as const;
  it('hint vs explain, practice vs result, two sessions and two questions are all different', () => {
    const keys = new Set([
      studyKey(base), studyKey({ ...base, kind: 'explain' }), studyKey({ ...base, source: 'result' }),
      studyKey({ ...base, sessionKey: 'j'.repeat(32) }), studyKey({ ...base, questionId: 'q2' }), studyKey({ ...base, mode: 'exam' }),
    ]);
    assert.equal(keys.size, 6);
  });
  it('session part: k<key> for a key, r<id> for a record without one, so a record with a key shares its result page entry', () => {
    assert.equal(sessionPart({ key: 'a'.repeat(32), recordId: 3 }), 'k' + 'a'.repeat(32));
    assert.equal(sessionPart({ key: null, recordId: 3 }), 'r3');
    assert.equal(sessionPart({ key: null, recordId: null }), null);
  });
  it('summary key', () => {
    assert.equal(summaryKey(42, '202611112158'), '42@202611112158');
    assert.equal(summaryKey(42, null), '42@');
  });
  it('the map keeps at most 200 entries and drops the oldest', () => {
    const m = new Map<string, number>();
    for (let i = 0; i < MAX_ENTRIES + 5; i++) boundedSet(m, 'k' + i, i);
    assert.equal(m.size, MAX_ENTRIES);
    assert.equal(m.has('k0'), false);
    assert.equal(m.has('k4'), false);
    assert.equal(m.has('k5'), true);
    boundedSet(m, 'k5', 1); // touching makes it newest
    boundedSet(m, 'new', 1);
    assert.equal(m.has('k5'), true);
    assert.equal(m.has('k6'), false);
  });
});

describe('parsing the wire', () => {
  it('parseSaved with the three shapes', () => {
    assert.deepEqual(parseSaved({ chatId: null, latest: null, lastGood: null }), { chatId: null, chatTitle: null, latest: null, lastGood: null });
    const p = parseSaved({
      chatId: 4, lastGood: null,
      latest: { messageId: 9, kind: 'hint', status: 'streaming', error: null, truncated: false, content: 'x', lang: 'zh-CN', createdAt: 5, finishedAt: null },
    });
    assert.equal(p.chatId, 4);
    assert.equal(p.chatTitle, null);
    assert.equal(p.latest?.status, 'streaming');
    assert.equal(p.latest?.lang, 'zh-CN');
  });
  it('junk becomes null / defaults', () => {
    assert.equal(answerOf(null), null);
    assert.equal(answerOf({ status: 'weird' }), null);
    const a = answerOf({ status: 'done', content: 5, lang: 'fr', sample: { used: 1 } });
    assert.equal(a?.content, '');
    assert.equal(a?.lang, null);
    assert.equal(a?.sample, null);
    assert.deepEqual(parseSaved('x'), { chatId: null, chatTitle: null, latest: null, lastGood: null });
  });
  it('a summary answer keeps its sample', () => {
    const a = answerOf({ status: 'done', content: 's', sample: { used: 70, usable: 75, wrong: 80 } });
    assert.deepEqual(a?.sample, { used: 70, usable: 75, wrong: 80 });
  });
  it('runningOf', () => {
    assert.equal(runningOf(null), null);
    assert.equal(runningOf({ chatId: 1 }), null);
    assert.deepEqual(runningOf({ chatId: 1, messageId: 2, chatKind: 'summary', kind: 'summary', recordId: 5 }), {
      chatId: 1, messageId: 2, chatKind: 'summary', kind: 'summary', questionId: null, questionNo: null, source: null, recordId: 5,
    });
    assert.equal(runningOf({ chatId: 1, messageId: 2, chatKind: 'x', kind: 'y' })?.kind, 'chat');
  });
  it('isAskError', () => {
    assert.equal(isAskError({ status: 409, code: 'ai.busy' }), true);
    assert.equal(isAskError({ status: 0, code: null }), true);
    assert.equal(isAskError(new Error('x')), false);
    assert.equal(isAskError(null), false);
  });
  it('the start event of a summary carries the sample into the attempt', () => {
    const e = run(emptyEntry(), { type: 'ask' }, ev('start', { chatId: 1, assistantMessageId: 2, lang: 'ja', sample: { used: 3, usable: 3, wrong: 3 } }));
    assert.deepEqual(entryView(e, 'ja').sample, { used: 3, usable: 3, wrong: 3 });
  });
  it('a thinking event shows thinking until the first text', () => {
    const e = run(emptyEntry(), { type: 'ask' }, ev('start', { chatId: 1, assistantMessageId: 2, lang: 'en' }), ev('thinking'));
    assert.equal(entryView(e, 'en').thinking, true);
    const e2 = run(e, ev('delta', { text: 'Hi' }));
    assert.equal(entryView(e2, 'en').thinking, false);
    assert.equal(entryView(e2, 'en').text, 'Hi');
  });
});

describe('call-cap refusals expire by themselves', () => {
  const quota = { cap: 'minute' as const, limit: 5, retryAt: 1000 };
  const limited = { ...emptyEntry(), failure: { status: 429, code: 'ai.callLimit', quota } };
  it('before retryAt the note shows (with its quota)', () => {
    const v = entryView(limited, 'en', 999);
    assert.equal(v.note?.kind === 'error' && v.note.quota?.cap, 'minute');
    assert.equal(v.phase, 'error');
  });
  it('at and after retryAt the failure is gone: the prompt (empty) shows again', () => {
    for (const now of [1000, 1500]) {
      const v = entryView(limited, 'en', now);
      assert.equal(v.note, null);
      assert.equal(v.phase, 'empty');
    }
  });
  it('without a clock nothing expires; other failures never expire', () => {
    assert.equal(entryView(limited, 'en').note?.kind, 'error');
    const other = { ...emptyEntry(), failure: { status: 500, code: 'ai.internal' } };
    assert.equal(entryView(other, 'en', 99999).note?.kind, 'error');
  });
  it('an expired cap failure leaves the saved answer without a note', () => {
    const good = answer({ content: 'ok' });
    const v = entryView({ ...limited, shown: good, attempt: good }, 'en', 2000);
    assert.equal(v.text, 'ok');
    assert.equal(v.note, null);
  });
});
