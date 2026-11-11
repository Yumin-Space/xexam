// The API client and Nginx's rate limit (v0.7.5): a GET refused with Nginx's own 429 is retried (3 tries in all, waiting
// Retry-After or 1 s, then 2 s), a POST never is, a request still refused shows "too many requests" (not "cannot reach the
// service" and without the sign-in hint), and the sign-in lock (429 with JSON "retryAfter") keeps its own handling.
// Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { afterEach, beforeEach, test } from 'node:test';
import { errorText } from '../i18n/errors.ts';
import { setUiLang, t } from '../i18n/index.ts';
import { waitText } from '../i18n/wait.ts';
import { ApiError, getJson, isTooManyRequests, postJson, RATE_LIMIT_TRIES, setRetryPause } from './client.ts';

const MSG_TOO_MANY_LOGINS = '登录失败次数太多，请稍后再试。';

interface Call {
  url: string;
  method: string;
}

let calls: Call[] = [];
let pauses: number[] = [];
let replies: Array<() => Response> = [];
const realFetch = globalThis.fetch;

/** The next responses of the fake fetch, in order */
function reply(...rs: Array<() => Response>): void {
  replies = rs;
}

/** Nginx's own 429: an HTML error page, optionally with Retry-After */
const nginx429 = (retryAfter?: string) => (): Response =>
  new Response('<html><body><h1>429 Too Many Requests</h1></body></html>', {
    status: 429,
    headers: retryAfter == null ? { 'Content-Type': 'text/html' } : { 'Content-Type': 'text/html', 'Retry-After': retryAfter },
  });

const json = (status: number, body: unknown, headers: Record<string, string> = {}) => (): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json; charset=utf-8', ...headers } });

beforeEach(() => {
  calls = [];
  pauses = [];
  replies = [];
  setRetryPause(async (ms) => {
    pauses.push(ms);
  });
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    calls.push({ url: String(input), method: init?.method ?? 'GET' });
    const next = replies.shift();
    if (!next) throw new TypeError('no more replies (network down)');
    return next();
  }) as typeof fetch;
});

afterEach(() => {
  globalThis.fetch = realFetch;
  setUiLang('en');
});

test("a GET refused by Nginx's 429 is retried after 1 s, then 2 s, and succeeds", async () => {
  reply(nginx429(), nginx429(), json(200, { version: '0.7.5' }));
  const info = await getJson<{ version: string }>('/api/info');
  assert.equal(info.version, '0.7.5');
  assert.equal(calls.length, 3);
  assert.ok(calls.every((c) => c.method === 'GET' && c.url === '/api/info'));
  assert.deepEqual(pauses, [1000, 2000]);
});

test('Retry-After (seconds) sets the wait, cut to 10 s; an HTTP date falls back to the default', async () => {
  reply(nginx429('3'), json(200, {}));
  await getJson('/api/bank');
  assert.deepEqual(pauses, [3000]);

  pauses = [];
  reply(nginx429('120'), nginx429('0'), json(200, {}));
  await getJson('/api/bank');
  assert.deepEqual(pauses, [10_000, 0]);

  pauses = [];
  reply(nginx429('Wed, 11 Nov 2026 07:28:00 GMT'), json(200, {}));
  await getJson('/api/bank');
  assert.deepEqual(pauses, [1000]);
});

test('still refused after 3 tries: a "too many requests" error, without the sign-in hint', async () => {
  assert.equal(RATE_LIMIT_TRIES, 3);
  reply(nginx429(), nginx429(), nginx429(), json(200, {}));
  const e = await getJson('/api/progress').then(
    () => assert.fail('should fail'),
    (err: unknown) => err,
  );
  assert.equal(calls.length, 3, 'no fourth try');
  assert.deepEqual(pauses, [1000, 2000]);
  assert.ok(e instanceof ApiError);
  assert.equal(e.kind, 'tooMany');
  assert.equal(e.status, 429);
  assert.ok(isTooManyRequests(e));

  const texts: Record<'en' | 'zh' | 'ja', string> = {
    en: 'Too many requests in a short time. Wait a few seconds, then try again.',
    zh: '短时间内请求太多，请等几秒再试。',
    ja: '短時間にリクエストが多すぎました。数秒待ってから、もう一度お試しください。',
  };
  for (const lang of ['en', 'zh', 'ja'] as const) {
    setUiLang(lang);
    const text = errorText(e);
    assert.equal(text, texts[lang]);
    // Neither "cannot reach the service" nor "your sign-in may have expired"
    assert.notEqual(text, t('err.network'));
    assert.notEqual(text, t('err.status', { status: 429 }));
    assert.ok(!/sign-in|登录|ログイン/.test(text), text);
  }
  // The fatal page's own title and text
  setUiLang('zh');
  assert.equal(t('app.tooMany.title'), '请求太频繁');
  assert.equal(t('app.tooMany.body'), '短时间内请求太多，请等几秒再刷新页面。');
  setUiLang('en');
  assert.equal(t('app.tooMany.title'), 'Too many requests');
  assert.equal(t('app.tooMany.body'), 'Too many requests in a short time. Wait a few seconds, then reload the page.');
  setUiLang('ja');
  assert.equal(t('app.tooMany.title'), 'リクエストが多すぎます');
  assert.equal(t('app.tooMany.body'), '短時間にリクエストが多すぎました。数秒待ってから、ページを再読み込みしてください。');
});

