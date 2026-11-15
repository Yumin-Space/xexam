// v1.3 frozen catalog contract: every key exists in en / zh / ja, the binding zh texts are exact, every action code of the
// activity log has a label, and the new error codes have texts. Run with `npm test`.
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { ApiError } from '../api/client.ts';
import { aiCodeText, errorText, genCodeText, knownAiCode } from './errors.ts';
import { hasKey, setUiLang, t, type MsgKey } from './index.ts';

afterEach(() => setUiLang('en'));

const GROUPS = ['auth', 'users', 'banks', 'study', 'settings', 'visit', 'ai', 'system'];
const SLOTS = ['default', 'chat', 'study', 'summary', 'generate', 'import'];
const ACTIONS = [
  'auth.signin', 'auth.signout', 'auth.expired', 'auth.password', 'auth.lock', 'users.create', 'users.update', 'users.password',
  'users.unlock', 'users.delete', 'users.signinInfo', 'users.signinInfoClear', 'bank.upload', 'bank.import', 'bank.importCancel', 'bank.delete',
  'bank.sheetsDelete', 'study.submit', 'records.delete', 'progress.reset', 'settings.update', 'settings.reset', 'logs.settings',
  'system.shutdown', 'page.view', 'admin.request', 'system.start', 'system.stop', 'system.migrate', 'system.guard',
  'system.aiLink', 'system.logPurge', 'system.logDropped', 'system.signinFlood', 'system.sessionsPurged',
  'ai.settings.connectionAdd', 'ai.settings.connectionUpdate', 'ai.settings.connectionRemove', 'ai.settings.connectionTest',
  'ai.settings.connectionOrder', 'ai.settings.key', 'ai.settings.keyReuse', 'ai.settings.model', 'ai.settings.modelTest',
  'ai.settings.modelRemove', 'ai.settings.enable', 'ai.settings.disable', 'ai.settings.features', 'ai.settings.limits',
  'ai.settings.retention', 'ai.users.features', 'ai.admin.viewChats', 'ai.admin.clear', 'ai.gen.create', 'ai.import.create',
  'ai.system.connection', 'ai.system.paused', 'ai.system.resumed',
  'users.geoSource', 'ai.geo.test', 'ai.geo.save', 'ai.geo.enable', 'ai.geo.delete',
];
const REASONS = ['idle', 'hard', 'badPassword', 'disabled', 'locked', 'changed', 'unknownUser'];

