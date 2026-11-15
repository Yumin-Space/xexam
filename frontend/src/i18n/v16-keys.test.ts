// v1.6 catalog contract: the thinking-strength keys exist in en / zh-CN / ja with the binding zh texts.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { setUiLang, t, type MsgKey } from './index.ts';

afterEach(() => setUiLang('en'));
const LEVELS = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'true', 'false'];
const KEYS = ['label', 'default', 'defaultTip', 'levelOf', 'modelDefault', ...LEVELS.map((l) => `level.${l}`)].map((k) => `ai.set.effort.${k}`);

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
  assert.equal(t('ai.set.effort.default'), '默认');
  assert.equal(t('ai.set.effort.modelDefault', { level: '中' }), '模型默认：中');
  assert.equal(t('act.a.ai.settings.effort' as MsgKey), '修改思考强度');
  assert.equal(t('ai.set.effort.level.true'), 'True（开启思考）');
  assert.equal(t('ai.set.effort.level.false'), 'False（关闭思考）');
  setUiLang('en');
  assert.equal(t('ai.set.effort.level.true'), 'True (thinking on)');
  setUiLang('ja');
  assert.equal(t('ai.set.effort.level.true'), 'True（思考オン）');
});
