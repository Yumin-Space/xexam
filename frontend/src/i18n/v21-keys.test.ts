// v2.1 catalog contract (AI settings and the chat strength picker): the new keys exist in en / zh / ja.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { setUiLang, t, type MsgKey } from './index.ts';

afterEach(() => setUiLang('en'));

const KEYS = [
  'ai.set.slot.paused', 'ai.set.conn.family', 'ai.set.conn.familyHelp', 'ai.set.conn.familyOf',
  'ai.model.pick.strength', 'ai.model.pick.defaultTag', 'ai.model.pick.strengthAria',
];

test('every v2.1 AI settings key exists in en, zh and ja', () => {
  for (const lang of ['en', 'zh', 'ja'] as const) {
    setUiLang(lang);
    for (const k of KEYS) {
      const v = t(k as MsgKey, { name: 'x', param: 'p' });
      assert.ok(v && v !== k, `${lang} ${k}`);
    }
  }
});

test('binding zh texts', () => {
  setUiLang('zh');
  assert.equal(t('ai.set.conn.family'), '思考强度参数');
  assert.equal(t('ai.model.pick.defaultTag'), '模型默认');
  assert.equal(t('ai.model.pick.strength', { param: 'reasoning_effort' }), '思考强度 · reasoning_effort');
});

test('parameters and names are interpolated unchanged', () => {
  for (const lang of ['en', 'zh', 'ja'] as const) {
    setUiLang(lang);
    assert.ok(t('ai.set.conn.familyOf', { name: 'Anthropic' }).includes('Anthropic'), lang);
    assert.ok(t('ai.model.pick.strengthAria', { param: 'thinking_level' }).includes('thinking_level'), lang);
  }
});

test('v2.1 limit wording: caps / calls / pacing texts say what counts and use the figures as entered', () => {
  for (const lang of ['en', 'zh', 'ja'] as const) {
    setUiLang(lang);
    const note = t('ai.limits.pace.note' as MsgKey);
    assert.ok(note.includes('13') && note.includes('240,000'), `${lang} defaults`);
    assert.ok(!/90|10 ?%|15|250,000/.test(note), `${lang} no hidden margin`);
    assert.ok(t('ai.set.caps.hint' as MsgKey).length > 60, `${lang} caps hint`);
    for (const k of ['ai.set.limit.genMaxCallsHint', 'ai.set.limit.recognizeMaxCallsHint', 'ai.set.limit.importMaxCallsHint']) {
      assert.ok(t(k as MsgKey).includes('8'), `${lang} ${k}`);
    }
    const live = t('ai.limits.pace.live' as MsgKey, { rpm: '13', tpm: '240,000', sec: '4.6' });
    assert.ok(live.includes('13') && live.includes('4.6'), `${lang} live`);
  }
});
