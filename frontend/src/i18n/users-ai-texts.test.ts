// The AI+ words of User management and of the dark badge (v0.8 UX review, owner's decisions): the per-user suspend is
// PAUSE / RESUME, the global switch and the per-user grant are the "AI+ feature", and a user who is not an administrator
// never reads the container's name. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import test from 'node:test';
import ai from './catalog/ai.ts';
import users from './catalog/users.ts';
import { AI_BADGE_TEXT, badgeReasonText } from '../components/ai-badge.ts';
import { setUiLang } from './index.ts';

const LANGS = ['en', 'zh', 'ja'] as const;

test('a paused AI+ function reads "Paused" / 已暂停 / 一時停止中 and never uses the account words', () => {
  assert.equal(users.en['users.ai.paused'], 'Paused');
  assert.equal(users.zh['users.ai.paused'], '已暂停');
  assert.equal(users.ja['users.ai.paused'], '一時停止中');
  assert.match(ai.zh['ai.badge.suspended'], /暂停/);
  assert.match(ai.ja['ai.badge.suspended'], /一時停止/);
  assert.match(ai.en['ai.badge.suspended'], /paused/);
  assert.doesNotMatch(ai.zh['ai.badge.suspended'], /停用|启用|停止/);
  assert.doesNotMatch(ai.ja['ai.badge.suspended'], /(^|[^時])停止/);
});

test('User management calls the grant "AI+ feature" in every text, never the service', () => {
  const keys = Object.keys(users.en).filter((k) => k.startsWith('users.ai.') || k === 'users.field.ai' || k === 'users.tip.ai');
  assert.ok(keys.length >= 8);
  for (const lang of LANGS) {
    for (const k of keys) {
      const text = (users[lang] as Record<string, string>)[k];
      assert.ok(text, `${lang} ${k}`);
      assert.doesNotMatch(text, /AI\+ 服务|AI\+ service|AI\+ サービス/i, `${lang} ${k}: ${text}`);
    }
  }
  assert.equal(users.zh['users.field.ai'], 'AI+ 功能');
  assert.equal(users.en['users.field.ai'], 'AI+ feature');
  assert.equal(users.ja['users.field.ai'], 'AI+ 機能');
});

test('the note about hidden AI+ switches gives a cause; only it (administrators) names ai-service', () => {
  for (const lang of LANGS) {
    assert.match(users[lang]['users.ai.hidden'], /ai-service/);
    assert.match(users[lang]['users.ai.hiddenDisconnected'], /ai-service/);
    assert.ok(users[lang]['users.ai.hidden'].length < 200);
  }
  assert.match(users.zh['users.ai.hidden'], /AI\+ 功能/);
});

test('the dark badge tells a user "AI is not available" without the container; an administrator reads the feature', () => {
  assert.equal(ai.zh['ai.badge.unavailable'], 'AI 暂时无法使用，请稍后再试或联系管理员。');
  assert.equal(ai.en['ai.badge.unavailable'], 'AI is not available right now. Try again later or ask an administrator.');
  assert.equal(ai.ja['ai.badge.unavailable'], 'AI は現在利用できません。しばらくしてからもう一度試すか、管理者に連絡してください。');
  for (const lang of LANGS) assert.doesNotMatch(ai[lang]['ai.badge.unavailable'], /ai-service|AI\+/);
  assert.equal(ai.zh['ai.badge.unavailableAdmin'], 'AI+ 功能暂时不可用。');
  setUiLang('en');
  assert.equal(badgeReasonText('unavailable'), ai.en['ai.badge.unavailable']);
  assert.equal(badgeReasonText('unavailable', true), ai.en['ai.badge.unavailableAdmin']);
  // v1.1 (BT-2): an administrator reads the admin wording of the paused tip
  assert.equal(badgeReasonText('suspended', true), ai.en['ai.badge.tip.adminSuspended']);
  assert.equal(badgeReasonText('suspended'), ai.en['ai.badge.tip.suspended']);
  assert.equal(AI_BADGE_TEXT, 'AI+');
});
