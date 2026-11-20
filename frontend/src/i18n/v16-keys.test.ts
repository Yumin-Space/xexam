// v1.6 catalog contract: the thinking-strength keys exist in en / zh-CN / ja with the binding zh texts.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { setUiLang, t, type MsgKey } from './index.ts';

afterEach(() => setUiLang('en'));
const KEYS = ['label', 'levelOf', 'modelDefault'].map((k) => `ai.set.effort.${k}`);

test('thinking-strength keys exist in every language', () => {
  for (const lang of ['en', 'zh', 'ja'] as const) {
    setUiLang(lang);
    for (const k of [...KEYS, 'act.a.ai.settings.effort']) {
      const v = t(k as MsgKey, { slot: 'x', level: 'y' });
      assert.ok(v && v !== k, `${lang} ${k}`);
    }
  }
});

test('binding zh texts', () => {
  setUiLang('zh');
  assert.equal(t('ai.set.effort.label'), '思考强度');
  assert.equal(t('act.a.ai.settings.effort' as MsgKey), '修改思考强度');
});

test('known-default hint shows the raw English level in every language', () => {
  setUiLang('zh');
  assert.equal(t('ai.set.effort.modelDefault', { level: 'HIGH' }), '模型默认：HIGH');
  setUiLang('ja');
  assert.equal(t('ai.set.effort.modelDefault', { level: 'xhigh' }), 'モデルの既定：xhigh');
  setUiLang('en');
  assert.equal(t('ai.set.effort.modelDefault', { level: 'low' }), 'Model default: low');
});

test('translated level labels and Gemma hints are gone from every catalog', () => {
  for (const lang of ['en', 'zh', 'ja'] as const) {
    setUiLang(lang);
    for (const l of ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'gemma.HIGH', 'gemma.MINIMAL']) {
      const k = `ai.set.effort.level.${l}`;
      assert.equal(t(k as MsgKey), k, `${lang} ${k}`);
    }
  }
});
