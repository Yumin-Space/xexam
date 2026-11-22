// v2.5 catalog contract (AI study assessment): the aiplus / error keys exist in en, zh and ja, and the feature relabel and
// the "Exam & practice" wording are in place.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { aiCodeText, knownAiCode } from './errors.ts';
import { setUiLang, t, type MsgKey } from './index.ts';

afterEach(() => setUiLang('en'));

const LANGS = ['en', 'zh', 'ja'] as const;
const KEYS = ['aiplus.why.assessFew', 'aiplus.chat.suffix.assess', 'aiplus.busy.assess', 'ai.err.assessFew'];

test('every v2.5 AI assessment key exists in en, zh and ja', () => {
  for (const lang of LANGS) {
    setUiLang(lang);
    for (const k of KEYS) {
      const v = t(k as MsgKey);
      assert.ok(v && v !== k, `${lang} ${k}`);
    }
  }
});

test('binding texts', () => {
  setUiLang('zh');
  assert.equal(t('aiplus.why.assessFew'), '至少完成 3 次考试或练习后可用');
  assert.equal(t('aiplus.chat.suffix.assess'), 'AI学习评估');
  setUiLang('en');
  assert.equal(t('aiplus.busy.assess'), 'Writing the study assessment…');
  assert.equal(t('ai.feature.summary'), 'AI summary & study assessment');
  assert.equal(t('ai.feature.summary.short'), 'Summary · check');
  // The AI Chat list names the assessment as the dashboard button and the window do
  assert.equal(t('aiplus.chat.suffix.assess'), 'AI study assessment');
  assert.equal(t('aiplus.chat.suffix.assess'), t('dash.ai.title'));
  setUiLang('ja');
  assert.equal(t('ai.feature.summary'), 'AI まとめ・学習評価');
});

test('ai.assessFew is a known code with its own text', () => {
  assert.equal(knownAiCode('ai.assessFew'), true);
  for (const lang of LANGS) {
    setUiLang(lang);
    assert.equal(aiCodeText('ai.assessFew'), t('ai.err.assessFew'));
  }
  setUiLang('en');
  assert.equal(aiCodeText('ai.assessFew'), 'Too few study records; at least 3 are needed.');
});

test('the generation texts name Exam & practice, not Home', () => {
  const GEN = ['ai.set.gen.compBankHint', 'ai.set.gen.compWrongHint', 'ai.set.gen.source.scope', 'ai.set.gen.source.wrong'];
  const WORD = { en: 'Exam & practice', zh: '考试&练习', ja: '試験・練習' } as const;
  for (const lang of LANGS) {
    setUiLang(lang);
    for (const k of GEN) {
      const v = t(k as MsgKey);
      assert.ok(v.includes(WORD[lang]), `${lang} ${k}: ${v}`);
      assert.ok(!/Home|主页|ホーム/.test(v), `${lang} ${k}`);
    }
  }
});
