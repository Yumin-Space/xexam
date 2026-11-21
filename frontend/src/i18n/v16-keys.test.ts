// v1.6 catalog contract: the thinking-strength keys exist in en / zh-CN / ja with the binding zh texts.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { setUiLang, t, type MsgKey } from './index.ts';

afterEach(() => setUiLang('en'));
const KEYS = ['label', 'levelOf', 'default', 'defaultTip'].map((k) => `ai.set.effort.${k}`);

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

test('Default option texts', () => {
  setUiLang('zh');
  assert.equal(t('ai.set.effort.default'), '默认（不发送强度参数）');
  assert.equal(t('ai.set.effort.defaultTip'), '默认（不发送强度参数，由模型自行决定）');
  setUiLang('ja');
  assert.equal(t('ai.set.effort.default'), '既定（強さのパラメーターを送らない）');
  setUiLang('en');
  assert.equal(t('ai.set.effort.default'), 'Default (no strength parameter sent)');
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
