// v2.5 catalog contract (Home dashboard): every dash.* key and nav.study exist in en, zh and ja with the binding texts.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { setUiLang, t, type MsgKey } from './index.ts';

afterEach(() => setUiLang('en'));

const LANGS = ['en', 'zh', 'ja'] as const;
const KEYS = [
  'nav.study', 'dash.hello', 'dash.last', 'dash.new', 'dash.tiles', 'dash.hero.sub', 'dash.ai.sub', 'dash.ai.title',
  'dash.ai.dated', 'dash.ai.generate', 'dash.ai.update', 'dash.ai.writing', 'dash.ai.stop', 'dash.ai.wait.daily',
  'dash.ai.err.daily', 'dash.wrong.title', 'dash.wrong.meta', 'dash.wrong.miss', 'dash.wrong.misses', 'dash.wrong.aria',
  'dash.wrong.hint', 'dash.wrong.region', 'dash.wrong.stats', 'dash.wrong.podium', 'dash.wrong.empty', 'dash.rank.title', 'dash.rank.basis', 'dash.rank.you', 'dash.rank.nameYou', 'dash.rank.noneYou',
  'dash.rank.empty', 'dash.err',
];

test('every v2.5 dashboard key exists in en, zh and ja', () => {
  for (const lang of LANGS) {
    setUiLang(lang);
    for (const k of KEYS) {
      const v = t(k as MsgKey);
      assert.ok(v && v !== k, `${lang} ${k}`);
    }
  }
});

test('placeholders are filled in every language', () => {
  for (const lang of LANGS) {
    setUiLang(lang);
    assert.ok(t('dash.hello', { name: 'Mia' }).includes('Mia'), lang);
    assert.ok(t('dash.last', { at: '2026-01-02 03:04' }).includes('2026-01-02 03:04'), lang);
    assert.ok(t('dash.wrong.misses', { n: 7 }).includes('7'), lang);
    const aria = t('dash.wrong.aria', { rank: 2, misses: t('dash.wrong.misses', { n: 7 }) });
    assert.ok(aria.includes('2') && aria.includes('7'), lang);
    const stats = t('dash.wrong.stats', { bank: 'B', sheet: 'S', attempts: 9, wrong: 4 });
    assert.ok(stats.includes('B') && stats.includes('S') && stats.includes('9') && stats.includes('4'), lang);
    assert.ok(t('dash.ai.dated', { date: '2026-01-02' }).includes('2026-01-02'), lang);
  }
});

test('binding texts', () => {
  setUiLang('zh');
  assert.equal(t('nav.study'), '考试&练习');
  assert.equal(t('dash.hello', { name: 'A' }), '你好，A');
  assert.equal(t('dash.wrong.title'), '最常错的题');
  assert.equal(t('dash.wrong.meta'), '你的错题 · 前 5');
  assert.equal(t('dash.wrong.hint'), '点一下曲线上的点，看看是哪道题');
  assert.equal(t('dash.wrong.empty'), '还没有错题，继续保持！');
  assert.equal(t('dash.ai.err.daily'), '今天的 AI 学习评估次数已用完，明天再来吧。');
  assert.equal(t('dash.ai.generate'), '生成评估');
  assert.equal(t('dash.ai.update'), '更新评估');
  setUiLang('en');
  assert.equal(t('nav.study'), 'Exam & practice');
  assert.equal(t('dash.rank.title'), 'Leaderboard');
  assert.equal(t('dash.wrong.title'), 'Most missed');
  assert.equal(t('dash.ai.err.daily'), 'You have used up today’s AI study assessments. Come back tomorrow.');
  setUiLang('ja');
  assert.equal(t('nav.study'), '試験・練習');
});
