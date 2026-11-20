// v1.9 catalog contract: the new keys exist in en / zh / ja, the binding zh texts are exact, removed keys are gone.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { hasKey, setUiLang, t, type MsgKey } from './index.ts';

afterEach(() => setUiLang('en'));

const SUBS = ['caps', 'chat', 'study', 'summary', 'gen', 'import', 'server'];
const KEYS: string[] = [
  'ai.set.conn.statusOk', 'ai.set.conn.statusOff', 'ai.set.conn.enableAria', 'ai.set.conn.groupOn', 'ai.set.conn.groupOff',
  'ai.set.conn.groupEmpty', 'ai.set.conn.offConfirm', 'ai.set.conn.offConfirmBody', 'ai.set.conn.offConfirmGo',
  'ai.set.card.conn.someOff', 'ai.set.card.model.off', 'ai.set.slot.connOff', 'ai.set.slot.connOffWarn', 'ai.set.slot.hint',
  'ai.set.slot.explain', 'ai.set.slot.off', 'ai.set.slot.swAria', 'ai.set.models.note', 'ai.set.feat.adminAll',
  'ai.err.connectionOff', 'ai.err.connectionOffAdmin', 'ai.err.featureOffAdmin',
  'ai.model.pick.title', 'ai.model.api', 'ai.model.apiPick', 'ai.model.modelPick', 'ai.model.isDefault', 'ai.model.useDefault',
  'ai.model.none', 'ai.model.offOpt', 'ai.model.off', 'ai.model.label',
  'ai.set.safeTab.aria', ...SUBS.flatMap((s) => [`ai.set.safeTab.${s}`, `ai.set.safeTab.${s}Short`]),
  'ai.limits.group.running', 'ai.limits.group.pace', 'ai.limits.paceOn', 'ai.set.limit.paceRpm', 'ai.set.limit.paceTpm',
  'ai.limits.pace.note', 'ai.limits.pace.live', 'ai.limits.pace.offNote', 'ai.set.limit.resetSub', 'ai.set.gen.phase.waiting',
  'aigen.card.waiting', 'aigen.err.providerDaily',
  'banks.dl.body', 'banks.dl.langs', 'banks.dl.langsNone', 'banks.dl.noEnglish',
  'act.a.ai.settings.connectionOn', 'act.a.ai.settings.connectionOff',
];

test('every new key exists in en, zh and ja', () => {
  for (const lang of ['en', 'zh', 'ja'] as const) {
    setUiLang(lang);
    for (const k of KEYS) {
      const v = t(k as MsgKey, { name: 'x', n: 1, on: 1, list: 'x', conn: 'x', model: 'x', rpm: 1, tpm: 1, sec: 1 });
      assert.ok(v && v !== k, `${lang} ${k}`);
    }
  }
});

test('binding zh texts', () => {
  setUiLang('zh');
  assert.equal(t('ai.set.conn.statusOk'), '正常启用');
  assert.equal(t('ai.set.conn.statusOff'), '未启用');
  assert.equal(t('ai.set.conn.groupOn', { n: 2 }), '已启用的连接（2）');
  assert.equal(t('ai.set.conn.groupOff', { n: 0 }), '未启用的连接（0）');
  assert.equal(t('aigen.card.waiting'), '等待额度…');
  assert.equal(t('act.a.ai.settings.connectionOn'), '启用连接');
  assert.equal(t('act.a.ai.settings.connectionOff'), '停用连接');
  assert.equal(t('ai.set.slot.hint'), '提示');
  assert.equal(t('ai.set.slot.explain'), '解析');
});

test('binding en and ja texts of the connection chip and the download dialog', () => {
  assert.equal(t('ai.set.conn.statusOff'), 'Off');
  assert.equal(t('ai.set.conn.statusOk'), 'On · working');
  setUiLang('ja');
  assert.equal(t('ai.set.conn.statusOff'), '無効');
  assert.equal(t('banks.dl.body', { name: 'Garden' }), '「Garden」の言語と形式を選んでください。');
});

test('removed keys are gone', () => {
  for (const k of ['ai.set.slot.study', 'ai.set.switches.title', 'ai.set.switches.note']) assert.equal(hasKey('en', k), false, k);
  assert.equal(hasKey('en', 'ai.set.switches.globalOff'), true);
});
