// v1.0 AI request caps: the refusal text in three languages (cap named, when it frees, administrator advice), the day cap's
// fixed "0:00 tomorrow", the plain sentence without placeholders, and errorText / studyErrorText taking quota first.
import assert from 'node:assert/strict';
import test from 'node:test';
import { ApiError } from '../api/client.ts';
import { callLimitText, errorText, setAdminForTexts, studyErrorText } from './errors.ts';
import { setUiLang } from './index.ts';
import { clockText } from './wait.ts';

const NOW = 1_762_950_000;
const TIME = /\d{2}:\d{2}/;

test('hour cap in English, Chinese and Japanese: the cap named, the wait rounded up, a clock time', () => {
  const q = { cap: 'hour', limit: 20, retryAt: NOW + 712 } as const; // 11 min 52 s -> 12 minutes
  try {
    setUiLang('en');
    const en = callLimitText(q, false, NOW);
    assert.match(en, /^You have used up your AI requests \(20 per hour\)\. Ask an administrator to change the AI request limits\. You can use AI again in about 12 minutes \(\d{2}:\d{2}\)\.$/);
    setUiLang('zh');
    const zh = callLimitText(q, false, NOW);
    assert.match(zh, /^你的 AI 请求额度已用完（每小时 20 次），请联系管理员设置 AI 请求额度上限。约 12 分钟后（\d{2}:\d{2}）可以再次使用。$/);
    setUiLang('ja');
    const ja = callLimitText(q, false, NOW);
    assert.match(ja, /^AI リクエストの上限に達しました（1 時間あたり 20 回）。上限の変更は管理者に依頼してください。約 12 分後（\d{2}:\d{2}）に再び使えます。$/);
  } finally {
    setUiLang('en');
  }
});

test('minute cap under 60 s says "less than a minute"; a minute cap above shows the wait and time', () => {
  try {
    setUiLang('zh');
    assert.equal(
      callLimitText({ cap: 'minute', limit: 5, retryAt: NOW + 40 }, false, NOW),
      '你的 AI 请求额度已用完（每分钟 5 次），请联系管理员设置 AI 请求额度上限。不到 1 分钟后可以再次使用。',
    );
    assert.equal(
      callLimitText({ cap: 'minute', limit: 5, retryAt: NOW }, false, NOW),
      '你的 AI 请求额度已用完（每分钟 5 次），请联系管理员设置 AI 请求额度上限。不到 1 分钟后可以再次使用。',
      'a past retryAt still reads as soon',
    );
    assert.match(callLimitText({ cap: 'minute', limit: 5, retryAt: NOW + 60 }, false, NOW), /约 1 分钟后（\d{2}:\d{2}）/);
  } finally {
    setUiLang('en');
  }
});

test('day cap: always "after 0:00 tomorrow", never a wait or a time, whatever retryAt is', () => {
  const q = { cap: 'day', limit: 100, retryAt: NOW + 5 * 3600 } as const;
  try {
    setUiLang('zh');
    assert.equal(callLimitText(q, false, NOW), '你的 AI 请求额度已用完（每天 100 次），请联系管理员设置 AI 请求额度上限。明天 0 点后可以再次使用。');
    setUiLang('en');
    assert.equal(
      callLimitText(q, false, NOW),
      'You have used up your AI requests (100 per day). Ask an administrator to change the AI request limits. You can use AI again after 0:00 tomorrow.',
    );
    setUiLang('ja');
    assert.ok(callLimitText(q, false, NOW).endsWith('明日 0 時以降に再び使えます。'));
    assert.doesNotMatch(callLimitText(q, false, NOW), TIME);
  } finally {
    setUiLang('en');
  }
});

test('administrators are pointed to AI settings > Safety limits instead of "ask an administrator"', () => {
  const q = { cap: 'day', limit: 100, retryAt: NOW + 3600 } as const;
  try {
    setUiLang('zh');
    assert.equal(callLimitText(q, true, NOW), '你的 AI 请求额度已用完（每天 100 次）。可以在「AI 设置 › 安全上限」里调整。明天 0 点后可以再次使用。');
    setUiLang('en');
    assert.match(callLimitText(q, true, NOW), /You can change the limits in AI settings › Safety limits\./);
    setUiLang('ja');
    assert.match(callLimitText(q, true, NOW), /「AI 設定 › 安全上限」で変更できます。/);
  } finally {
    setUiLang('en');
  }
});

test('errorText and studyErrorText use the quota first (admin from setAdminForTexts); without quota it is the plain sentence', () => {
  const retryAt = Math.floor(Date.now() / 1000) + 3 * 3600;
  const withQuota = new ApiError('http', 429, '你的 AI 请求额度已用完（每天 100 次）', 3600, {
    code: 'ai.callLimit', limit: 100, quota: { cap: 'day', limit: 100, retryAt },
  });
  const plain = new ApiError('http', 429, '你的 AI 请求额度已用完', 3600, { code: 'ai.callLimit', limit: 100 });
  try {
    setUiLang('en');
    setAdminForTexts(() => false);
    assert.match(errorText(withQuota), /^You have used up your AI requests \(100 per day\)\. Ask an administrator/);
    assert.doesNotMatch(errorText(withQuota), /minutes|hours/, 'not the sign-in lock text');
    assert.equal(errorText(plain), 'You have used up your AI requests. Ask an administrator to change the AI request limits.');
    assert.doesNotMatch(errorText(plain), /\{|\}/, 'no placeholder leaks');
    setAdminForTexts(() => true);
    assert.match(errorText(withQuota), /AI settings › Safety limits/);
    assert.match(studyErrorText({ status: 429, code: 'ai.callLimit', limit: 100, quota: { cap: 'hour', limit: 20, retryAt } }), /\(20 per hour\)\. You can change the limits in AI settings/);
    assert.equal(studyErrorText({ status: 429, code: 'ai.callLimit', limit: 100 }), 'You have used up your AI requests. Ask an administrator to change the AI request limits.');
  } finally {
    setAdminForTexts(() => false);
    setUiLang('en');
  }
});

test('clockText: HH:MM today, M/D HH:MM on another day', () => {
  const now = new Date(2026, 10, 12, 10, 0, 0).getTime() / 1000;
  const later = new Date(2026, 10, 12, 14, 5, 0).getTime() / 1000;
  const tomorrow = new Date(2026, 10, 13, 0, 30, 0).getTime() / 1000;
  setUiLang('en');
  assert.equal(clockText(later, now), '14:05');
  assert.equal(clockText(tomorrow, now), '11/13 00:30');
});
