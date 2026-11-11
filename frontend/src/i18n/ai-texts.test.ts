// v0.8 texts that other texts or the spec depend on: the always-visible administrator notice in AI Chat (owner), the tips of
// "Select all" / "Deselect all" quoting the exact wording of "Include banks imported later" in each language, the limit
// numbers in AI error texts, an unknown AI code, and (round 2) the owner's exact Chinese words for the AI+ function, the
// badge reasons, the refusal of User management and free mode. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import test from 'node:test';
import ai from './catalog/ai.ts';
import users from './catalog/users.ts';
import { ApiError } from '../api/client.ts';
import { aiCodeText, errorText, knownAiCode } from './errors.ts';
import { setUiLang } from './index.ts';
import { localizeServerMessage } from './server-messages.ts';

test('the administrator notice of AI Chat says what administrators can see, in all three languages', () => {
  assert.equal(ai.en['ai.adminNotice'], 'Administrators can view and delete your AI chats and memories and see your AI usage.');
  assert.equal(ai.zh['ai.adminNotice'], '管理员可以查看和删除你的 AI 对话和记忆，也能看到你的 AI 用量。');
  assert.equal(ai.ja['ai.adminNotice'], '管理者はあなたの AI チャットとメモリーを閲覧・削除でき、AI の使用量も確認できます。');
});

test('the tips of Select all / Deselect all quote "Include banks imported later" exactly', () => {
  for (const lang of ['en', 'zh', 'ja'] as const) {
    const later = users[lang]['users.banks.later'];
    assert.ok(later);
    for (const key of ['users.tip.selectAll', 'users.tip.deselectAll'] as const) {
      const tip = users[lang][key];
      assert.ok(tip.includes(`“${later}”`) || tip.includes(`「${later}」`), `${lang} ${key}: ${tip}`);
    }
  }
  // the removed helper texts are gone
  for (const key of ['users.field.banksHint', 'users.field.manager', 'users.field.managerHint', 'users.field.managerAdmin', 'users.banks.body']) {
    assert.ok(!(key in users.en), key);
  }
});

test('AI limit texts carry the number of the answer, or the default', () => {
  try {
    assert.equal(aiCodeText('ai.tooLong', 20000), 'The message is too long (at most 20,000 characters).');
    assert.equal(aiCodeText('ai.tooLong'), 'The message is too long (at most 16,000 characters).');
    assert.equal(aiCodeText('ai.chatLimit'), 'You have too many chats (at most 1,000). Delete some old chats first.');
    assert.equal(aiCodeText('ai.memoryTooLong', 500), 'A memory can have at most 500 characters.');
    setUiLang('zh');
    assert.equal(aiCodeText('ai.memoryFull', 40), '记忆已满（最多 40 条），请先删除一条。');
    setUiLang('ja');
    assert.equal(aiCodeText('ai.tooLong', 20000), 'メッセージが長すぎます（最大 20,000 文字）。');
  } finally {
    setUiLang('en');
  }
});

test('codes: ai.* and auth.guard have texts; an unknown code falls back to the internal error', () => {
  assert.ok(knownAiCode('ai.up.timeout'));
  assert.ok(knownAiCode('ai.setup'));
  assert.ok(knownAiCode('auth.guard'));
  assert.ok(!knownAiCode('auth.signin'), 'the backend text is translated like any backend message');
  assert.ok(!knownAiCode('ai.somethingNew'));
  assert.equal(aiCodeText('ai.somethingNew'), 'AI had an internal error. Try again shortly.');
});

test('round 2: the exact Chinese words of the owner', () => {
  assert.equal(ai.zh['ai.badge.suspended'], 'AI+ 功能已被管理员暂停。');
  assert.equal(ai.zh['ai.badge.unavailable'], 'AI 暂时无法使用，请稍后再试或联系管理员。');
  assert.equal(ai.zh['ai.set.fn.suspend'], '暂停');
  assert.equal(ai.zh['ai.set.fn.resume'], '恢复');
  assert.equal(ai.zh['ai.set.fn.suspended'], '已暂停');
  assert.equal(ai.zh['ai.err.serviceOff'], '请先在 AI 设置里开启 AI+ 功能。');
  assert.equal(ai.zh['ai.err.disabled'], 'AI+ 功能暂时不可用。');
  assert.equal(ai.zh['ai.err.suspended'], '管理员已暂停你的 AI+ 功能。');
  assert.equal(ai.zh['ai.err.adminAlways'], '管理员始终可以使用 AI，不能暂停。');
  assert.equal(ai.en['ai.set.enable'], 'AI+ feature');
  // the replaced keys are gone
  for (const key of ['ai.badge.off', 'ai.badge.unreachable', 'ai.set.unreachable', 'ai.set.col.aiPlus']) assert.ok(!(key in ai.en), key);
  // no container is called by its old name any more (the pattern is built from parts, so the repository's leftover grep for
  // the old name does not find this line)
  const oldName = new RegExp(['xexam', 'ai(?!-service)'].join('-'), 'i');
  for (const lang of ['en', 'zh', 'ja'] as const) {
    for (const [key, text] of Object.entries(ai[lang])) assert.ok(!oldName.test(text), `${lang} ${key}`);
  }
});

