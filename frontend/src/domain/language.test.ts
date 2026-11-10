import assert from 'node:assert/strict';
import test from 'node:test';
import { effectiveLang, hasLang, isLang, shown } from './language.ts';
import type { Question } from '../types/api.ts';

const base = {
  tag: '', translation: '', explanation: '', mnemonic: '', note: '', id: 'q', sheet: 'S', row: 1,
};

const choice = (extra: Partial<Question> = {}): Question => ({
  ...base,
  kind: 'single',
  stem: '日本語の題干',
  options: [{ key: 'A', text: '甲' }, { key: 'B', text: '乙' }],
  items: [],
  answer: ['B'],
  zh: { stem: '中文题干', options: ['甲(中)', '乙(中)'] },
  english: 'English stem',
  en: { stem: 'English stem', options: ['Alpha', 'Beta'] },
  ...extra,
});

const hotspot = (extra: Partial<Question> = {}): Question => ({
  ...base,
  kind: 'hotspot',
  stem: '日本語の題干',
  options: [],
  items: [{ text: '太陽は東から昇る。', choices: ['はい', 'いいえ'] }, { text: '月は恒星である。', choices: ['はい', 'いいえ'] }],
  answer: [0, 1],
  zh: { stem: '中文题干', items: ['太阳东升。', '月亮是恒星。'], choices: [['是', '否'], ['是', '否']] },
  en: { stem: 'English stem', items: ['The sun rises in the east.', 'The moon is a star.'], choices: [['Yes', 'No'], ['Yes', 'No']] },
  ...extra,
});

test('isLang 只认 ja / zh / en（存储里的脏值一律不要）', () => {
  assert.equal(isLang('ja'), true);
  assert.equal(isLang('zh'), true);
  assert.equal(isLang('en'), true);
  for (const bad of ['', 'EN', 'fr', null, undefined, 1, {}]) assert.equal(isLang(bad), false);
});

test('日语永远可用；中文 / 英语要有对应结构才可用', () => {
  const q = choice();
  assert.deepEqual(['ja', 'zh', 'en'].map((l) => hasLang(q, l as 'ja')), [true, true, true]);
  const noEn = choice({ en: null });
  assert.equal(hasLang(noEn, 'en'), false);
  const noZh = choice({ zh: null });
  assert.equal(hasLang(noZh, 'zh'), false);
  assert.equal(hasLang(noZh, 'en'), true);
});

test('旧题库的题（根本没有 en / english 字段）：英语不可用，选了英语就退回日语，不报错', () => {
  const old = choice();
  delete (old as { en?: unknown }).en;
  delete (old as { english?: unknown }).english;
  assert.equal(hasLang(old, 'en'), false);
  assert.equal(effectiveLang(old, 'en'), 'ja');
  const v = shown(old, 'en');
  assert.equal(v.lang, 'ja');
  assert.equal(v.stem, '日本語の題干');
  assert.equal(v.optionText(1), '乙');
});

test('单选题：三种语言各取各的题干和选项', () => {
  const q = choice();
  const en = shown(q, 'en');
  assert.deepEqual([en.lang, en.stem, en.optionText(0), en.optionText(1)], ['en', 'English stem', 'Alpha', 'Beta']);
  const zh = shown(q, 'zh');
  assert.deepEqual([zh.lang, zh.stem, zh.optionText(1)], ['zh', '中文题干', '乙(中)']);
  const ja = shown(q, 'ja');
  assert.deepEqual([ja.lang, ja.stem, ja.optionText(0)], ['ja', '日本語の題干', '甲']);
});

test('选项文字缺一项时退回日语那一项，而不是显示 undefined', () => {
  const q = choice({ en: { stem: 'English stem', options: ['Alpha'] } });
  const v = shown(q, 'en');
  assert.equal(v.optionText(0), 'Alpha');
  assert.equal(v.optionText(1), '乙');
  assert.equal(v.optionText(9), '');
});

test('热点题：每种语言用自己那一列里写的选项 —— 日语はい / いいえ，中文 是 / 否，英语 Yes / No', () => {
  const q = hotspot();
  const en = shown(q, 'en');
  assert.equal(en.itemText(0), 'The sun rises in the east.');
  assert.deepEqual([en.choiceText(0, 0), en.choiceText(0, 1), en.choiceText(1, 1)], ['Yes', 'No', 'No']);
  const zh = shown(q, 'zh');
  assert.equal(zh.itemText(1), '月亮是恒星。');
  assert.deepEqual([zh.choiceText(0, 0), zh.choiceText(0, 1), zh.choiceText(1, 1)], ['是', '否', '否']);
  assert.deepEqual([en.choiceLang(0, 0), zh.choiceLang(0, 0), shown(q, 'ja').choiceLang(0, 0)], ['en', 'zh', 'ja']);
  const ja = shown(q, 'ja');
  assert.deepEqual([ja.itemText(0), ja.choiceText(0, 0)], ['太陽は東から昇る。', 'はい']);
});

test('这门语言没写热点选项时，选项退回日语原文，lang 标成 ja（文字是日语）', () => {
  const q = hotspot({ en: { stem: 'English stem', items: ['a', 'b'] }, zh: { stem: '中文题干', items: ['a', 'b'] } });
  for (const code of ['en', 'zh'] as const) {
    const v = shown(q, code);
    assert.equal(v.lang, code);
    assert.deepEqual([v.choiceText(0, 0), v.choiceText(1, 1)], ['はい', 'いいえ']);
    assert.equal(v.choiceLang(0, 0), 'ja');
  }
});

test('选的是英语、这道题没有英语：退回日语，整道题都是日语（不混语言）', () => {
  const q = choice({ en: null });
  const v = shown(q, 'en');
  assert.equal(v.lang, 'ja');
  assert.deepEqual([v.stem, v.optionText(0), v.optionText(1)], ['日本語の題干', '甲', '乙']);
});
