// v2.2: phone account tip text, and the wiring of the phone top bar / quiz theme toggle / footer rules (source checks).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { joinAccountTip } from './account-tip.ts';

const src = (rel: string): string => readFileSync(new URL(rel, import.meta.url), 'utf8');

test('account tip is one line without a badge and two lines with one', () => {
  assert.equal(joinAccountTip('Signed in as ann (User)', null), 'Signed in as ann (User)');
  assert.equal(joinAccountTip('Signed in as ann (User)', 'AI+ is on'), 'Signed in as ann (User)\nAI+ is on');
});

test('quiz top row carries the shared theme toggle left of save-and-exit', () => {
  const quiz = src('../pages/quiz.ts');
  assert.match(quiz, /themeButton\('qtheme'\),\s*exit\)/);
  assert.match(src('../components/shell.ts'), /export function themeButton/);
});

test('phone bar hides the name; footer note and shutdown are centered', () => {
  assert.match(src('../styles/shell.css'), /\.acct-name \{ display: none; \}/);
  const main = src('../styles/main.css');
  assert.match(main, /\.foot-note \{[^}]*width: fit-content; max-width: 100%; margin-inline: auto;/);
  assert.match(main, /\.foot \.btn\.quiet \{ align-self: center;/);
});

test('v2.2 fix round: account button redraws across the phone line, one tip at a time, badge never squeezed', () => {
  const shell = src('../components/shell.ts');
  assert.match(shell, /matchMedia\(PHONE_MQ\)\.addEventListener\('change', refreshAccountControl\)/);
  assert.match(shell, /wrap\.replaceWith\(bd\)/);
  assert.match(src('../styles/shell.css'), /\.acct-btn \.ai-badge-tip \{ flex: none; \}/);
});

test('quiz top row steps down (short count, tighter gaps, no clock icon) before the position is clipped', () => {
  assert.match(src('../pages/quiz.ts'), /\['tight', 'tighter', 'tightest', 'tightest1', 'tightest1b', 'tightest2', 'tightest3']/);
  const css = src('../styles/quiz.css');
  assert.match(css, /\.qbar\.tight \.qcount-s \{ display: inline; \}/);
  // The toggle's box is its own 44 px tap area (no ::after a neighbour could cover)
  assert.match(css, /\.qbar :is\(\.theme-btn, \.hand-btn\) \{ position: relative; width: 44px; height: 44px; margin-inline: -8px; \}/);
  assert.doesNotMatch(css, /\.theme-btn::after \{ content: ""; position: absolute; inset: 0 -/);
});

test('phone top bar keeps the brand name and drops the admin tag first, then the icon', () => {
  assert.match(src('../components/shell.ts'), /matches\(PHONE_MQ\) \? \['fit-0', 'fit-2', 'fit-1', 'fit-3', 'fit-4'\]/);
  assert.match(src('../styles/shell.css'), /min-width: 44px; justify-content: center;/);
});
