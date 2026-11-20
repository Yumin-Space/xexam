import assert from 'node:assert/strict';
import { test } from 'node:test';
import { drawnDark, toggledTheme } from './theme-color.ts';

test('toggle saves the opposite of what is on screen', () => {
  assert.equal(toggledTheme('light', false), 'dark');
  assert.equal(toggledTheme('light', true), 'dark');
  assert.equal(toggledTheme('dark', false), 'light');
  assert.equal(toggledTheme('dark', true), 'light');
  assert.equal(toggledTheme('system', true), 'light');
  assert.equal(toggledTheme('system', false), 'dark');
});

test('drawnDark follows the system only for "system"', () => {
  assert.equal(drawnDark('system', true), true);
  assert.equal(drawnDark('system', false), false);
  assert.equal(drawnDark('light', true), false);
  assert.equal(drawnDark('dark', false), true);
});
