// The Markdown subset of AI answers (v0.8): blocks, inline marks, an unclosed fence while an answer is still being written,
// links as plain text, and no HTML interpretation ("<script>" stays text). Made-up text only.
// Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import test from 'node:test';
import { inlineText, parseInline, parseMarkdown } from './ai-markdown.ts';

test('paragraphs, headings, lists, code blocks and rules', () => {
  const src = [
    '# Water cycle',
    'Water moves in a **loop**.',
    'It never stops.',
    '',
    '- evaporation',
    '- condensation',
    '* precipitation',
    '',
    '3. first',
    '4. second',
    '',
    '```text',
    'a  b',
    '  c',
    '```',
    '---',
    'End.',
  ].join('\n');
  const blocks = parseMarkdown(src);
  assert.deepEqual(blocks.map((b) => b.t), ['h', 'p', 'ul', 'ol', 'pre', 'hr', 'p']);
  const [h, p, ul, ol, pre] = blocks;
  assert.equal(h?.t === 'h' && inlineText(h.c), 'Water cycle');
  assert.ok(p?.t === 'p');
  assert.deepEqual(p.c, [{ t: 'text', v: 'Water moves in a ' }, { t: 'b', c: [{ t: 'text', v: 'loop' }] }, { t: 'text', v: '.\nIt never stops.' }]);
  assert.ok(ul?.t === 'ul');
  assert.deepEqual(ul.items.map(inlineText), ['evaporation', 'condensation', 'precipitation']);
  assert.ok(ol?.t === 'ol');
  assert.equal(ol.start, 3);
  assert.deepEqual(ol.items.map(inlineText), ['first', 'second']);
  assert.deepEqual(pre, { t: 'pre', v: 'a  b\n  c' });
});

test('inline: bold, emphasis, code; lone stars, snake_case and unclosed marks stay text', () => {
  assert.deepEqual(parseInline('a **b** *c* `d`'), [
    { t: 'text', v: 'a ' }, { t: 'b', c: [{ t: 'text', v: 'b' }] }, { t: 'text', v: ' ' }, { t: 'em', c: [{ t: 'text', v: 'c' }] },
    { t: 'text', v: ' ' }, { t: 'code', v: 'd' },
  ]);
  assert.deepEqual(parseInline('2 * 3 * 4'), [{ t: 'text', v: '2 * 3 * 4' }]);
  assert.deepEqual(parseInline('snake_case_name'), [{ t: 'text', v: 'snake_case_name' }]);
  assert.deepEqual(parseInline('**open'), [{ t: 'text', v: '**open' }]);
  assert.deepEqual(parseInline('`open'), [{ t: 'text', v: '`open' }]);
  assert.deepEqual(parseInline('``a ` b``'), [{ t: 'code', v: 'a ` b' }]);
  // code keeps its content literally, marks inside it are not parsed
  assert.deepEqual(parseInline('`**x**`'), [{ t: 'code', v: '**x**' }]);
});

test('an unclosed fence (an answer still being written) runs to the end', () => {
  const blocks = parseMarkdown('Look:\n```\nline 1\nline 2');
  assert.deepEqual(blocks, [{ t: 'p', c: [{ t: 'text', v: 'Look:' }] }, { t: 'pre', v: 'line 1\nline 2' }]);
  assert.deepEqual(parseMarkdown(''), []);
  assert.deepEqual(parseMarkdown('\n\n'), []);
});

test('links become plain text; HTML is never interpreted', () => {
  assert.deepEqual(parseInline('see [the notes](https://example.com/notes) here'), [{ t: 'text', v: 'see the notes (https://example.com/notes) here' }]);
  const blocks = parseMarkdown('<script>alert(1)</script>\n\n<b>bold?</b> & <i>');
  assert.deepEqual(blocks, [
    { t: 'p', c: [{ t: 'text', v: '<script>alert(1)</script>' }] },
    { t: 'p', c: [{ t: 'text', v: '<b>bold?</b> & <i>' }] },
  ]);
});

test('a list item continues on an indented line; a blank line between items keeps one list', () => {
  const blocks = parseMarkdown('- one\n  more of one\n\n- two\n\nAfter.');
  assert.equal(blocks.length, 2);
  const [ul, p] = blocks;
  assert.ok(ul?.t === 'ul');
  assert.deepEqual(ul.items.map(inlineText), ['one\nmore of one', 'two']);
  assert.ok(p?.t === 'p' && inlineText(p.c) === 'After.');
  // CRLF line ends
  assert.deepEqual(parseMarkdown('a\r\n\r\nb').map((b) => b.t), ['p', 'p']);
});
