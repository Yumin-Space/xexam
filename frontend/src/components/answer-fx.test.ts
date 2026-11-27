// v3.0: the answer effects of a checked practice answer (source checks: the module needs a browser to run).
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { DEFAULT_SETTINGS } from '../state/settings-cache.ts';

const src = (rel: string): string => readFileSync(new URL(rel, import.meta.url), 'utf8');

test('answer effects are on by default', () => {
  assert.equal(DEFAULT_SETTINGS.answerFx, true);
});

test('the effect runs only for a fresh practice verdict and only with the setting on', () => {
  const quiz = src('../pages/quiz.ts');
  assert.match(quiz, /const fresh = practice && freshlyChecked\(se, q\.id\)/);
  assert.match(quiz, /if \(fresh && verdictEl\) \{[\s\S]*?if \(S\.settings\.answerFx\) \{[\s\S]*?answerFx\(ok, \{ card: paper, key: se\.key \}\)/);
  assert.equal(quiz.match(/answerFx\(/g)?.length, 1, 'one call site');
  assert.match(quiz, /if \(practice && S\.settings\.answerFx\) prewarmAnswerFx\(\);/);
});

test('another question ends the effect (Next, the arrow keys, the answer sheet)', () => {
  assert.match(src('../pages/quiz.ts'), /if \(q\.id !== fxQuestion\) \{\s*fxQuestion = q\.id;\s*stopAnswerFx\(\);/);
});

test('leaving the quiz stops the effect and frees its bitmaps', () => {
  assert.match(src('../state/actions.ts'), /from\.name === 'quiz' && next\.name !== 'quiz'\) releaseAnswerFx\(\)/);
  assert.match(src('./answer-fx.ts'), /export function releaseAnswerFx\(\): void \{\s*stopAnswerFx\(\);\s*rasterCache\.clear\(\);/);
});

test('ending an effect early cancels its timers, sounds and buzz', () => {
  const fx = src('./answer-fx.ts');
  assert.match(fx, /timers\.forEach\(\(id\) => window\.clearTimeout\(id\)\)/);
  assert.match(fx, /bus\.gain\.cancelScheduledValues\(0\)/);
  assert.match(fx, /buzz\(0\)/);
  // every sound goes through the effect's bus, none straight to the speakers
  assert.equal(fx.match(/ctx\.destination/g)?.length, 1, 'only the bus is connected to the destination');
  // the claw timeline uses the layer's timers only
  const claws = fx.slice(fx.indexOf('function claws('), fx.indexOf('export function answerFx'));
  assert.doesNotMatch(claws, /window\.setTimeout/);
  assert.match(claws, /layer\(3000\)/);
});

test('the effects work under the CSP: a stylesheet file, data: images, no CSS filter on the moving layer', () => {
  const fx = src('./answer-fx.ts');
  assert.match(fx, /^import '\.\.\/styles\/answer-fx\.css';/m);
  assert.doesNotMatch(fx, /createElement\('style'\)|createObjectURL/);
  assert.match(fx, /'data:image\/svg\+xml;charset=utf-8,'/);
  assert.doesNotMatch(src('../styles/answer-fx.css'), /filter:|mask-image/);
});

test('reduced motion skips both effects; one effect at a time; taps go through and do not end it', () => {
  const fx = src('./answer-fx.ts');
  assert.match(fx, /export function answerFx\(ok: boolean, opts: \{ card\?: HTMLElement \| null; key\?: unknown \}\): void \{\s*if \(reduced\(\)\) return;/);
  assert.match(fx, /prefers-reduced-motion: reduce/);
  assert.match(fx, /function layer\([^)]*\)[^{]*\{\s+stopAnswerFx\(\);/);
  assert.doesNotMatch(fx, /addEventListener\('pointerdown'/);
  const css = src('../styles/answer-fx.css');
  assert.match(css, /pointer-events: none/);
  assert.match(css, /z-index: 9999/);
});

test('no preview-only code is left in the port', () => {
  const fx = src('./answer-fx.ts');
  assert.doesNotMatch(fx, /PREVIEW|__fxCost|demoButtons|fx-demo|fx-crack|fx-word|soundOn/);
});

test('the settings page has the switch, saved like auto-advance', () => {
  const page = src('../pages/settings.ts');
  assert.match(page, /save\(\{ answerFx: !S\.settings\.answerFx \}\)/);
  assert.match(page, /s\.answerFx === DEFAULT_SETTINGS\.answerFx/);
});