const KEYS: string[] = [
  'auth.remember', 'auth.remember.hint', 'auth.expired', 'auth.hardSoon',
  'users.fact.lastActive', 'users.fact.neverActive', 'users.online', 'users.sort', 'users.sort.name', 'users.sort.login',
  'users.sort.active', 'users.signinInfo', 'users.signinInfo.hidden', 'users.signinInfo.title', 'users.signinInfo.desc', 'users.signinInfo.state.label', 'users.signinInfo.state.on', 'users.signinInfo.state.off', 'users.signinInfo.clear.tip', 'users.signinInfo.clear', 'users.signinInfo.clear.name', 'users.signinInfo.clear.ask',
  'users.signinInfo.clear.ok', 'users.signinInfo.clear.done', 'users.fact.loginIp',
  'users.fact.loginPlace', 'users.fact.noPlace', 'users.fact.placeUnknown', 'users.fact.records',
  'logs.tab.activity', 'logs.tab.service', 'logs.lead',
  'act.col.time', 'act.col.user', 'act.col.action', 'act.col.object', 'act.col.result', 'act.col.ip', 'act.col.device',
  'act.filter.user', 'act.filter.user.all', 'act.filter.user.system', 'act.filter.user.anon', 'act.filter.action',
  'act.filter.action.all', ...GROUPS.map((g) => `act.group.${g}`), 'act.filter.result.all', 'act.filter.result.ok',
  'act.filter.result.fail', 'act.result.ok', 'act.result.fail', 'act.refresh', 'act.pause', 'act.resume', 'act.more',
  'act.empty', 'act.noMatch', 'act.readError', 'act.merged', 'act.deleted', 'act.actor.system', 'act.actor.anon',
  'act.settings', 'act.settings.days', 'act.settings.days.hint', 'act.settings.visits', 'act.settings.visits.hint',
  'act.settings.saved', 'act.note', 'act.ipHint', 'act.ipOff', 'act.detail.title', 'act.detail.ua', 'act.detail.place', 'act.detail.ip',
  ...ACTIONS.map((a) => `act.a.${a}`), ...REASONS.map((r) => `act.reason.${r}`),
  'ai.regenerate', 'ai.edit', 'ai.version.prev', 'ai.version.next', 'ai.version.of', 'ai.edit.banner', 'ai.edit.cancel',
  'ai.branch.moved', 'ai.model.label', 'ai.model.chosen', 'ai.model.default', 'ai.model.fallback', 'ai.set.viewer.branch',
  'ai.panel.shared.hint', 'ai.panel.shared.explain', 'ai.panel.regenerate.tip',
  ...['calls', 'chat', 'study', 'summary', 'gen', 'import', 'server'].map((g) => `ai.limits.group.${g}`),
  'ai.limits.callsAdminsExempt', 'ai.limits.callsAdminsExempt.note', 'ai.limits.memoryHint', 'ai.limits.replyNote',
  'ai.limits.studyShare', 'ai.limits.studyShareDays', 'ai.limits.studyShareClear', 'ai.limits.studyShareClear.ask',
  'ai.limits.studyShareClear.done', 'ai.limits.summaryMaxItems', 'ai.limits.genMaxChunks', 'ai.limits.dailyTokenBudget',
  'ai.limits.dailyTokenBudget.note', 'ai.usage.budget', 'ai.usage.shared',
  'ai.usage.col.connection',
  'ai.set.tab.conn', 'ai.set.card.main', 'ai.set.card.conn', 'ai.set.card.model', 'ai.set.card.features',
  'ai.set.card.main.on', 'ai.set.card.main.off', 'ai.set.card.main.setup', 'ai.set.card.main.pausedNoRoute',
  'ai.set.card.main.pausedKey', 'ai.set.card.conn.allOk', 'ai.set.card.conn.someBad', 'ai.set.card.conn.none',
  'ai.set.card.model.value', 'ai.set.card.model.none', 'ai.set.card.model.bad', 'ai.set.card.features.allOn',
  'ai.set.card.features.someOff', 'ai.set.conn.name', 'ai.set.conn.provider', 'ai.set.conn.host', 'ai.set.conn.keySaved',
  'ai.set.conn.statusOk', 'ai.set.conn.statusBad', 'ai.set.conn.usedBy', 'ai.set.conn.edit', 'ai.set.conn.retest',
  'ai.set.conn.delete', 'ai.set.conn.add', 'ai.set.conn.addFull', 'ai.set.conn.reuseFrom', 'ai.set.conn.reuseNone',
  'ai.set.conn.save', 'ai.set.conn.testing', 'ai.set.conn.ok', 'ai.set.conn.okNoList', 'ai.set.conn.fail',
  'ai.set.conn.rotated', 'ai.set.conn.rotateFail', 'ai.set.conn.httpNote', 'ai.set.reuse.title', 'ai.set.reuse.body',
  'ai.set.reuse.ok', 'ai.set.del.title', 'ai.set.del.usedBy', 'ai.set.del.newDefault', 'ai.set.del.noneLeft',
  'ai.set.enable.first', 'ai.set.enable.needConn', 'ai.set.enable.on', ...SLOTS.map((s) => `ai.set.slot.${s}`),
  'ai.set.slot.connOf', 'ai.set.slot.modelOf', 'ai.set.slot.pickConn', 'ai.set.slot.pickModel', 'ai.set.slot.connBad',
  'ai.set.slot.follow', 'ai.set.slot.manual', 'ai.set.slot.testing', 'ai.set.slot.switched', 'ai.set.slot.failed',
  'ai.set.slot.fallback', 'ai.set.slot.noRoute', 'ai.set.models.refresh', 'ai.set.switches.title', 'ai.set.switches.note',
  'ai.set.switches.globalOff', 'ai.err.timeout90',
  'ai.err.branchMoved', 'ai.err.badModel', 'ai.err.nameTaken', 'ai.err.badName', 'ai.err.conflict', 'ai.err.connectionLimit',
  'ai.err.connectionInUse', 'ai.err.defaultNeeded', 'ai.err.modelInUse', 'ai.err.modelMissing', 'ai.err.keyMissing',
  'ai.err.budget', 'ai.err.up.timeout',
  'aigen.phase.planning', 'aigen.items.title', 'aigen.items.note', 'aigen.placeholder', 'aigen.tooBig',
  'result.unsaved', 'result.unsaved.retry', 'result.unsaved.saving', 'result.unsaved.done',
  'app.fatal.retry', 'app.fatal.retrying', 'err.tooLarge', 'err.backendDown',
];

