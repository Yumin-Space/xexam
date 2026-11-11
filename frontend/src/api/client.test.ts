// The API client and Nginx's rate limit (v0.7.5): a GET refused with Nginx's own 429 is retried (3 tries in all, waiting
// Retry-After or 1 s, then 2 s), a POST never is, a request still refused shows "too many requests" (not "cannot reach the
// service" and without the sign-in hint), and the sign-in lock (429 with JSON "retryAfter") keeps its own handling.
// Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { afterEach, beforeEach, test } from 'node:test';
import { errorText } from '../i18n/errors.ts';
import { setUiLang, t } from '../i18n/index.ts';
import { waitText } from '../i18n/wait.ts';
import {
  ApiError, getJson, isAbortError, isTooManyRequests, postJson, postStream, RATE_LIMIT_TRIES, setAuthHandler, setRetryPause, type AuthProblem,
  type StreamEvent,
} from './client.ts';

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

// ---------------------------------------------------------------- v0.8: the AI routes and the streamed answer

/** A 200 event stream whose body arrives in the given chunks (bytes, so a character can be split) */
const sse = (...chunks: Array<string | Uint8Array>) => (): Response =>
  new Response(new ReadableStream<Uint8Array>({
    start(c) {
      for (const ch of chunks) c.enqueue(typeof ch === 'string' ? new TextEncoder().encode(ch) : ch);
      c.close();
    },
  }), { status: 200, headers: { 'Content-Type': 'text/event-stream; charset=utf-8' } });

async function collect(path: string, body: unknown, opts: { signal?: AbortSignal } = {}): Promise<{ events: StreamEvent[]; error: unknown }> {
  const events: StreamEvent[] = [];
  try {
    await postStream(path, body, { ...opts, onEvent: (ev) => events.push(ev) });
    return { events, error: null };
  } catch (e) {
    return { events, error: e };
  }
}

test('postStream delivers the events of a stream (JSON data, split chunks, a character split between chunks)', async () => {
  const text = new TextEncoder().encode('event: delta\ndata: {"text":"漢字"}\n\n');
  // (byte 29 is inside 漢: "event: delta\n" is 13 bytes, 'data: {"text":"' 15 more)
  reply(sse(
    'event: start\r\ndata: {"chatId":3,"assistantMessageId":8}\r\n\r\n: keep-alive\n\n',
    text.slice(0, 29), text.slice(29),
    'event: done\ndata: {"status":"done","truncated":false}\n\n',
  ));
  const { events, error } = await collect('/api/ai/send', { chatId: null, text: 'Hello' });
  assert.equal(error, null);
  assert.deepEqual(events, [
    { event: 'start', data: { chatId: 3, assistantMessageId: 8 } },
    { event: 'delta', data: { text: '漢字' } },
    { event: 'done', data: { status: 'done', truncated: false } },
  ]);
  assert.equal(calls.length, 1);
  assert.equal(calls[0]?.method, 'POST');
});

test("postStream: Nginx's 429 is 'tooMany' at once, never retried", async () => {
  reply(nginx429('1'), sse('data: {}\n\n'));
  const { error } = await collect('/api/ai/send', { chatId: null, text: 'x' });
  assert.ok(isTooManyRequests(error));
  assert.equal(calls.length, 1);
  assert.deepEqual(pauses, []);
});

test('postStream: an error JSON before the stream carries its code, field and limit; 401 goes to the auth handler', async () => {
  const problems: AuthProblem[] = [];
  setAuthHandler((p) => problems.push(p));
  try {
    reply(json(409, { error: '上一个回答还在生成，请等它结束或按停止。', code: 'ai.busy' }));
    let r = await collect('/api/ai/send', { chatId: 1, text: 'x' });
    assert.ok(r.error instanceof ApiError && r.error.status === 409 && r.error.code === 'ai.busy');
    assert.equal(errorText(r.error), 'An answer is still being written. Wait for it or press Stop.');

    reply(json(400, { error: '消息太长了（最多 20000 个字符）。', code: 'ai.tooLong', limit: 20000 }));
    r = await collect('/api/ai/send', { chatId: 1, text: 'x' });
    assert.ok(r.error instanceof ApiError && r.error.limit === 20000);
    assert.equal(errorText(r.error), 'The message is too long (at most 20,000 characters).');

    reply(json(401, { error: '请先登录。', code: 'auth.signin' }));
    r = await collect('/api/ai/send', { chatId: 1, text: 'x' });
    assert.ok(r.error instanceof ApiError && r.error.status === 401);
    assert.deepEqual(problems, ['signin']);

    // The password change: by code (the text alone was enough before; both work)
    reply(json(403, { error: 'something else', code: 'auth.password' }));
    r = await collect('/api/ai/send', { chatId: 1, text: 'x' });
    assert.deepEqual(problems, ['signin', 'password']);
  } finally {
    setAuthHandler(() => {});
  }
});

