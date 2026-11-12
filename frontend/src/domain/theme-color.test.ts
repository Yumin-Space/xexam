import assert from 'node:assert/strict';
import { test } from 'node:test';
import { DARK_BAR, LIGHT_BAR, barColor } from './theme-color.ts';

const LIGHT_META = '(prefers-color-scheme: light)';
const DARK_META = '(prefers-color-scheme: dark)';

test('a forced theme sets both metas to its colour', () => {
  for (const sys of [true, false]) {
    assert.equal(barColor('light', LIGHT_META, sys), LIGHT_BAR);
    assert.equal(barColor('light', DARK_META, sys), LIGHT_BAR);
    assert.equal(barColor('dark', LIGHT_META, sys), DARK_BAR);
    assert.equal(barColor('dark', DARK_META, sys), DARK_BAR);
  }
});

test('follow system restores each meta to its own colour', () => {
  assert.equal(barColor('system', LIGHT_META, true), LIGHT_BAR);
  assert.equal(barColor('system', DARK_META, false), DARK_BAR);
});

test('a meta without media follows the system theme', () => {
  assert.equal(barColor('system', null, true), DARK_BAR);
  assert.equal(barColor('system', null, false), LIGHT_BAR);
});