test('every v1.3 key of the contract exists in en, zh and ja', () => {
  const missing: string[] = [];
  for (const k of KEYS) for (const l of ['en', 'zh', 'ja'] as const) if (!hasKey(l, k)) missing.push(`${l}:${k}`);
  assert.deepEqual(missing, []);
  assert.equal(new Set(KEYS).size, KEYS.length, 'the list above has a duplicate');
});

test('binding zh texts of the contract are exact', () => {
  setUiLang('zh');
  const want: Array<[string, string]> = [
    ['auth.remember', '保持登录'], ['auth.expired', '登录已过期，请重新登录。'], ['users.online', '在线'],
    ['users.fact.neverActive', '从未活动'], ['users.sort.active', '最后活动'], ['users.fact.loginIp', '最后登录 IP '], ['users.fact.loginPlace', '登录所在地 '],
    ['users.fact.placeUnknown', '未知'], ['users.fact.lastActive', '最后活动 {at}'], ['users.fact.records', '练习和考试记录 {n} 次'], ['users.signinInfo', '登录 IP 和所在地：显示中。点击隐藏；隐藏后不再记录新的，已保存的记录会保留。'], ['users.signinInfo.hidden', '登录 IP 和所在地：已隐藏。点击重新显示，并恢复记录。'], ['users.signinInfo.clear', '清除记录'], ['users.signinInfo.clear.name', '清除已保存的登录 IP 和所在地记录'],
    ['act.a.users.signinInfoClear', '清除登录 IP 和所在地记录'], ['logs.tab.activity', '活动记录'],
    ['logs.tab.service', '运行日志'], ['act.col.ip', 'IP / 所在地 / 设备'], ['act.merged', '×{n}'], ['act.a.auth.signin', '登录'],
    ['act.a.ai.settings.keyReuse', '密钥用于新地址'], ['ai.regenerate', '重新生成'], ['ai.version.of', '第 {n} 个版本，共 {total} 个'],
    ['ai.set.slot.study', '提示 + 解析'], ['ai.feature.study', 'AI 提示+解析'], ['ai.feature.study.short', '提示+解析'],
    ['aiplus.explain', 'AI 解析'], ['aiplus.analyze', 'AI 解析'], ['ai.set.usage.purpose.hint', 'AI 提示'], ['ai.set.usage.purpose.explain', 'AI 解析'],
    ['ai.set.slot.noRoute', '这个功能现在没有可用的模型，已暂停。'],
    ['ai.set.card.main.pausedNoRoute', '已暂停：没有可用的模型'], ['ai.set.reuse.ok', '确认使用'], ['aigen.placeholder', '生成中 {made} / {goal}'],
    ['ai.limits.callsAdminsExempt', '管理员不受 AI 请求次数限制'], ['result.unsaved.retry', '重试保存'],
    ['ai.set.switches.note', '关闭后对所有普通用户生效，管理员不受影响；每位用户的单独开关在“用户”页。'],
  ];
  for (const [k, zh] of want) assert.equal(t(k as MsgKey), zh, k);
});