test('a POST is never retried automatically', async () => {
  reply(nginx429(), json(200, { ok: true }));
  const e = await postJson('/api/attempt', { results: [] }).then(
    () => assert.fail('should fail'),
    (err: unknown) => err,
  );
  assert.equal(calls.length, 1);
  assert.equal(calls[0]?.method, 'POST');
  assert.deepEqual(pauses, []);
  assert.ok(isTooManyRequests(e));
});

test('the sign-in lock (429 with JSON "retryAfter") keeps its own handling and is not retried', async () => {
  reply(json(429, { error: MSG_TOO_MANY_LOGINS, retryAfter: 300 }, { 'Retry-After': '300' }));
  const e = await postJson('/api/auth/login', { username: 'demo', password: 'x' }).then(
    () => assert.fail('should fail'),
    (err: unknown) => err,
  );
  assert.equal(calls.length, 1);
  assert.ok(e instanceof ApiError);
  assert.equal(e.kind, 'http');
  assert.equal(e.retryAfter, 300);
  assert.ok(!isTooManyRequests(e));
  assert.equal(errorText(e), t('auth.err.tooMany', { wait: waitText(300) }));

  // The same answer to a GET is not Nginx's limit either: no retry
  reply(json(429, { error: MSG_TOO_MANY_LOGINS, retryAfter: 60 }));
  const g = await getJson('/api/auth/state').then(
    () => assert.fail('should fail'),
    (err: unknown) => err,
  );
  assert.equal(calls.length, 2);
  assert.deepEqual(pauses, []);
  assert.ok(g instanceof ApiError && g.kind === 'http' && g.retryAfter === 60);
});

test('other failures are unchanged: no retry for a 502 or a lost connection', async () => {
  reply(json(502, { error: '后端暂时连不上，请稍后刷新页面再试。' }));
  const bad = await getJson('/api/bank').then(
    () => assert.fail('should fail'),
    (err: unknown) => err,
  );
  assert.ok(bad instanceof ApiError && bad.kind === 'http' && bad.status === 502);
  assert.equal(calls.length, 1);

  reply();
  const down = await getJson('/api/bank').then(
    () => assert.fail('should fail'),
    (err: unknown) => err,
  );
  assert.ok(down instanceof ApiError && down.kind === 'network');
  assert.equal(calls.length, 2);
  assert.deepEqual(pauses, []);
});

// A time limit per try (v0.7.5, the System log's poll reads): a read that hangs fails like a lost connection, so the page
// shows the error and backs off instead of waiting for the network to give up.
const aborted = (): Error => Object.assign(new Error('The operation was aborted.'), { name: 'AbortError' });

test('a GET with a time limit and no answer fails with kind "network"', async () => {
  globalThis.fetch = ((input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    calls.push({ url: String(input), method: init?.method ?? 'GET' });
    return new Promise((_, reject) => {
      const signal = init?.signal;
      assert.ok(signal, 'the try carries an abort signal');
      signal.addEventListener('abort', () => reject(aborted()));
    });
  }) as typeof fetch;
  const started = Date.now();
  const e = await getJson('/api/logs?after=3', { timeoutMs: 30 }).then(
    () => assert.fail('should fail'),
    (err: unknown) => err,
  );
  assert.ok(e instanceof ApiError && e.kind === 'network', String(e));
  assert.ok(Date.now() - started < 5000);
  assert.equal(calls.length, 1, 'a cut-off read is not retried by the client');
  assert.deepEqual(pauses, []);
});

test('a body that stalls past the time limit is a lost connection too, not "unexpected"', async () => {
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    calls.push({ url: String(input), method: init?.method ?? 'GET' });
    const signal = init?.signal;
    const body = new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(new TextEncoder().encode('{"lines":['));
        signal?.addEventListener('abort', () => c.error(aborted()));
      },
    });
    return new Response(body, { status: 200, headers: { 'Content-Type': 'application/json' } });
  }) as typeof fetch;
  const e = await getJson('/api/logs', { timeoutMs: 30 }).then(
    () => assert.fail('should fail'),
    (err: unknown) => err,
  );
  assert.ok(e instanceof ApiError && e.kind === 'network', String(e));
});

test('the time limit is per try: the wait before a retry does not count, and a quick answer is not cut off', async () => {
  setRetryPause((ms) => {
    pauses.push(ms);
    return new Promise((resolve) => setTimeout(resolve, 60));
  });
  reply(nginx429(), json(200, { lines: [] }));
  const ok = await getJson<{ lines: unknown[] }>('/api/logs', { timeoutMs: 40 });
  assert.deepEqual(ok.lines, []);
  assert.equal(calls.length, 2);
  assert.deepEqual(pauses, [1000]);
});

test('without a time limit no abort signal is added', async () => {
  let signal: AbortSignal | null | undefined = null;
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    signal = init?.signal;
    return json(200, {})();
  }) as typeof fetch;
  await getJson('/api/bank');
  assert.equal(signal, undefined);
});
