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
  // v2.6: the assessment is its own feature; the summary label is just "AI summary"
  assert.equal(t('ai.feature.summary'), 'AI summary');
  assert.equal(t('ai.feature.assess'), 'AI study assessment');
  // The AI Chat list names the assessment as the dashboard button and the window do
  assert.equal(t('aiplus.chat.suffix.assess'), 'AI study assessment');
  assert.equal(t('aiplus.chat.suffix.assess'), t('dash.ai.title'));
  setUiLang('ja');
  assert.equal(t('ai.feature.summary'), 'AI まとめ');
  assert.equal(t('ai.feature.assess'), 'AI 学習状況の評価');
  setUiLang('zh');
  assert.equal(t('ai.feature.summary'), 'AI 总结');
  assert.equal(t('ai.feature.assess'), 'AI 学习评估');
  assert.equal(t('ai.err.assessDaily'), '今天的 AI 学习评估次数已用完，明天再来吧。');
  assert.equal(t('dash.ai.wait.daily'), '今天的评估次数已用完，明天再来');
  assert.equal(t('dash.ai.remaining', { n: 2 }), '今天还能评估 2 次');
});

test('v2.6 assessment feature keys exist in en, zh and ja', () => {
  const K = ['ai.feature.assess', 'ai.feature.assess.short', 'ai.set.slot.assess', 'ai.set.safeTab.assess', 'ai.set.safeTab.assessShort',
    'ai.limits.group.assess', 'ai.set.limit.assessPerDay', 'ai.set.limit.assessPerDayHint', 'ai.set.limit.assessAdminsExempt',
    'ai.set.usage.purpose.assess', 'dash.ai.remaining', 'dash.ai.wait.daily', 'ai.err.assessDaily'];
  for (const lang of LANGS) {
    setUiLang(lang);
    for (const k of K) {
      const v = t(k as MsgKey, { n: 1 });
      assert.ok(v && v !== k, `${lang} ${k}`);
    }
  }
  assert.equal(knownAiCode('ai.assessDaily'), true);
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