test('the sign-in info and global switch texts say what the owner decided (en)', () => {
  assert.match(t('ai.set.switches.note'), /administrators are not affected/i);
  assert.equal(t('users.signinInfo.title'), 'Sign-in IP and location');
  assert.match(t('users.signinInfo.desc'), /sign-in and last activity\. The location source is set below\./);
  assert.equal(t('users.signinInfo.state.label'), 'Status');
  assert.equal(t('users.signinInfo.state.on'), 'On');
  assert.equal(t('users.signinInfo.state.off'), 'Off');
  assert.match(t('users.signinInfo'), /Click to hide; while hidden, no new ones are recorded and saved records are kept/);
  assert.match(t('users.signinInfo.hidden'), /Click to show them again and resume recording/);
  assert.match(t('users.signinInfo.clear.tip'), /cannot be undone/);
  assert.equal(hasKey('en', 'users.signinInfo.hint'), false);
  assert.equal(t('users.signinInfo.clear'), 'Clear records');
  assert.match(t('users.signinInfo.clear.ask'), /cannot be undone/);
  assert.doesNotMatch(t('users.signinInfo.clear.ask') + t('users.signinInfo.clear.done'), /delete/i);
  assert.match(t('act.ipOff'), /hidden/);
  assert.match(t('act.note'), /recorded only while the eye in User management is open; while it is hidden nothing new is recorded and saved records are kept/);
  assert.doesNotMatch(t('act.note'), /recorded only after/);
  assert.match(t('ai.set.conn.httpNote'), /not encrypted/);
});

test('the sign-in info eye stops recording, keeps what is saved, and the clear cannot be undone, in zh and ja too', () => {
  setUiLang('zh');
  assert.equal(t('users.signinInfo.title'), '登录 IP 和所在地');
  assert.equal(t('users.signinInfo.desc'), '记录每位用户登录和最后活动时的 IP 地址和大致所在地。所在地来源见下方设置。');
  assert.equal(t('users.signinInfo.state.label'), '当前状态');
  assert.equal(t('users.signinInfo.state.on'), '开启');
  assert.equal(t('users.signinInfo.state.off'), '关闭');
  assert.equal(t('users.signinInfo').includes('点击隐藏；隐藏后不再记录新的，已保存的记录会保留。'), true);
  assert.equal(t('users.signinInfo.hidden'), '登录 IP 和所在地：已隐藏。点击重新显示，并恢复记录。');
  assert.equal(t('users.signinInfo.clear.tip'), '清除所有已保存的登录 IP 和所在地，无法撤销。');
  assert.match(t('users.signinInfo.clear.ask'), /无法撤销/);
  assert.equal(t('users.signinInfo.clear.ask').includes('删除'), false);
  assert.equal(t('act.ipOff').includes('未记录'), false);
  assert.equal(t('act.note').includes('IP 和大致所在地只在“用户管理”里的眼睛打开时才记录；隐藏期间不再记录新的内容，已保存的记录会保留。'), true);
  setUiLang('ja');
  assert.equal(t('users.signinInfo.clear'), '記録を消去');
  assert.match(t('users.signinInfo.clear.ask'), /元に戻せません/);
  assert.equal(t('act.note').includes('「ユーザー管理」の目が開いている間だけ記録されます。非表示の間は新しい記録は作られず、保存済みの記録は残ります。'), true);
  assert.equal(t('users.signinInfo.title'), 'ログイン IP と所在地');
  assert.equal(t('users.signinInfo.desc').includes('所在地の取得元は下の設定で選びます。'), true);
  assert.equal(t('users.signinInfo.state.label'), '現在の状態');
  assert.equal(t('users.signinInfo.state.on'), 'オン');
  assert.equal(t('users.signinInfo.state.off'), 'オフ');
  assert.equal(t('users.signinInfo.clear.tip'), '保存済みのログイン IP と所在地をすべて消去します。元に戻せません。');
  assert.equal(t('act.note').includes('常に記録'), false);
  for (const l of ['en', 'zh', 'ja'] as const) {
    assert.equal(hasKey(l, 'users.signinInfo.offAsk'), false, l);
    assert.equal(hasKey(l, 'users.meta.recordsShort'), false, l);
    assert.equal(hasKey(l, 'users.signinInfo.hint'), false, l);
  }
});

