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
    // a placeholder for the number and the topic the user fills in
    assert.match(p, lang === 'en' ? /\[NUMBER OF QUESTIONS\]/ : lang === 'zh' ? /\[题目数量\]/ : /\[問題数\]/);
  });
}