test('round 2: the refusal of User management (409 ai.serviceOff) reads the same by code and by the backend text', () => {
  const off = new ApiError('http', 409, '请先在 AI 设置里开启 AI+ 功能。', null, { code: 'ai.serviceOff' });
  const textOnly = new ApiError('http', 409, '请先在 AI 设置里开启 AI+ 功能。');
  try {
    assert.equal(errorText(off), 'Turn on the AI+ feature in AI settings first.');
    assert.equal(errorText(textOnly), 'Turn on the AI+ feature in AI settings first.');
    assert.equal(localizeServerMessage('请先在 AI 设置里开启 AI+ 功能。', 'ja'), '先に AI 設定で AI+ 機能をオンにしてください。');
    setUiLang('zh');
    assert.equal(errorText(off), '请先在 AI 设置里开启 AI+ 功能。');
    setUiLang('ja');
    assert.equal(errorText(off), '先に AI 設定で AI+ 機能をオンにしてください。');
    assert.equal(aiCodeText('ai.suspended'), '管理者があなたの AI+ 機能を一時停止しました。');
  } finally {
    setUiLang('en');
  }
  assert.ok(knownAiCode('ai.serviceOff') && knownAiCode('ai.suspended') && knownAiCode('ai.adminAlways'));
});

test('AI Chat texts for users call it the AI+ feature, never the service or the container (UX review decisions 2 and 3)', () => {
  const keys = ['ai.readOnlyTail', 'ai.noAccess.suspended', 'ai.noAccess.disabled',
    'ai.noAccess.noFn', 'ai.err.notAllowed', 'ai.err.unavailable'] as const;
  for (const lang of ['en', 'zh', 'ja'] as const) {
    for (const k of keys) {
      const text = ai[lang][k];
      assert.ok(!/ai-service|AI\+ 服务|AI\+ service|AI\+ サービス|AI 服务|AI サービス/.test(text), `${lang} ${k}: ${text}`);
      assert.ok(!/停用|启用/.test(text), `${lang} ${k} must say pause, not 停用: ${text}`);
    }
  }
  // the pause words of the owner
  assert.ok(ai.zh['ai.noAccess.suspended'].includes('暂停'));
  assert.ok(ai.ja['ai.noAccess.suspended'].includes('一時停止'));
  assert.ok(ai.en['ai.noAccess.suspended'].includes('paused'));
  // administrators alone see the technical name
  assert.equal(ai.en['ai.err.unavailableAdmin'], 'ai-service is not connected right now.');
  assert.equal(ai.zh['ai.err.unavailableAdmin'], 'ai-service 未连接。');
  assert.equal(ai.ja['ai.err.unavailableAdmin'], 'ai-service に接続していません。');
  assert.equal(ai.zh['ai.err.unavailable'], 'AI 暂时无法使用，请稍后再试或联系管理员。');
  assert.equal(ai.en['ai.err.unavailable'], 'AI is not available right now. Try again later or ask an administrator.');
  assert.equal(ai.ja['ai.err.unavailable'], 'AI は今使えません。しばらくしてからもう一度試すか、管理者に連絡してください。');
});

test('the empty-chat hint names the reply language by its own name', () => {
  assert.equal(ai.en['ai.langName.zh'], '简体中文');
  assert.equal(ai.zh['ai.langName.ja'], '日本語');
  for (const lang of ['en', 'zh', 'ja'] as const) assert.ok(ai[lang]['ai.emptyChat'].includes('{lang}'));
});

