// v2.2 catalog contract (AI Chat strength button): the Default texts exist in en / zh / ja.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { setUiLang, t } from './index.ts';

afterEach(() => setUiLang('en'));

test('strength button Default texts exist in every language', () => {
  const want = { en: 'Default', zh: '默认', ja: '既定' } as const;
  for (const lang of ['en', 'zh', 'ja'] as const) {
    setUiLang(lang);
    assert.equal(t('ai.effort.default'), want[lang]);
    assert.ok(t('ai.effort.defaultOption').startsWith(want[lang]) && t('ai.effort.defaultOption').length > want[lang].length + 4, lang);
    assert.ok(t('ai.effort.follow', { level: 'LOW' }).includes('LOW'), lang);
    assert.ok(!t('ai.effort.defaultOption').includes('パラメータを'), lang);
  }
});
