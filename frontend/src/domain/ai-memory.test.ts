// AI Memory (v0.8): only an explicit request to remember something offers a note, in English, Chinese and Japanese; study
// questions that merely contain the word do not. Made-up sentences only. Run with `node --experimental-strip-types --test`.
import assert from 'node:assert/strict';
import test from 'node:test';
import { memoryBoxChars, memoryLength, memoryTokenCount, wantsMemory } from './ai-memory.ts';

test('explicit requests in en / zh / ja', () => {
  for (const s of [
    'Remember that I prefer short answers.',
    'please remember: my exam is in spring',
    '  Remember this  ',
    '记住我喜欢简短的回答。',
    '请记住我在准备考试。',
    '帮我记住这个日期',
    '请帮我记住：周三复习',
    '覚えておいて：短い答えが好きです。',
    '私は短い答えが好きだと覚えておいて',
    '試験は春だと覚えておいてください。',
    '朝に勉強することを覚えておいて！',
  ]) assert.equal(wantsMemory(s), true, s);
});

test('study questions that only mention remembering are not requests', () => {
  for (const s of [
    '这个公式怎么记住？',
    '覚えておくべきポイントは？',
    'How do I remember this formula?',
    'What should I remember about cells?',
    'Remembering dates is hard',
    '',
    '   ',
  ]) assert.equal(wantsMemory(s), false, JSON.stringify(s));
});

test('the length counts characters, not UTF-16 units', () => {
  assert.equal(memoryLength('abc'), 3);
  assert.equal(memoryLength('漢字'), 2);
  assert.equal(memoryLength('😀'), 1);
});

test('memoryTokenCount estimates tokens like the service', () => {
  assert.equal(memoryTokenCount('abcd'), 1);
  assert.equal(memoryTokenCount('abcde'), 2);
  assert.equal(memoryTokenCount('漢字'), 3);
  assert.equal(memoryBoxChars(1000), 4000);
});
