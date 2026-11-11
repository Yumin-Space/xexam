import assert from 'node:assert/strict';
import test from 'node:test';
import { LANG_LABEL, SWITCH_ORDER, baseLang, effectiveLang, explainOf, hasLang, htmlLang, isLang, shown } from './language.ts';
import type { Question } from '../types/api.ts';

const base = {
  tag: '', translation: '', explanation: '', mnemonic: '', note: '', id: 'q', sheet: 'S', row: 1, bank: 1,
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

test('isLang only accepts ja / zh / en (any dirty value from storage is rejected)', () => {
  assert.equal(isLang('ja'), true);
  assert.equal(isLang('zh'), true);
  assert.equal(isLang('en'), true);
  for (const bad of ['', 'EN', 'fr', null, undefined, 1, {}]) assert.equal(isLang(bad), false);
});

test('Japanese is always available; Chinese / English need a matching structure to be available', () => {
  const q = choice();
  assert.deepEqual(['ja', 'zh', 'en'].map((l) => hasLang(q, l as 'ja')), [true, true, true]);
  const noEn = choice({ en: null });
  assert.equal(hasLang(noEn, 'en'), false);
  const noZh = choice({ zh: null });
  assert.equal(hasLang(noZh, 'zh'), false);
  assert.equal(hasLang(noZh, 'en'), true);
});

test('question from an old bank (no en / english field at all): English is unavailable, and choosing English falls back to Japanese without error', () => {
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

test('single-choice question: each of the three languages uses its own stem and options', () => {
  const q = choice();
  const en = shown(q, 'en');
  assert.deepEqual([en.lang, en.stem, en.optionText(0), en.optionText(1)], ['en', 'English stem', 'Alpha', 'Beta']);
  const zh = shown(q, 'zh');
  assert.deepEqual([zh.lang, zh.stem, zh.optionText(1)], ['zh', '中文题干', '乙(中)']);
  const ja = shown(q, 'ja');
  assert.deepEqual([ja.lang, ja.stem, ja.optionText(0)], ['ja', '日本語の題干', '甲']);
});

test('when one option text is missing, fall back to the Japanese one instead of showing undefined', () => {
  const q = choice({ en: { stem: 'English stem', options: ['Alpha'] } });
  const v = shown(q, 'en');
  assert.equal(v.optionText(0), 'Alpha');
  assert.equal(v.optionText(1), '乙');
  assert.equal(v.optionText(9), '');
});

test('hotspot question: each language uses the choices written in its own column - Japanese はい / いいえ, Chinese 是 / 否, English Yes / No', () => {
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

test('when this language has no hotspot choices written, choices fall back to the Japanese original and lang is set to ja (the text is Japanese)', () => {
  const q = hotspot({ en: { stem: 'English stem', items: ['a', 'b'] }, zh: { stem: '中文题干', items: ['a', 'b'] } });
  for (const code of ['en', 'zh'] as const) {
    const v = shown(q, code);
    assert.equal(v.lang, code);
    assert.deepEqual([v.choiceText(0, 0), v.choiceText(1, 1)], ['はい', 'いいえ']);
    assert.equal(v.choiceLang(0, 0), 'ja');
  }
});

test('English is chosen but this question has no English: fall back to Japanese, the whole question is Japanese (no mixed languages)', () => {
  const q = choice({ en: null });
  const v = shown(q, 'en');
  assert.equal(v.lang, 'ja');
  assert.deepEqual([v.stem, v.optionText(0), v.optionText(1)], ['日本語の題干', '甲', '乙']);
});

// ---------------------------------------------------------------- six-column questions (v0.6.5): English is the base

/** A made-up six-column choice question: English base text, Chinese in zh, Japanese in ja, an explanation per language */
const six = (extra: Partial<Question> = {}): Question => ({
  ...base,
  kind: 'multi',
  stem: 'Which two planets are gas giants?',
  options: [{ key: 'A', text: 'Mars' }, { key: 'B', text: 'Jupiter' }, { key: 'C', text: 'Saturn' }],
  items: [],
  answer: ['B', 'C'],
  translation: '哪两颗行星是气态巨行星？',
  zh: { stem: '哪两颗行星是气态巨行星？', options: ['火星', '木星', '土星'] },
  lang: 'en',
  ja: { stem: 'ガス惑星はどの 2 つですか？', options: ['火星', '木星', '土星'] },
  explanation: '木星和土星主要由氢和氦组成。',
  mnemonic: '木土是气球',
  note: '',
  explain: {
    en: { explanation: 'Jupiter and Saturn are mostly hydrogen and helium.', mnemonic: 'J and S are balloons', note: 'sure' },
    zh: { explanation: '木星和土星主要由氢和氦组成。', mnemonic: '木土是气球', note: '' },
    ja: { explanation: '木星と土星は主に水素とヘリウムです。', mnemonic: '', note: '' },
  },
  ...extra,
});

test('six-column question: English is the base language, Japanese comes from q.ja, Chinese from q.zh', () => {
  const q = six();
  assert.equal(baseLang(q), 'en');
  assert.equal(baseLang(choice()), 'ja');
  assert.deepEqual(['en', 'zh', 'ja'].map((l) => hasLang(q, l as 'ja')), [true, true, true]);
  const en = shown(q, 'en');
  assert.deepEqual([en.lang, en.stem, en.optionText(1)], ['en', 'Which two planets are gas giants?', 'Jupiter']);
  const ja = shown(q, 'ja');
  assert.deepEqual([ja.lang, ja.stem, ja.optionText(2)], ['ja', 'ガス惑星はどの 2 つですか？', '土星']);
  const zh = shown(q, 'zh');
  assert.deepEqual([zh.lang, zh.stem, zh.optionText(0)], ['zh', '哪两颗行星是气态巨行星？', '火星']);
});

test('six-column question without a usable Japanese: Japanese is unavailable and falls back to the English base, never to q.en', () => {
  const q = six({ ja: null, en: { stem: 'stray', options: ['x', 'y', 'z'] } });
  assert.equal(hasLang(q, 'ja'), false);
  assert.equal(effectiveLang(q, 'ja'), 'en');
  const v = shown(q, 'ja');
  assert.deepEqual([v.lang, v.stem, v.optionText(0)], ['en', 'Which two planets are gas giants?', 'Mars']);
  // English is the base: a stray `en` object is never used for it
  assert.equal(shown(q, 'en').stem, 'Which two planets are gas giants?');
  const noZh = six({ zh: null });
  assert.equal(hasLang(noZh, 'zh'), false);
  assert.equal(shown(noZh, 'zh').lang, 'en');
});

test('six-column hotspot: choices fall back to the English base and say so in choiceLang', () => {
  const q: Question = {
    ...six(), kind: 'hotspot', options: [], answer: [0, 1],
    items: [{ text: 'Water boils at 100 °C at sea level.', choices: ['Yes', 'No'] }, { text: 'Ice sinks in water.', choices: ['Yes', 'No'] }],
    zh: { stem: '判断下列说法', items: ['海平面上水在 100 °C 沸腾。', '冰会沉入水中。'], choices: [['是', '否'], ['是', '否']] },
    ja: { stem: '次の記述を判定', items: ['海面で水は 100 °C で沸騰する。', '氷は水に沈む。'] },
  };
  const ja = shown(q, 'ja');
  assert.deepEqual([ja.itemText(1), ja.choiceText(1, 1), ja.choiceLang(1, 1)], ['氷は水に沈む。', 'No', 'en']);
  const zh = shown(q, 'zh');
  assert.deepEqual([zh.choiceText(0, 0), zh.choiceLang(0, 0)], ['是', 'zh']);
});

test('explainOf: six-column questions show the explanation of the chosen language, with its lang', () => {
  const q = six();
  const en = explainOf(q, 'en');
  assert.deepEqual(en.explanation, { text: 'Jupiter and Saturn are mostly hydrogen and helium.', lang: 'en' });
  assert.deepEqual(en.mnemonic, { text: 'J and S are balloons', lang: 'en' });
  assert.deepEqual(en.note, { text: 'sure', lang: 'en' });
  const ja = explainOf(q, 'ja');
  assert.deepEqual(ja.explanation, { text: '木星と土星は主に水素とヘリウムです。', lang: 'ja' });
  // Japanese has no memory aid: the Chinese one is shown (compatibility order zh, ja, en), marked as Chinese
  assert.deepEqual(ja.mnemonic, { text: '木土是气球', lang: 'zh' });
  // Only English has a note: every language shows it, marked as English
  assert.deepEqual(explainOf(q, 'zh').note, { text: 'sure', lang: 'en' });
});

test('explainOf: a language left out of explain falls back field by field; an empty explain shows the compatibility text', () => {
  const q = six({ explain: { en: { explanation: 'Only English.', mnemonic: '', note: '' } } });
  assert.deepEqual(explainOf(q, 'zh').explanation, { text: 'Only English.', lang: 'en' });
  const empty = six({ explain: {}, explanation: '', mnemonic: '', note: '' });
  assert.deepEqual(explainOf(empty, 'ja').explanation, { text: '', lang: null });
});

test('explainOf: 3- / 4-column questions show their single explanation for every language, without a lang', () => {
  const q = choice({ explanation: '因为乙是对的。', mnemonic: '选乙', note: '把握大' });
  for (const l of ['en', 'zh', 'ja'] as const) {
    const v = explainOf(q, l);
    assert.deepEqual([v.explanation, v.mnemonic, v.note], [
      { text: '因为乙是对的。', lang: null }, { text: '选乙', lang: null }, { text: '把握大', lang: null },
    ]);
  }
});

test('htmlLang and the switch labels: Chinese is zh-CN; every language is named in its own script', () => {
  assert.deepEqual(SWITCH_ORDER.map(htmlLang), ['en', 'zh-CN', 'ja']);
  assert.deepEqual(SWITCH_ORDER.map((l) => LANG_LABEL[l]), ['English', '中文', '日本語']);
});
