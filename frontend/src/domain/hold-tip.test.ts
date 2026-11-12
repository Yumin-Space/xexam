// The tap-and-hold tip timing (spec SH-2): hold 450 ms shows the tip and swallows the next click once; an early release,
// a move over 10 px, a cancel or a scroll show nothing (or hide it).
import assert from 'node:assert/strict';
import test from 'node:test';
import { HIDE_AFTER_MS, HOLD_MS, holdIdle, holdStep, type HoldEffect, type HoldEvent, type HoldState } from './hold-tip.ts';

/** Runs events in order and collects every effect */
function run(events: HoldEvent[], start: HoldState = holdIdle()): { s: HoldState; fx: HoldEffect[] } {
  let s = start;
  const fx: HoldEffect[] = [];
  events.forEach((ev, i) => {
    const r = holdStep(s, ev, i * 100);
    s = r.s;
    fx.push(...r.fx);
  });
  return { s, fx };
}
const types = (fx: HoldEffect[]): string[] => fx.map((f) => f.type);

test('a hold of 450 ms shows the tip and the next click is swallowed once', () => {
  const a = run([{ type: 'down', x: 5, y: 5 }, { type: 'timer' }]);
  assert.deepEqual(a.fx[0], { type: 'startTimer', ms: HOLD_MS });
  assert.equal(HOLD_MS, 450);
  assert.deepEqual(types(a.fx), ['startTimer', 'show']);
  assert.equal(a.s.phase, 'shown');
  const b = run([{ type: 'up' }, { type: 'click' }], a.s);
  assert.equal(b.s.phase, 'shown');
  assert.deepEqual(b.fx, [{ type: 'hide', afterMs: HIDE_AFTER_MS }, { type: 'swallowClick' }]);
  assert.equal(HIDE_AFTER_MS, 1500);
});

test('a release before the timer shows nothing and the click passes', () => {
  const r = run([{ type: 'down', x: 5, y: 5 }, { type: 'up' }, { type: 'click' }]);
  assert.deepEqual(types(r.fx), ['startTimer', 'clearTimer']);
  assert.equal(r.s.phase, 'idle');
  assert.equal(r.s.swallow, false);
});

test('a move of more than 10 px cancels the hold, a small move does not', () => {
  const small = run([{ type: 'down', x: 0, y: 0 }, { type: 'move', x: 6, y: 8 }]);
  assert.equal(small.s.phase, 'pressing');
  const far = run([{ type: 'down', x: 0, y: 0 }, { type: 'move', x: 11, y: 0 }, { type: 'timer' }]);
  assert.deepEqual(types(far.fx), ['startTimer', 'clearTimer']);
  assert.equal(far.s.phase, 'idle');
});

test('pointercancel while pressing cancels; after the tip is shown it stays and hides later', () => {
  const pressing = run([{ type: 'down', x: 0, y: 0 }, { type: 'cancel' }]);
  assert.deepEqual(types(pressing.fx), ['startTimer', 'clearTimer']);
  const shown = run([{ type: 'down', x: 0, y: 0 }, { type: 'timer' }, { type: 'cancel' }, { type: 'click' }]);
  assert.deepEqual(types(shown.fx), ['startTimer', 'show', 'hide', 'hide']);
  assert.equal(shown.s.swallow, false);
});

test('a scroll hides the tip and cancels a hold', () => {
  const shown = run([{ type: 'down', x: 0, y: 0 }, { type: 'timer' }, { type: 'scroll' }]);
  assert.deepEqual(shown.fx.slice(-1), [{ type: 'hide', afterMs: 0 }]);
  assert.equal(shown.s.phase, 'idle');
  assert.equal(shown.s.swallow, false);
  const pressing = run([{ type: 'down', x: 0, y: 0 }, { type: 'scroll' }]);
  assert.deepEqual(types(pressing.fx), ['startTimer', 'clearTimer']);
});

test('a new tap hides the old tip and forgets an unused swallow', () => {
  const shown = run([{ type: 'down', x: 0, y: 0 }, { type: 'timer' }]).s;
  const again = run([{ type: 'down', x: 1, y: 1 }, { type: 'up' }, { type: 'click' }], shown);
  assert.deepEqual(types(again.fx), ['hide', 'startTimer', 'clearTimer']);
  assert.equal(again.s.swallow, false);
});

test('a stale timer or events in the idle phase do nothing', () => {
  const r = run([{ type: 'timer' }, { type: 'move', x: 50, y: 50 }, { type: 'up' }, { type: 'cancel' }, { type: 'scroll' }, { type: 'click' }]);
  assert.deepEqual(r.fx, []);
  assert.equal(r.s.phase, 'idle');
});

test('a click on a shown tip without a swallow hides it', () => {
  const s: HoldState = { phase: 'shown', x: 0, y: 0, since: 0, swallow: false };
  const r = holdStep(s, { type: 'click' }, 1);
  assert.deepEqual(r.fx, [{ type: 'hide', afterMs: 0 }]);
  assert.equal(r.s.phase, 'idle');
});
