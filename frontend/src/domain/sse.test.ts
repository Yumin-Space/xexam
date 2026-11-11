// The server-sent events parser of AI Chat's streamed answers (v0.8): line ends LF / CRLF / CR (also split between chunks),
// chunks split anywhere (and inside a multi-byte UTF-8 character, which TextDecoder's stream mode keeps whole), multi-line
// data, comments, unknown event names, and the 1 MiB cap. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import test from 'node:test';
import { createSseParser, SSE_MAX_EVENT, SseTooLarge, type SseEvent } from './sse.ts';

function parseAll(chunks: string[], max?: number): SseEvent[] {
  const out: SseEvent[] = [];
  const p = createSseParser((ev) => out.push(ev), max);
  for (const c of chunks) p.push(c);
  p.end();
  return out;
}

const STREAM = 'event: start\ndata: {"chatId":1}\n\nevent: delta\ndata: {"text":"Hi"}\n\nevent: done\ndata: {"status":"done"}\n\n';
const EXPECTED: SseEvent[] = [
  { event: 'start', data: '{"chatId":1}' },
  { event: 'delta', data: '{"text":"Hi"}' },
  { event: 'done', data: '{"status":"done"}' },
];

test('LF, CRLF and CR line ends give the same events', () => {
  assert.deepEqual(parseAll([STREAM]), EXPECTED);
  assert.deepEqual(parseAll([STREAM.replace(/\n/g, '\r\n')]), EXPECTED);
  assert.deepEqual(parseAll([STREAM.replace(/\n/g, '\r')]), EXPECTED);
});

test('a stream split at every position gives the same events (also a CRLF split between two chunks)', () => {
  for (const text of [STREAM, STREAM.replace(/\n/g, '\r\n'), STREAM.replace(/\n/g, '\r')]) {
    for (let i = 0; i <= text.length; i++) {
      assert.deepEqual(parseAll([text.slice(0, i), text.slice(i)]), EXPECTED, `split at ${i}`);
    }
    // one character at a time
    assert.deepEqual(parseAll([...text]), EXPECTED);
  }
});

test('a multi-byte character split between two network chunks stays whole (decoder in stream mode)', () => {
  const bytes = new TextEncoder().encode('event: delta\ndata: {"text":"日本語と中文"}\n\n');
  for (let i = 1; i < bytes.length; i++) {
    const dec = new TextDecoder('utf-8');
    const out: SseEvent[] = [];
    const p = createSseParser((ev) => out.push(ev));
    p.push(dec.decode(bytes.slice(0, i), { stream: true }));
    p.push(dec.decode(bytes.slice(i), { stream: true }));
    p.push(dec.decode());
    p.end();
    assert.deepEqual(out, [{ event: 'delta', data: '{"text":"日本語と中文"}' }], `split at byte ${i}`);
  }
});

test('multi-line data is joined with LF; one space after the colon is dropped, more are kept', () => {
  assert.deepEqual(parseAll(['data: line one\ndata:line two\ndata:  indented\n\n']), [{ event: 'message', data: 'line one\nline two\n indented' }]);
});

test('comments (the keep-alive) are ignored; the default event name is "message"; unknown names are delivered', () => {
  assert.deepEqual(parseAll([': keep-alive\n\n: another\ndata: {}\n\n']), [{ event: 'message', data: '{}' }]);
  assert.deepEqual(parseAll(['event: something-new\ndata: {"x":1}\nid: 7\nretry: 100\n\n']), [{ event: 'something-new', data: '{"x":1}' }]);
});

test('an event without data is not delivered; an unfinished event at the end is dropped', () => {
  assert.deepEqual(parseAll(['event: ping\n\n']), []);
  assert.deepEqual(parseAll(['event: delta\ndata: {"text":"cut']), []);
  assert.deepEqual(parseAll(['data: a\n\ndata: b\n']), [{ event: 'message', data: 'a' }]);
});

test('an event over the size cap fails, also when it arrives as one endless line', () => {
  assert.equal(SSE_MAX_EVENT, 1024 * 1024);
  assert.throws(() => parseAll(['data: ' + 'x'.repeat(200) + '\n\n'], 100), SseTooLarge);
  assert.throws(() => parseAll(['data: ' + 'x'.repeat(60) + '\n', 'data: ' + 'y'.repeat(60) + '\n\n'], 100), SseTooLarge);
  assert.throws(() => parseAll(['data: ' + 'z'.repeat(300)], 100), SseTooLarge);
  // up to the cap is fine
  assert.equal(parseAll(['data: ' + 'x'.repeat(50) + '\n\n'], 100).length, 1);
});