test('JSON errors of the AI routes: code and field; the guard and the texts in three languages', async () => {
  reply(json(400, { error: '密钥格式不对：只能是 1–400 个可见的英文字符，不能有空格。', code: 'ai.badKey', field: 'gatewayKey' }));
  const e = await postJson('/api/ai/admin/settings', { gatewayKey: 'a b' }).then(() => assert.fail('should fail'), (err: unknown) => err);
  assert.ok(e instanceof ApiError);
  assert.equal(e.code, 'ai.badKey');
  assert.equal(e.field, 'gatewayKey');
  assert.equal(errorText(e), 'A key may have 1–400 visible characters and no spaces.');
  setUiLang('zh');
  assert.equal(errorText(e), '密钥格式不对：只能是 1–400 个可见的英文字符，不能有空格。');
  setUiLang('ja');
  assert.equal(errorText(e), 'キーは空白なしの表示可能な文字 1〜400 文字です。');
  setUiLang('en');

  reply(json(403, { error: '被拒绝：这个请求的 Host 或 Origin 不在允许的列表里，请检查 XEXAM_ALLOWED_HOSTS 和 XEXAM_ALLOWED_ORIGINS。', code: 'auth.guard' }));
  const g = await getJson('/api/ai/status').then(() => assert.fail('should fail'), (err: unknown) => err);
  assert.ok(g instanceof ApiError && g.code === 'auth.guard');
  assert.match(errorText(g), /XEXAM_ALLOWED_HOSTS/);
  assert.match(errorText(g), /^Refused/);

  // An upstream code and a code the catalog does not know
  reply(json(502, { error: 'AI 服务商拒绝了这个密钥。', code: 'ai.up.auth' }));
  const up = await postJson('/api/ai/admin/verify', {}).then(() => assert.fail('should fail'), (err: unknown) => err);
  assert.equal(errorText(up), 'The AI provider rejected the key.');
});

test('a 5xx without JSON on /api/ai/ is "ai.unavailable"; elsewhere it stays a plain status error', async () => {
  reply(() => new Response('<html>502 Bad Gateway</html>', { status: 502, headers: { 'Content-Type': 'text/html' } }));
  const e = await getJson('/api/ai/status').then(() => assert.fail('should fail'), (err: unknown) => err);
  assert.ok(e instanceof ApiError && e.kind === 'status' && e.status === 502 && e.code === 'ai.unavailable');
  assert.equal(errorText(e), 'AI is not available right now. Try again later or ask an administrator.');

  reply(() => new Response('<html>502</html>', { status: 502, headers: { 'Content-Type': 'text/html' } }));
  const s = await collect('/api/ai/send', { chatId: null, text: 'x' });
  assert.ok(s.error instanceof ApiError && s.error.code === 'ai.unavailable');

  reply(() => new Response('<html>502</html>', { status: 502, headers: { 'Content-Type': 'text/html' } }));
  const other = await getJson('/api/bank').then(() => assert.fail('should fail'), (err: unknown) => err);
  assert.ok(other instanceof ApiError && other.code === undefined);
});

test('postStream: aborting rejects with an AbortError; a read error that is not an abort is "network"', async () => {
  const ctrl = new AbortController();
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    const signal = init?.signal;
    return new Response(new ReadableStream<Uint8Array>({
      start(c) {
        c.enqueue(new TextEncoder().encode('event: start\ndata: {"chatId":1,"assistantMessageId":2}\n\n'));
        signal?.addEventListener('abort', () => c.error(aborted()));
      },
    }), { status: 200, headers: { 'Content-Type': 'text/event-stream' } });
  }) as typeof fetch;
  const run = collect('/api/ai/send', { chatId: null, text: 'x' }, { signal: ctrl.signal });
  await new Promise((r) => setTimeout(r, 20));
  ctrl.abort();
  const r = await run;
  assert.equal(r.events.length, 1);
  assert.ok(isAbortError(r.error), String(r.error));

  globalThis.fetch = (async (): Promise<Response> => new Response(new ReadableStream<Uint8Array>({
    start(c) {
      c.enqueue(new TextEncoder().encode('event: delta\ndata: {"text":"a"}\n\n'));
      setTimeout(() => c.error(new TypeError('network connection lost')), 5);
    },
  }), { status: 200, headers: { 'Content-Type': 'text/event-stream' } })) as typeof fetch;
  const lost = await collect('/api/ai/send', { chatId: null, text: 'x' });
  assert.equal(lost.events.length, 1);
  assert.ok(lost.error instanceof ApiError && lost.error.kind === 'network', String(lost.error));
});

test('postStream: data that is not JSON, or a 200 that is not an event stream, is "unexpected"', async () => {
  reply(sse('event: delta\ndata: not json\n\n'));
  const bad = await collect('/api/ai/send', { chatId: null, text: 'x' });
  assert.ok(bad.error instanceof ApiError && bad.error.kind === 'unexpected');
  reply(json(200, { ok: true }));
  const plain = await collect('/api/ai/send', { chatId: null, text: 'x' });
  assert.ok(plain.error instanceof ApiError && plain.error.kind === 'unexpected');
});

test('postStream sends JSON and asks for an event stream', async () => {
  let seen: RequestInit | undefined;
  globalThis.fetch = (async (_input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    seen = init;
    return sse('event: done\ndata: {"status":"done"}\n\n')();
  }) as typeof fetch;
  await postStream('/api/ai/retry', { chatId: 4 }, { onEvent: () => {} });
  assert.equal(seen?.method, 'POST');
  assert.equal(seen?.credentials, 'same-origin');
  assert.equal(seen?.body, '{"chatId":4}');
  const headers = seen?.headers as Record<string, string>;
  assert.equal(headers['Content-Type'], 'application/json');
  assert.equal(headers.Accept, 'text/event-stream');
});
