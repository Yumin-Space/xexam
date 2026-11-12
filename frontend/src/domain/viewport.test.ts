import assert from 'node:assert/strict';
import { test } from 'node:test';
import { keyboardInset, revealDelta, shouldUndoPan, toastLiftPx } from './viewport.ts';

test('keyboardInset: keyboard closed gives 0', () => {
  assert.equal(keyboardInset(812, 812, 0), 0);
});

test('keyboardInset: iOS keyboard open gives its height', () => {
  assert.equal(keyboardInset(812, 470, 0), 342);
});

test('keyboardInset: a panned visual viewport moves the strip, not its size', () => {
  // Safari panned the page by 120 px; the keyboard still covers the bottom 342 - 120 = 222 px of the layout viewport
  assert.equal(keyboardInset(812, 470, 120), 222);
  assert.equal(keyboardInset(812, 470, 342), 0);
});

test('keyboardInset: never negative (pinch zoom, rubber band, odd values)', () => {
  assert.equal(keyboardInset(812, 400, 500), 0);
  assert.equal(keyboardInset(812, 900, 0), 0);
  assert.equal(keyboardInset(812, 406, 406), 0);
  assert.equal(keyboardInset(NaN, 400, 0), 0);
});

test('keyboardInset: Android with resizes-content shrinks innerHeight too, so 0', () => {
  assert.equal(keyboardInset(470, 470, 0), 0);
});

test('keyboardInset: rounds fractional values', () => {
  assert.equal(keyboardInset(812, 469.6, 0), 342);
});

test('revealDelta: inside the band needs no scroll', () => {
  assert.equal(revealDelta(100, 160, 0, 400), 0);
});

test('revealDelta: below the band scrolls down just enough, with the margin', () => {
  assert.equal(revealDelta(300, 420, 0, 400), 28);
});

test('revealDelta: above the band scrolls up (negative)', () => {
  assert.equal(revealDelta(-30, 20, 0, 400), -38);
});

test('revealDelta: honours a panned visual viewport', () => {
  assert.equal(revealDelta(500, 560, 200, 600), 0);
  assert.equal(revealDelta(560, 640, 200, 600), 48);
});

test('revealDelta: a box taller than the band aligns its top', () => {
  assert.equal(revealDelta(150, 700, 0, 400), 142);
});

test('shouldUndoPan: only a locked document that moved', () => {
  assert.equal(shouldUndoPan(false, 40, 0), false);
  assert.equal(shouldUndoPan(true, 0, 0), false);
  assert.equal(shouldUndoPan(true, 12, 0), true);
  assert.equal(shouldUndoPan(true, 0, 80), true);
});

test('toastLiftPx: above a pinned bar, 24 when the bar is far up or away, never off-screen', () => {
  // bar pinned at the bottom of an 844 px window (64 px tall), toast 42 px, top bar 44 px
  assert.equal(toastLiftPx(true, 780, 844, 844, 42, 44), 76);
  assert.equal(toastLiftPx(false, 780, 844, 844, 42, 44), 24);
  // bar in the upper half: the default place is free
  assert.equal(toastLiftPx(true, 200, 264, 844, 42, 44), 24);
  // bar partly above the screen
  assert.equal(toastLiftPx(true, -86, 65, 844, 42, 44), 24);
  // bar low but not pinned: lifted just above it
  assert.equal(toastLiftPx(true, 740, 804, 844, 42, 44), 116);
  // a bar so high that clearing it would push the toast off the top: clamped inside the screen
  assert.equal(toastLiftPx(true, 100, 790, 844, 42, 44), 750);
});

test('toastLiftPx: the bottom safe area moves the default place up, so it counts for the touch test', () => {
  // a bar at 720..770 of an 844 px window: clear of the default toast without a safe area, touched with 34 px of it
  assert.equal(toastLiftPx(true, 720, 770, 844, 42, 44), 24);
  assert.equal(toastLiftPx(true, 720, 770, 844, 42, 44, 34), 136);
  // landscape 844x390, the Home start bar pinned at 300..390, safe bottom 21: just above the bar
  assert.equal(toastLiftPx(true, 300, 390, 390, 41, 44, 21), 102);
});

test('toastLiftPx: iOS keyboard open, measured against the visible bottom so the toast clears the composer', () => {
  // 390x844, keyboard 380: the visible bottom is 464; the composer (60 px) is lifted to 404..464; #toast CSS adds --kb itself
  assert.equal(toastLiftPx(true, 404, 464, 844 - 380, 42, 44, 0), 72);
});
