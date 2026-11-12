import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { Block } from './changelog.ts';
import { SHOWN_BULLETS, foldEntry } from './about-fold.ts';

const item = (text: string): { text: string; children: string[] } => ({ text, children: [] });
const list = (...texts: string[]): Block => ({ kind: 'list', items: texts.map(item) });
const para = (text: string): Block => ({ kind: 'para', text });

test('the lead paragraph and the first three bullets show; the rest is folded', () => {
  const f = foldEntry([para('Lead'), list('a', 'b', 'c', 'd', 'e')]);
  assert.equal(SHOWN_BULLETS, 3);
  assert.equal(f.total, 5);
  assert.equal(f.folded, true);
  assert.deepEqual(f.shown, [para('Lead'), list('a', 'b', 'c')]);
});

test('a short entry is not folded', () => {
  const f = foldEntry([para('Lead'), list('a', 'b')]);
  assert.equal(f.folded, false);
  assert.equal(f.total, 2);
  assert.equal(f.shown.length, 2);
});

test('bullets spread over several lists are counted together; a paragraph after the bullets is folded', () => {
  const f = foldEntry([para('Lead'), list('a', 'b'), list('c', 'd'), para('Tail')]);
  assert.equal(f.total, 4);
  assert.equal(f.folded, true);
  assert.deepEqual(f.shown, [para('Lead'), list('a', 'b'), list('c')]);
  const g = foldEntry([para('Lead'), list('a'), para('Tail')]);
  assert.equal(g.folded, true);
  assert.deepEqual(g.shown, [para('Lead'), list('a')]);
});

test('an entry without a list or without text', () => {
  assert.deepEqual(foldEntry([]), { shown: [], total: 0, folded: false });
  assert.deepEqual(foldEntry([para('Only a lead')]), { shown: [para('Only a lead')], total: 0, folded: false });
});

test('a bullet keeps its children when it is shown', () => {
  const b: Block = { kind: 'list', items: [{ text: 'a', children: ['x', 'y'] }] };
  assert.deepEqual(foldEntry([b]).shown, [b]);
});
