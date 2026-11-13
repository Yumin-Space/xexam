// The mini icon table (spec 4.2): every icon of the inventory exists and every shape is well-formed and inside the 16×16 box.
import assert from 'node:assert/strict';
import test from 'node:test';
import { MINI, MINI_NAMES, type Shape } from './mini-icon-shapes.ts';

/** The mini icons of the inventory in spec 4.2 (the two 24 px shell icons `gear` and `aiSliders` live in shell.ts) */
const INVENTORY = [
  'back', 'fwd', 'chevDown', 'up', 'down', 'flag', 'exit', 'clock', 'grid', 'bulb', 'book', 'sparkle', 'copy', 'check',
  'xCircle', 'x', 'refresh', 'retake', 'stop', 'send', 'chat', 'memory', 'more', 'plus', 'pencil', 'keyRound', 'pauseCircle',
  'playCircle', 'userOff', 'userOn', 'unlock', 'eye', 'trash', 'upload', 'warn', 'search', 'pause', 'play', 'sun', 'moon',
  'auto', 'filter', 'list', 'home', 'info', 'key', 'plug', 'users', 'chart', 'shield', 'hourglass', 'aiGen', 'aiImport', 'submit',
];

test('every icon of the inventory exists', () => {
  for (const name of INVENTORY) assert.ok(name in MINI, `missing icon ${name}`);
});

test('the names list matches the table and has no duplicates', () => {
  assert.deepEqual(MINI_NAMES, Object.keys(MINI));
  assert.equal(new Set(MINI_NAMES).size, MINI_NAMES.length);
});

const PATH = /^(?:[MmLlHhVvCcSsQqAaZz]|-?\d*\.?\d+|[\s,])+$/;
const inBox = (n: number): boolean => Number.isFinite(n) && n >= 0 && n <= 16;

test('every icon has shapes, paths use only path syntax, primitives stay inside the 16x16 box', () => {
  for (const name of MINI_NAMES) {
    const shapes = MINI[name] as Shape[];
    assert.ok(shapes.length > 0, `${name} has no shapes`);
    for (const sh of shapes) {
      if ('d' in sh) {
        assert.match(sh.d, PATH, `${name}: bad path ${sh.d}`);
        assert.match(sh.d, /^M/, `${name}: a path starts with M`);
      } else if ('cx' in sh) {
        assert.ok(sh.r > 0 && inBox(sh.cx - sh.r) && inBox(sh.cx + sh.r) && inBox(sh.cy - sh.r) && inBox(sh.cy + sh.r), `${name}: circle outside the box`);
      } else {
        assert.ok(sh.w > 0 && sh.h > 0 && inBox(sh.x) && inBox(sh.y) && inBox(sh.x + sh.w) && inBox(sh.y + sh.h) && sh.rx >= 0, `${name}: rect outside the box`);
      }
    }
  }
});

test('the icons that share a meaning are not drawn twice', () => {
  const sig = (n: keyof typeof MINI): string => JSON.stringify(MINI[n]);
  const seen = new Map<string, string>();
  for (const name of MINI_NAMES) {
    const s = sig(name);
    assert.ok(!seen.has(s), `${name} duplicates ${seen.get(s)}`);
    seen.set(s, name);
  }
});
