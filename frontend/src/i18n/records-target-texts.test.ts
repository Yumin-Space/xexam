// v2.7: the read-only record view speaks about the target user: every language names them and keeps the {a} slot.
import assert from 'node:assert/strict';
import test from 'node:test';
import { setUiLang, t } from './index.ts';
import type { MsgKey } from './index.ts';

for (const lang of ['zh', 'en', 'ja'] as const) {
  test(`target wording names the user in ${lang}`, () => {
    setUiLang(lang);
    for (const k of ['result.q.mineTagOf', 'result.q.mineOf', 'result.q.youChoseOf', 'records.target.detailTitle', 'records.target.detailTitlePractice'] as MsgKey[]) {
      const s = t(k, { name: 'Lee' });
      assert.ok(s.includes('Lee'), `${lang} ${k}`);
      assert.ok(!s.includes('{name}'), `${lang} ${k}`);
    }
    for (const k of ['result.q.mineOf', 'result.q.youChoseOf'] as MsgKey[]) assert.ok(t(k, { name: 'Lee' }).includes('{a}'), `${lang} ${k}`);
  });
}
