// The Home AI study assessment card: the answer of the latest route, the run state of a streaming generation and the button.
import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { controlState, type ControlState } from './ai-plus.ts';
import { afterRun, assessDay, capsuleAction, capsuleVisible, IDLE, parseAssessLatest, remainingNote, runReduce, STARTING, type AssessRun } from './dash-assess.ts';

const usable: ControlState = { show: true, usable: true };
const base = { ctl: usable, hasSaved: false, remaining: null, running: false, few: false };

describe('parseAssessLatest', () => {
  it('reads a saved assessment', () => {
    const r = parseAssessLatest({
      assess: { text: ' Steady work. ', day: '2026-11-22', createdAt: 100, latestAt: '2026-11-22 09:10', records: 4, answered: 70, model: 'm' },
      today: '2026-11-22', perDay: 3, remaining: 0,
    });
    assert.deepEqual(r, {
      assess: { text: 'Steady work.', day: '2026-11-22', createdAt: 100, latestAt: '2026-11-22 09:10', records: 4, answered: 70, model: 'm' },
      today: '2026-11-22', perDay: 3, remaining: 0, running: false,
    });
  });
  it('perDay and remaining are numbers or null (unlimited for this viewer)', () => {
    const r = parseAssessLatest({ assess: null, today: 'd', perDay: null, remaining: null })!;
    assert.deepEqual([r.perDay, r.remaining], [null, null]);
    const q = parseAssessLatest({ assess: null, today: 'd', perDay: 3, remaining: 2 })!;
    assert.deepEqual([q.perDay, q.remaining], [3, 2]);
  });
  it('reads the running flag of a run written elsewhere', () => {
    assert.equal(parseAssessLatest({ assess: null, today: 'd', remaining: null, running: true })!.running, true);
    assert.equal(parseAssessLatest({ assess: null, today: 'd' })!.running, false);
  });
  it('null assess, empty text and a missing model', () => {
    assert.equal(parseAssessLatest({ assess: null, today: '2026-11-22', remaining: null })!.assess, null);
    assert.equal(parseAssessLatest({ assess: { text: '  ' }, today: 'd' })!.assess, null);
    assert.equal(parseAssessLatest({ assess: { text: 'x', day: 'd' }, today: 'd' })!.assess!.model, null);
  });
  it('another shape is null', () => {
    assert.equal(parseAssessLatest(null), null);
    assert.equal(parseAssessLatest({ today: 'd' }), null);
    assert.equal(parseAssessLatest('x'), null);
  });
});

describe('runReduce', () => {
  const ev = (event: string, data: unknown = {}) => ({ event, data });
  it('collects delta text and ends with done', () => {
    let r: AssessRun = STARTING;
    r = runReduce(r, ev('start', { chatId: 3 }));
    r = runReduce(r, ev('thinking'));
    assert.deepEqual(r, { phase: 'streaming', text: '', thinking: true });
    r = runReduce(r, ev('delta', { text: 'One. ' }));
    r = runReduce(r, ev('thinking'));
    r = runReduce(r, ev('delta', { text: 'Two.' }));
    assert.deepEqual(r, { phase: 'streaming', text: 'One. Two.', thinking: false });
    assert.deepEqual(runReduce(r, ev('done')), { phase: 'done', text: 'One. Two.' });
  });
  it('stopped goes back to idle; an error event keeps its code and drops the text', () => {
    const r = runReduce(STARTING, ev('delta', { text: 'x' }));
    assert.deepEqual(runReduce(r, ev('stopped')), IDLE);
    assert.deepEqual(runReduce(r, ev('error', { code: 'ai.up.rateLimit' })), { phase: 'error', error: { status: 0, code: 'ai.up.rateLimit' } });
    assert.deepEqual(runReduce(r, ev('error', {})), { phase: 'error', error: { status: 0, code: 'ai.internal' } });
  });
  it('events after the end of a run are ignored', () => {
    const done: AssessRun = { phase: 'done', text: 'x' };
    assert.equal(runReduce(done, { event: 'delta', data: { text: 'y' } }), done);
    assert.equal(runReduce(IDLE, { event: 'delta', data: { text: 'y' } }), IDLE);
  });
});

