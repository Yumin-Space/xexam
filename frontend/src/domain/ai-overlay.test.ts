import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { backdropCloses, focusTitleOnOpen, overlayTop, pickFocusTarget } from './ai-overlay.ts';

describe('backdropCloses', () => {
  it('the overlay closes on its backdrop only when pointerdown and click both hit it', () => {
    assert.equal(backdropCloses(true, true), true);
    assert.equal(backdropCloses(false, true), false);
    assert.equal(backdropCloses(true, false), false);
  });
});

describe('focusTitleOnOpen', () => {
  it('a keyboard click (detail 0) moves focus to the title', () => {
    assert.equal(focusTitleOnOpen(0), true);
    assert.equal(focusTitleOnOpen(1), false);
  });
});

describe('overlayTop', () => {
  it('below the page title + 8 px', () => {
    assert.equal(overlayTop(80, 900, 0), 88);
  });
  it('no title: 16 px; the safe area is added; at most 40 % of the window', () => {
    assert.equal(overlayTop(null, 900, 0), 16);
    assert.equal(overlayTop(null, 900, 47), 63);
    assert.equal(overlayTop(700, 900, 0), 360);
  });
});

describe('pickFocusTarget', () => {
  it('the button when there is one, else the fallback, else null', () => {
    assert.equal(pickFocusTarget('b', 'f'), 'b');
    assert.equal(pickFocusTarget(null, 'f'), 'f');
    assert.equal(pickFocusTarget(undefined, null), null);
  });
});
