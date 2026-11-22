// v2.5 catalog contract (Home dashboard): every dash.* key and nav.study exist in en, zh and ja with the binding texts.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { setUiLang, t, type MsgKey } from './index.ts';

afterEach(() => setUiLang('en'));

const LANGS = ['en', 'zh', 'ja'] as const;
const KEYS = [
  'nav.study', 'dash.hello', 'dash.last', 'dash.new', 'dash.tiles', 'dash.hero.sub', 'dash.ai', 'dash.ai.sub', 'dash.ai.title',
  'dash.ai.meta', 'dash.ai.dated', 'dash.rank.title', 'dash.rank.basis', 'dash.rank.cols', 'dash.rank.you', 'dash.rank.noneYou',
  'dash.rank.empty', 'dash.hot.title', 'dash.hot.window', 'dash.hot.times', 'dash.hot.empty', 'dash.err',
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
    assert.ok(t('dash.hot.window', { days: 30 }).includes('30'), lang);
    assert.ok(t('dash.hot.empty', { days: 30 }).includes('30'), lang);
    assert.ok(t('dash.hot.times', { n: 7 }).includes('7'), lang);
    const meta = t('dash.ai.meta', { n: 12, q: 340 });
    assert.ok(meta.includes('12') && meta.includes('340'), lang);
    assert.ok(t('dash.ai.dated', { date: '2026-01-02' }).includes('2026-01-02'), lang);
  }
});

test('binding texts', () => {
  setUiLang('zh');
  assert.equal(t('nav.study'), '考试&练习');
  assert.equal(t('dash.hello', { name: 'A' }), '你好，A');
  setUiLang('en');
  assert.equal(t('nav.study'), 'Exam & practice');
  assert.equal(t('dash.rank.title'), 'Leaderboard');
  setUiLang('ja');
  assert.equal(t('nav.study'), '試験・練習');
});
