import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import {
  backdropCloses, escapeClosesPopover, focusOutCloses, focusTitleOnOpen, KEEP_SELECTOR, openerStillMatches, outsidePointerCloses, overlayTop, placePopover, popWidth,
  togglePlan, type PlaceInput,
} from './ai-popover.ts';

const base: PlaceInput = {
  opener: { top: 100, bottom: 132, left: 800, right: 832 },
  pop: { width: 440, height: 300 },
  vw: 1200, vh: 800, inset: { top: 0, bottom: 0 }, preferAbove: false,
};

describe('placePopover', () => {
  it('below the opener, right edges aligned', () => {
    const p = placePopover(base);
    assert.deepEqual(p, { top: 138, left: 392, placement: 'below', fullWidth: false });
  });
  it('a bar button opens upwards', () => {
    const p = placePopover({ ...base, opener: { top: 700, bottom: 740, left: 300, right: 400 }, preferAbove: true });
    assert.equal(p.placement, 'above');
    assert.equal(p.top, 700 - 6 - 300);
  });
  it('less than 240 px below and more above: above', () => {
    const p = placePopover({ ...base, opener: { top: 600, bottom: 632, left: 800, right: 832 } });
    assert.equal(p.placement, 'above');
  });
  it('less than 240 px below but even less above: stays below and is clamped', () => {
    const p = placePopover({ ...base, vh: 400, opener: { top: 60, bottom: 92, left: 800, right: 832 } });
    assert.equal(p.placement, 'below');
    assert.equal(p.top, 92);
    const q = placePopover({ ...base, vh: 330, opener: { top: 60, bottom: 92, left: 800, right: 832 } });
    assert.equal(q.top, 330 - 300 - 8, 'clamped so the bottom edge stays in the window');
  });
  it('an opener scrolled far out of view never takes the popover off screen', () => {
    const up = placePopover({ ...base, opener: { top: -500, bottom: -468, left: 800, right: 832 } });
    assert.equal(up.top, 8);
    const down = placePopover({ ...base, opener: { top: 3000, bottom: 3032, left: 800, right: 832 } });
    assert.ok(down.top >= 8 && down.top + 300 <= 800 - 8);
  });
  it('the safe-area insets are respected', () => {
    const p = placePopover({ ...base, inset: { top: 44, bottom: 34 }, opener: { top: -500, bottom: -468, left: 800, right: 832 } });
    assert.equal(p.top, 52);
    const q = placePopover({ ...base, inset: { top: 44, bottom: 34 }, opener: { top: 3000, bottom: 3032, left: 800, right: 832 } });
    assert.equal(q.top, 800 - 300 - 8 - 34);
  });
  it('left is clamped to the window', () => {
    assert.equal(placePopover({ ...base, opener: { top: 100, bottom: 132, left: 10, right: 50 } }).left, 8);
    assert.equal(placePopover({ ...base, vw: 700, opener: { top: 100, bottom: 132, left: 690, right: 760 } }).left, 700 - 440 - 8);
  });
  it('windows up to 600 px wide: full width with 8 px sides', () => {
    const p = placePopover({ ...base, vw: 390, pop: { width: 374, height: 300 } });
    assert.equal(p.fullWidth, true);
    assert.equal(p.left, 8);
    assert.equal(popWidth(390), 374);
    assert.equal(popWidth(1200), 440);
  });
  it('a popover taller than the window starts at the top edge', () => {
    assert.equal(placePopover({ ...base, vh: 200 }).top, 8);
  });
});

describe('togglePlan', () => {
  const open = { kind: 'explain', qid: 'q1', source: 'result', anchorKey: 'rv-book-q1' };
  it('nothing open: open', () => {
    assert.equal(togglePlan(null, open), 'open');
  });
  it('the same opener again closes', () => {
    assert.equal(togglePlan(open, { ...open }), 'close');
  });
  it('the other opener of the same pair moves the popover', () => {
    assert.equal(togglePlan(open, { ...open, anchorKey: 'rv-ai-q1' }), 'move');
  });
  it('another kind, question or place switches the content', () => {
    assert.equal(togglePlan(open, { ...open, kind: 'hint' }), 'switch');
    assert.equal(togglePlan(open, { ...open, qid: 'q2' }), 'switch');
    assert.equal(togglePlan(open, { ...open, source: 'practice' }), 'switch');
  });
});

describe('closing rules', () => {
  it('pointerdown outside closes; inside the popover or on an opener does not', () => {
    assert.equal(outsidePointerCloses(false, false), true);
    assert.equal(outsidePointerCloses(true, false), false);
    assert.equal(outsidePointerCloses(false, true), false);
  });
  it('focus out: only a known target outside both closes; a null target (a redraw) keeps it open', () => {
    assert.equal(focusOutCloses(true, false, false), true);
    assert.equal(focusOutCloses(false, false, false), false);
    assert.equal(focusOutCloses(true, true, false), false);
    assert.equal(focusOutCloses(true, false, true), false);
  });
  it('a keep element (language switch, copy fallback textarea) neither closes by pointerdown nor by focus', () => {
    assert.equal(outsidePointerCloses(false, false, true), false);
    assert.equal(focusOutCloses(true, false, false, true), false);
    assert.equal(focusOutCloses(true, false, false, false), true);
    assert.ok(KEEP_SELECTOR.includes('.lang') && KEEP_SELECTOR.includes('[data-aip-keep]') && KEEP_SELECTOR.includes('[data-aip-copy]'));
  });
  it('after a redraw an opener of another question no longer matches; no pair attribute keeps it', () => {
    assert.equal(openerStillMatches('q3|hint|practice', 'q3|hint|practice'), true);
    assert.equal(openerStillMatches('q4|hint|practice', 'q3|hint|practice'), false);
    assert.equal(openerStillMatches(null, 'q3|hint|practice'), true);
  });
  it('Escape closes the popover only when no modal dialog is open', () => {
    assert.equal(escapeClosesPopover(true, false), true);
    assert.equal(escapeClosesPopover(true, true), false);
    assert.equal(escapeClosesPopover(false, false), false);
  });
  it('the overlay closes on its backdrop only when pointerdown and click both hit it', () => {
    assert.equal(backdropCloses(true, true), true);
    assert.equal(backdropCloses(false, true), false);
    assert.equal(backdropCloses(true, false), false);
  });
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
