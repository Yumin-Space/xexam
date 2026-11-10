import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { blocks, inlineTokens, parseChangelog } from './changelog.ts';

const SAMPLE = [
  '# Title', '', '## Changelog', '', 'Intro text that is not an entry.', '',
  '### v0.5 — 2026-11-10', '', '**Summary line.**', '', '- **New**: one', '  continued', '- two `code`', '  - nested',
  '', '### v0.4 - 2026-11-09', '- old', '', '## Next section', '- not part of the changelog',
].join('\n');

test('parseChangelog finds every entry and stops at the next level-2 section', () => {
  const e = parseChangelog(SAMPLE);
  assert.deepEqual(e.map((x) => [x.version, x.date]), [['0.5', '2026-11-10'], ['0.4', '2026-11-09']]);
  assert.deepEqual(e[1]?.lines, ['- old']);
  assert.equal(e[0]?.lines[0], '**Summary line.**');
});

test('blocks groups paragraphs, bullets, continuation lines and nested bullets', () => {
  const b = blocks(parseChangelog(SAMPLE)[0]?.lines ?? []);
  assert.equal(b.length, 2);
  assert.deepEqual(b[0], { kind: 'para', text: '**Summary line.**' });
  assert.deepEqual(b[1], { kind: 'list', items: [{ text: '**New**: one continued', children: [] }, { text: 'two `code`', children: ['nested'] }] });
});

test('inlineTokens splits bold, code and links and keeps the rest as text', () => {
  assert.deepEqual(inlineTokens('a **b** `c` [d](docs/en/api.md) e'), [
    { kind: 'text', text: 'a ' }, { kind: 'bold', text: 'b' }, { kind: 'text', text: ' ' }, { kind: 'code', text: 'c' },
    { kind: 'text', text: ' ' }, { kind: 'link', text: 'd', href: 'docs/en/api.md' }, { kind: 'text', text: ' e' },
  ]);
  assert.deepEqual(inlineTokens('plain'), [{ kind: 'text', text: 'plain' }]);
});

test('the three READMEs have a changelog whose newest entry equals VERSION', () => {
  const root = new URL('../../../', import.meta.url);
  const version = readFileSync(new URL('VERSION', root), 'utf8').trim();
  for (const f of ['README.md', 'README.ja.md', 'README.zh-CN.md']) {
    const e = parseChangelog(readFileSync(new URL(f, root), 'utf8'));
    assert.ok(e.length > 3, `${f}: changelog entries found`);
    assert.equal(e[0]?.version, version, `${f}: newest entry`);
    assert.ok((e[0]?.lines.length ?? 0) > 0, `${f}: newest entry has text`);
  }
});
