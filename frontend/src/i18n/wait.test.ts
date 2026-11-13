// The wait of a sign-in lock in the three UI languages, and the 429 error text built from the backend's retryAfter.
import assert from 'node:assert/strict';
import test from 'node:test';
import { ApiError } from '../api/client.ts';
import { errorText } from './errors.ts';
import { setUiLang } from './index.ts';
import { waitShort, waitText } from './wait.ts';

const MSG_TOO_MANY_LOGINS = '登录失败次数太多，请稍后再试。';

test('wait texts in English, Chinese and Japanese', () => {
  const cases: Array<[number, string, string, string]> = [
    [45, '1 minute', '1 分钟', '1 分'],
    [61, '2 minutes', '2 分钟', '2 分'],
    [900, '15 minutes', '15 分钟', '15 分'],
    [3600, '1 hour', '1 小时', '1 時間'],
    [5000, '2 hours', '2 小时', '2 時間'],
    [86_400, '1 day', '1 天', '1 日'],
  ];
  try {
    for (const [secs, en, zh, ja] of cases) {
      setUiLang('en');
      assert.equal(waitText(secs), en);
      setUiLang('zh');
      assert.equal(waitText(secs), zh);
      setUiLang('ja');
      assert.equal(waitText(secs), ja);
    }
    setUiLang('en');
    assert.equal(waitShort(240), '4 min');
    assert.equal(waitShort(7200), '2 h');
    assert.equal(waitShort(86_400), '1 day');
  } finally {
    setUiLang('en');
  }
});

test('a 429 with retryAfter says how long to wait; without it the general message stays', () => {
  try {
    const e = new ApiError('http', 429, MSG_TOO_MANY_LOGINS, 300);
    setUiLang('en');
    assert.equal(errorText(e), 'Too many failed attempts. Try again in 5 minutes.');
    setUiLang('zh');
    assert.equal(errorText(e), '失败次数太多，请在 5 分钟后再试。');
    setUiLang('ja');
    assert.equal(errorText(e), 'ログインの失敗が多すぎます。5 分後にもう一度お試しください。');
    setUiLang('en');
    assert.equal(errorText(new ApiError('http', 429, MSG_TOO_MANY_LOGINS, 86_400)), 'Too many failed attempts. Try again in 1 day.');
    assert.equal(errorText(new ApiError('http', 429, MSG_TOO_MANY_LOGINS)), 'Too many failed attempts. Please try again later.');
  } finally {
    setUiLang('en');
  }
});
