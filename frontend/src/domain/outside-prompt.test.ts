// The "use an outside AI" prompt of the Excel guide (A-OWNER-79): the copy button copies the full prompt, and the prompt states
// the real six-column format in each language. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import test from 'node:test';
import { setUiLang } from '../i18n/index.ts';
import { copyOutsidePrompt, outsidePrompt } from './outside-prompt.ts';

for (const lang of ['en', 'zh', 'ja'] as const) {
  test(`the copy button copies the whole prompt (${lang})`, async () => {
    setUiLang(lang);
    let copied = '';
    const ok = await copyOutsidePrompt(async (text) => {
      copied = text;
      return true;
    });
    assert.equal(ok, true);
    assert.equal(copied, outsidePrompt());
    assert.ok(copied.length > 500);
  });

  test(`the prompt states the six columns and the headings the parser reads (${lang})`, () => {
    setUiLang(lang);
    const p = outsidePrompt();
    for (const s of ['Question (English)', 'Answer (Japanese)', '【Answer】', '【答案】', '【正解】', '【Explanation】', '【解析】', '【解説】',
      '【Memory aid】', '【记忆口诀】', '【覚え方】', '.xlsx', 'A. ']) {
      assert.ok(p.includes(s), `${lang}: ${s}`);
    }
    // v1.4: one fixed prompt that works as it is: no [placeholder] is left to fill in (the only "[" is in the list of
    // characters a sheet name must not contain) and the 【】 brackets are untouched
    const withoutForbiddenList = p.replace(': \\ / ? * [ ]', '');
    assert.ok(!withoutForbiddenList.includes('['), `${lang}: placeholder`);
    assert.ok(!/[A-Z ]*NUMBER OF QUESTIONS|\[TOPIC\]/.test(p));
    assert.equal(p.split('【').length, p.split('】').length);
    assert.ok(!p.includes('['+'题目数量') && !p.includes('[問題数]'));
  });
}

// v1.4: the prompt ends with two complete example rows (one single choice, one multiple choice, all six cells) that only show the
// format: their headings are the ones the rules require and their answer letters match their options
const HEADINGS = ['【Answer】', '【答案】', '【正解】', '【Explanation】', '【解析】', '【解説】', '【Memory aid】', '【记忆口诀】', '【覚え方】'];
const EXAMPLE_MARK = { en: 'EXAMPLE\n', zh: '示例\n', ja: '例\n' } as const;
const LAST_LINE = { en: 'Answer with the file', zh: '只给我文件', ja: 'ファイル（または表）だけを' } as const;

for (const lang of ['en', 'zh', 'ja'] as const) {
  test(`the example rows are in the prompt and agree with the rules (${lang})`, () => {
    setUiLang(lang);
    const p = outsidePrompt();
    const start = p.indexOf(EXAMPLE_MARK[lang]);
    const end = p.indexOf(LAST_LINE[lang]);
    assert.ok(start > 0 && end > start, `${lang}: example section before the last line`);
    const rules = p.slice(0, start);
    const example = p.slice(start, end);
    // two examples with six labelled cells each
    for (const c of 'ABCDEF') {
      const label = lang === 'en' ? `Column ${c}:\n` : `${c} 列：\n`;
      assert.equal(example.split(label).length - 1, 2, `${lang}: column ${c} twice`);
    }
    // the cell texts of the guide's made-up questions
    assert.ok(example.includes('Which planet is known as the Red Planet?') && example.includes('Which two of the following are fruits?'));
    assert.ok(example.includes('哪颗行星被称为') && example.includes('下面哪两个是水果') && example.includes('「赤い惑星」') && example.includes('果物はどれですか'));
    // per example: D, E and F give the same letters, A, B and C list the same options, and every answer letter is one of them
    const label = (c: string) => (lang === 'en' ? `Column ${c}:\n` : `${c} 列：\n`);
    const two = { en: 'Example 2 (', zh: '示例 2（', ja: '例 2（' }[lang];
    const cut = example.indexOf(two);
    assert.ok(cut > 0, `${lang}: example 2 label`);
    const cell = (ex: string, c: string) => {
      const i = ex.indexOf(label(c));
      assert.ok(i >= 0, `${lang}: column ${c}`);
      const rest = ex.slice(i + label(c).length);
      const next = rest.search(/\n\n(?:Column [A-F]:|[A-F] 列：)\n/);
      return next < 0 ? rest : rest.slice(0, next);
    };
    const sections = [[example.slice(0, cut), 1], [example.slice(cut), 2]] as const;
    for (const [ex, kind] of sections) {
      const answers = ['D', 'E', 'F'].map((c) => {
        const m = cell(ex, c).match(/^【(?:Answer|答案|正解)】([A-H](?:[、, ]+[A-H])*)/);
        assert.ok(m, `${lang}: answer line in column ${c}`);
        return (m[1] ?? '').split(/[、, ]+/);
      });
      const [d = [], e, f] = answers;
      assert.deepEqual(e, d, `${lang}: E = D`);
      assert.deepEqual(f, d, `${lang}: F = D`);
      const [oa = [], ob, oc] = ['A', 'B', 'C'].map((c) => [...cell(ex, c).matchAll(/^([A-H])\. /gm)].map((m) => m[1]));
      assert.deepEqual(ob, oa, `${lang}: B options = A`);
      assert.deepEqual(oc, oa, `${lang}: C options = A`);
      assert.ok(oa.length >= 4, `${lang}: options`);
      for (const a of d) assert.ok(oa.includes(a), `${lang}: ${a} is an option`);
      if (kind === 1) assert.equal(d.length, 1, `${lang}: single choice`);
      else assert.ok(d.length >= 2, `${lang}: multiple choice`);
    }
    assert.equal((example.match(/【(?:Answer|答案|正解)】/g) ?? []).length, 6);
    // the 【】 headings of the examples are the ones the rules require
    const used = new Set(example.match(/【[^】]*】/g));
    assert.equal(used.size, 9);
    for (const h of used) assert.ok(HEADINGS.includes(h) && rules.includes(h), `${lang}: ${h}`);
    assert.ok(example.includes('NOT') || example.includes('不能') || example.includes('入れないで'));
    assert.ok(!example.includes('['));
  });
}