test('owner decisions of 2026-11-12: pause / resume everywhere, one name, no container name for users', () => {
  const langs = ['en', 'zh', 'ja'] as const;
  assert.deepEqual([ai.en['ai.set.fn.suspend'], ai.en['ai.set.fn.resume'], ai.en['ai.set.fn.suspended']], ['Pause', 'Resume', 'Paused']);
  assert.deepEqual([ai.zh['ai.set.fn.suspend'], ai.zh['ai.set.fn.resume'], ai.zh['ai.set.fn.suspended']], ['暂停', '恢复', '已暂停']);
  assert.deepEqual([ai.ja['ai.set.fn.suspend'], ai.ja['ai.set.fn.resume'], ai.ja['ai.set.fn.suspended']], ['一時停止', '再開', '一時停止中']);
  const fnKeys = Object.keys(ai.en).filter((k) => k.startsWith('ai.set.fn.') || ['ai.badge.suspended', 'ai.err.suspended', 'ai.err.adminAlways', 'ai.readOnlySuspended', 'ai.noAccess.suspended'].includes(k));
  assert.ok(fnKeys.length >= 10);
  for (const key of fnKeys) {
    assert.ok(!/停用|启用|停止/.test(ai.zh[key as keyof typeof ai.zh] ?? ''), `zh ${key}`);
    assert.ok(!/(^|[^時])停止/.test(ai.ja[key as keyof typeof ai.ja] ?? ''), `ja ${key}`);
    assert.ok(!/suspend/i.test(ai.en[key as keyof typeof ai.en]), `en ${key}`);
  }
  // no text of the AI or User management catalogs says "AI+ service" in any language
  for (const cat of [ai, users]) {
    for (const lang of langs) {
      for (const [key, text] of Object.entries(cat[lang])) assert.ok(!/AI\+ (服务|service|サービス)/i.test(text), `${lang} ${key}`);
    }
  }
  assert.equal(ai.zh['ai.set.enabled'], 'AI+ 功能已开启。');
  assert.equal(ai.zh['ai.set.disabled'], 'AI+ 功能已关闭。');
  assert.equal(ai.en['ai.set.state.on'], 'AI+ feature on');
  assert.equal(ai.zh['ai.set.state.off'], 'AI+ 功能已关闭');
  assert.equal(ai.ja['ai.set.state.on'], 'AI+ 機能はオン');
  // only administrators see the container: every user text outside the settings page and the *Admin keys leaves it out
  const userKeys = Object.keys(ai.en).filter((k) => !k.startsWith('ai.set.') && !k.endsWith('Admin') && k !== 'ai.err.guard');
  for (const lang of langs) {
    for (const key of userKeys) assert.ok(!/ai-service/i.test(ai[lang][key as keyof typeof ai.en] ?? ''), `${lang} ${key}`);
    assert.ok(/ai-service/.test(ai[lang]['ai.set.svc.notConnected']));
  }
  assert.equal(ai.ja['ai.err.backendDown'], ai.ja['ai.err.unavailable']);
  assert.equal(ai.zh['ai.badge.unavailable'], ai.zh['ai.err.unavailable']);
  // the administrators' viewer says Delete like the users' own pages, and ja says メモリー throughout
  assert.equal(ai.en['ai.set.user.clearOk'], 'Delete');
  assert.equal(ai.zh['ai.set.user.clearOk'], '删除');
  assert.equal(ai.ja['ai.set.user.clearMemories'], 'すべてのメモリーを削除');
  assert.ok(!/メモ(?!リー)/.test(ai.ja['ai.set.user.noData'] + ai.ja['ai.set.user.clearMemories.ask']));
});

test('UX findings: saving turns the AI+ feature off and says what to do; test lines and keep-on name the feature', () => {
  assert.equal(ai.zh['ai.set.saveOffAsk'], '保存后，AI+ 功能会对所有人关闭。新设置测试成功后，请重新开启 AI+ 功能。要保存吗？');
  assert.equal(ai.en['ai.set.savedOff'], 'The AI settings were saved. The AI+ feature is now off for everyone: test the new settings, then turn the AI+ feature on again.');
  assert.equal(ai.en['ai.set.testOk'], 'The test worked ({ms} ms). You can turn on the AI+ feature now.');
  assert.equal(ai.zh['ai.err.notVerified'], '请先保存设置并测试成功，再开启 AI+ 功能。');
  assert.equal(ai.ja['ai.set.testKeepOn'], 'AI+ 機能はオンのままです。プロバイダー側の一時的な問題のようです。');
  assert.ok(ai.en['ai.set.keyProblem'].endsWith('then test and turn on the AI+ feature.'));
  for (const lang of ['en', 'zh', 'ja'] as const) {
    assert.ok(!/enable AI|开启 AI。|AI を有効/.test(ai[lang]['ai.set.testOk'] + ai[lang]['ai.err.notVerified'] + ai[lang]['ai.set.keyProblem']), lang);
  }
});