describe('capsuleAction', () => {
  it('perDay 0 = the assessment is off: no button', () => {
    assert.deepEqual(capsuleAction({ ...base, perDay: 0, remaining: 0 }), { kind: 'none' });
  });
  it('first time: generate; with a saved text: update', () => {
    assert.deepEqual(capsuleAction(base), { kind: 'ask', label: 'generate' });
    assert.deepEqual(capsuleAction({ ...base, hasSaved: true }), { kind: 'ask', label: 'update' });
  });
  it('waits, with its reason: running first, then none left today (remaining 0), then too few records', () => {
    assert.deepEqual(capsuleAction({ ...base, hasSaved: true, running: true, remaining: 0, few: true }), { kind: 'wait', label: 'update', reason: 'running' });
    assert.deepEqual(capsuleAction({ ...base, hasSaved: true, remaining: 0, few: true }), { kind: 'wait', label: 'update', reason: 'daily' });
    assert.deepEqual(capsuleAction({ ...base, few: true }), { kind: 'wait', label: 'generate', reason: 'few' });
  });
  it('the page passes no extra reason: a new user (few records) sees the waiting reason, not a hidden card', () => {
    // pages/dashboard.ts builds ctl with extra = null so the card keeps its own `few` wait reason
    const ctl = controlState({ show: true, usable: true }, null, 'assess');
    assert.deepEqual(capsuleAction({ ...base, ctl, few: true }), { kind: 'wait', label: 'generate', reason: 'few' });
    assert.equal(capsuleVisible({ hasSaved: false, action: { kind: 'wait', label: 'generate', reason: 'few' }, run: IDLE }), true);
  });
  it('a limit with runs left (remaining 3) still asks; an unlimited viewer (null) too', () => {
    assert.deepEqual(capsuleAction({ ...base, remaining: 3 }), { kind: 'ask', label: 'generate' });
    assert.deepEqual(capsuleAction({ ...base, remaining: null }), { kind: 'ask', label: 'generate' });
  });
  it('no button without a usable AI+ (no badge, paused, assessment switch off)', () => {
    assert.deepEqual(capsuleAction({ ...base, ctl: { show: false } }), { kind: 'none' });
    const off = { show: true, usable: false, reason: 'featureOff', tag: true, tip: { key: 'ai.err.featureOff' } } as unknown as ControlState;
    assert.deepEqual(capsuleAction({ ...base, ctl: off }), { kind: 'none' });
  });
});

describe('capsuleVisible', () => {
  const none = { kind: 'none' } as const;
  it('hidden with no text, no button and no run', () => {
    assert.equal(capsuleVisible({ hasSaved: false, action: none, run: IDLE }), false);
  });
  it('shown with a saved text only (no AI+), with a button, or with a run', () => {
    assert.equal(capsuleVisible({ hasSaved: true, action: none, run: IDLE }), true);
    assert.equal(capsuleVisible({ hasSaved: false, action: { kind: 'ask', label: 'generate' }, run: IDLE }), true);
    assert.equal(capsuleVisible({ hasSaved: false, action: none, run: STARTING }), true);
  });
});

describe('assessDay', () => {
  it('the server day, else the local day of createdAt', () => {
    assert.equal(assessDay({ day: '2026-11-22', createdAt: null }), '2026-11-22');
    const t = new Date(2026, 0, 5, 12, 0).getTime() / 1000;
    assert.equal(assessDay({ day: '', createdAt: t }), '2026-01-05');
    assert.equal(assessDay({ day: '', createdAt: null }), '');
  });
});

describe('remaining count', () => {
  it('the meta line shows only a positive count; afterRun counts down to 0 and keeps null', () => {
    assert.equal(remainingNote(2), 2);
    assert.equal(remainingNote(0), null);
    assert.equal(remainingNote(null), null);
    assert.equal(afterRun(2), 1);
    assert.equal(afterRun(0), 0);
    assert.equal(afterRun(null), null);
  });
});