test('logs.lead names the 2-second read in all languages', () => {
  for (const l of ['en', 'zh', 'ja'] as const) {
    setUiLang(l);
    assert.ok(t('logs.lead').includes('2'), l);
    assert.ok(!t('logs.lead').includes('0.5'), l);
  }
});

test('every action code of the activity log has a label of its own in all languages', () => {
  for (const l of ['en', 'zh', 'ja'] as const) {
    setUiLang(l);
    for (const a of ACTIONS) assert.notEqual(t(`act.a.${a}` as MsgKey), `act.a.${a}`, `${l} ${a}`);
  }
});

test('the new error codes have texts (ApiError -> errorText)', () => {
  setUiLang('en');
  for (const c of ['ai.branchMoved', 'ai.badModel', 'ai.nameTaken', 'ai.badName', 'ai.conflict', 'ai.connectionInUse', 'ai.defaultNeeded',
    'ai.modelInUse', 'ai.modelMissing', 'ai.keyMissing', 'ai.budget', 'ai.up.timeout']) {
    assert.ok(knownAiCode(c), c);
    assert.notEqual(aiCodeText(c), aiCodeText('ai.nothingLikeThis'), c);
  }
  assert.equal(errorText(new ApiError('http', 409, '', null, { code: 'ai.connectionLimit', limit: 8 })), 'At most 8 connections can be added');
  assert.equal(errorText(new ApiError('http', 401, '', null, { code: 'auth.expired' })), t('auth.expired'));
  assert.equal(errorText(new ApiError('http', 413, '', null, { code: 'net.tooLarge' })), t('err.tooLarge'));
  assert.equal(errorText(new ApiError('http', 502, '', null, { code: 'net.backendDown' })), t('err.backendDown'));
});

test('ai.gen.tooBig carries the size of the bank', () => {
  setUiLang('en');
  assert.match(genCodeText('ai.gen.tooBig', { tokens: 1234567 }) ?? '', /about 1,234,567 tokens/);
  setUiLang('zh');
  assert.match(genCodeText('ai.gen.tooBig', { tokens: 5000 }) ?? '', /5,000/);
});

test('v1.3: one name for the analysis and one label for the hint + analysis switch, in every language', () => {
  const want: Record<'en' | 'zh' | 'ja', [string, string, string]> = {
    en: ['AI analysis', 'AI hint + analysis', 'Hint + analysis'],
    zh: ['AI 解析', 'AI 提示+解析', '提示 + 解析'],
    ja: ['AI 解説', 'AI ヒント+解説', 'ヒント + 解説'],
  };
  for (const lang of ['en', 'zh', 'ja'] as const) {
    setUiLang(lang);
    const [name, pair, slot] = want[lang];
    for (const k of ['aiplus.explain', 'aiplus.analyze', 'ai.feature.explain', 'ai.set.usage.purpose.explain']) assert.equal(t(k as MsgKey), name, `${lang} ${k}`);
    assert.equal(t('ai.feature.study' as MsgKey), pair, lang);
    assert.equal(t('ai.set.slot.study' as MsgKey), slot, lang);
    // the caps tip names the analysis once
    assert.equal(t('ai.set.caps.tip' as MsgKey).split(name).length - 1, 1, lang);
  }
});
