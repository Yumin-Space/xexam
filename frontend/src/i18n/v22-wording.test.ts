// v2.2 wording contract: one quote style per language, full-width colons in zh / ja, one slash form for counters.
import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { test } from 'node:test';

type Entry = { lang: string; key: string; line: string };

function entries(): Entry[] {
  const dir = new URL('./catalog/', import.meta.url);
  const out: Entry[] = [];
  for (const f of readdirSync(dir).filter((n) => n.endsWith('.ts'))) {
    let lang = '';
    for (const line of readFileSync(new URL(f, dir), 'utf8').split('\n')) {
      const m = /^ {2}(en|zh|ja): \{/.exec(line);
      if (m) lang = m[1]!;
      const k = /^ {4}'([^']+)':/.exec(line);
      if (k && lang && k[1] !== 'banks.guide.aiPrompt') out.push({ lang, key: k[1]!, line });
    }
  }
  return out;
}

test('zh uses 「」 for UI names, en uses curly quotes and apostrophes', () => {
  for (const e of entries()) {
    if (e.lang === 'zh') assert.ok(!/[“”]/.test(e.line), `${e.key}: zh uses 「」`);
    if (e.lang === 'en') {
      const val = e.line.replace(/^ {4}'[^']+': /, '');
      assert.ok(!val.includes(String.fromCharCode(92) + "'") && !val.includes('"'), `${e.key}: en uses curly quotes`);
    }
  }
});

test('zh and ja labels end in a full-width colon, and counters use " / "', () => {
  for (const e of entries()) {
    if (e.lang !== 'en') assert.ok(!/[぀-ヿ一-鿿]: /.test(e.line), `${e.key}: full-width colon`);
    assert.ok(!/\{(a|c|w|made|i)\}\/\{/.test(e.line), `${e.key}: spaced slash`);
  }
});
