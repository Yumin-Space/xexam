// Tap-and-hold tips (v1.0, spec SH-2): the timing of "hold a finger on an icon button to read its name" as a pure state
// machine. components/tooltip.ts feeds the DOM events in and applies the effects; nothing here touches the DOM or a clock.
//   · pointerdown starts a 450 ms hold; when it elapses the tip shows and the click that follows the release is swallowed
//     once (a long press must not also press the button);
//   · releasing earlier shows nothing and the click goes through; moving more than 10 px or pointercancel cancels the hold;
//   · the tip hides 1.5 s after the finger lifts, on scroll, and on the next tap.

/** How long the finger must stay down before the tip shows */
export const HOLD_MS = 450;
/** How far the finger may move (px) before the hold is a scroll or a drag instead */
export const MOVE_LIMIT = 10;
/** How long the tip stays after the finger lifts */
export const HIDE_AFTER_MS = 1500;

export interface HoldState {
  /** idle: nothing; pressing: the finger is down and the timer runs; shown: the tip is visible */
  phase: 'idle' | 'pressing' | 'shown';
  /** Where the finger went down */
  x: number;
  y: number;
  /** The `now` of the pointerdown (for callers that log or compare) */
  since: number;
  /** The next click is the tail of a long press: swallow it once */
  swallow: boolean;
}

export type HoldEvent =
  | { type: 'down'; x: number; y: number }
  | { type: 'move'; x: number; y: number }
  | { type: 'up' }
  | { type: 'cancel' }
  | { type: 'timer' }
  | { type: 'scroll' }
  | { type: 'click' };

export type HoldEffect =
  | { type: 'startTimer'; ms: number }
  | { type: 'clearTimer' }
  | { type: 'show' }
  | { type: 'hide'; afterMs: number }
  | { type: 'swallowClick' };

export const holdIdle = (): HoldState => ({ phase: 'idle', x: 0, y: 0, since: 0, swallow: false });

const none = (s: HoldState): { s: HoldState; fx: HoldEffect[] } => ({ s, fx: [] });

/** One step of the machine: the next state and what the caller must do */
export function holdStep(s: HoldState, ev: HoldEvent, now: number): { s: HoldState; fx: HoldEffect[] } {
  switch (ev.type) {
    case 'down': {
      // A new touch hides the previous tip at once and forgets a click that never came
      const fx: HoldEffect[] = [];
      if (s.phase === 'shown') fx.push({ type: 'hide', afterMs: 0 });
      if (s.phase === 'pressing') fx.push({ type: 'clearTimer' });
      fx.push({ type: 'startTimer', ms: HOLD_MS });
      return { s: { phase: 'pressing', x: ev.x, y: ev.y, since: now, swallow: false }, fx };
    }
    case 'move': {
      if (s.phase !== 'pressing') return none(s);
      if (Math.hypot(ev.x - s.x, ev.y - s.y) <= MOVE_LIMIT) return none(s);
      return { s: { ...s, phase: 'idle' }, fx: [{ type: 'clearTimer' }] };
    }
    case 'up': {
      if (s.phase === 'pressing') return { s: { ...s, phase: 'idle' }, fx: [{ type: 'clearTimer' }] };
      if (s.phase === 'shown') return { s, fx: [{ type: 'hide', afterMs: HIDE_AFTER_MS }] };
      return none(s);
    }
    case 'cancel': {
      if (s.phase === 'pressing') return { s: { ...s, phase: 'idle' }, fx: [{ type: 'clearTimer' }] };
      // The browser may cancel the pointer right after a long press (callout, drag): the tip stays, no click will follow
      if (s.phase === 'shown') return { s: { ...s, swallow: false }, fx: [{ type: 'hide', afterMs: HIDE_AFTER_MS }] };
      return none(s);
    }
    case 'timer': {
      if (s.phase !== 'pressing') return none(s);
      return { s: { ...s, phase: 'shown', swallow: true }, fx: [{ type: 'show' }] };
    }
    case 'scroll': {
      if (s.phase === 'idle') return none(s);
      const fx: HoldEffect[] = s.phase === 'pressing' ? [{ type: 'clearTimer' }] : [{ type: 'hide', afterMs: 0 }];
      return { s: { ...s, phase: 'idle', swallow: false }, fx };
    }
    case 'click': {
      if (s.swallow) return { s: { ...s, swallow: false }, fx: [{ type: 'swallowClick' }] };
      if (s.phase === 'shown') return { s: { ...s, phase: 'idle' }, fx: [{ type: 'hide', afterMs: 0 }] };
      return none(s);
    }
  }
}
