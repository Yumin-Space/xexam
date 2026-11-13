// The AI import upload (IM-5): a fake XHR: headers, progress, abort, success parsing, and for the same status / body / headers
// the same ApiError as the fetch client builds. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { afterEach, test } from 'node:test';
import { ApiError, errorFromResponse, getJson, setAuthHandler, setRetryPause, type AuthProblem } from './client.ts';
import { abortError, UPLOAD_PATH, uploadImport, type XhrLike } from './ai-import.ts';

class FakeXhr implements XhrLike {
  method = '';
  url = '';
  headers: Record<string, string> = {};
  sent: unknown = undefined;
  aborted = false;
  status = 0;
  responseText = '';
  respHeaders: Record<string, string> = {};
  upload: XhrLike['upload'] = { onprogress: null };
  onload: (() => void) | null = null;
  onerror: (() => void) | null = null;
  onabort: (() => void) | null = null;
  ontimeout: (() => void) | null = null;
  open(m: string, u: string): void { this.method = m; this.url = u; }
  setRequestHeader(n: string, v: string): void { this.headers[n] = v; }
  send(b: unknown): void { this.sent = b; }
  abort(): void { this.aborted = true; }
  getResponseHeader(n: string): string | null { return this.respHeaders[n] ?? null; }
  answer(status: number, body: string, headers: Record<string, string> = {}): void {
    this.status = status;
    this.responseText = body;
    this.respHeaders = headers;
    this.onload?.();
  }
}

const file = { name: 'Kitchen notes 1.pdf', size: 3 } as unknown as File;
const JOB = {
  id: 9, state: 'queued', phase: 'reading', source: 'file', name: 'n', from: [], fromCount: 0,
  file: { name: 'Kitchen notes 1.pdf', type: 'pdf', size: 3 }, made: 0, goal: 0, calls: 0, createdAt: 1, startedAt: null, finishedAt: null,
  bankId: null, error: null, charged: false,
};
const GEN = { perDay: 3, used: 0, left: 3, unlimited: false, resetAt: 5, minSource: 30, minWrong: 10, bank: { single: 14, multi: 6 }, wrong: { single: 14, multi: 6 }, job: JOB, unseen: [] };

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
  setAuthHandler(() => {});
});

function run(lang: 'zh' | 'en' | 'ja' = 'zh') {
  const x = new FakeXhr();
  const progress: Array<[number, number]> = [];
  const ctrl = new AbortController();
  const p = uploadImport(file, lang, (a, b) => progress.push([a, b]), ctrl.signal, () => x);
  return { x, p, progress, ctrl };
}

test('sends the raw file with the encoded name and the UI language', async () => {
  const { x, p } = run('zh');
  assert.equal(x.method, 'POST');
  assert.equal(x.url, UPLOAD_PATH);
  assert.equal(x.headers['Content-Type'], 'application/octet-stream');
  assert.equal(x.headers['X-Filename'], 'Kitchen%20notes%201.pdf');
  assert.equal(x.headers['X-Ui-Lang'], 'zh-CN');
  assert.equal(x.sent, file);
  x.answer(202, JSON.stringify({ job: JOB, gen: GEN }));
  const a = await p;
  assert.equal(a.job.id, 9);
  assert.equal(a.job.file?.type, 'pdf');
  assert.equal(a.gen.job?.id, 9);
  assert.equal(run('ja').x.headers['X-Ui-Lang'], 'ja');
  assert.equal(run('en').x.headers['X-Ui-Lang'], 'en');
});

test('progress events reach onProgress (only computable ones)', async () => {
  const { x, p, progress } = run();
  x.upload.onprogress?.({ loaded: 1, total: 3, lengthComputable: true });
  x.upload.onprogress?.({ loaded: 2, total: 0, lengthComputable: false });
  x.upload.onprogress?.({ loaded: 3, total: 3, lengthComputable: true });
  assert.deepEqual(progress, [[1, 3], [3, 3]]);
  x.answer(202, JSON.stringify({ job: JOB, gen: GEN }));
  await p;
});

test('aborting the signal aborts the request and rejects with an AbortError', async () => {
  const { x, p, ctrl } = run();
  ctrl.abort();
  await assert.rejects(p, (e: Error) => e.name === 'AbortError');
  assert.equal(x.aborted, true);
  // a signal that is already aborted never opens a request
  const c2 = new AbortController();
  c2.abort();
  let made = false;
  await assert.rejects(uploadImport(file, 'en', () => {}, c2.signal, () => { made = true; return new FakeXhr(); }), (e: Error) => e.name === 'AbortError');
  assert.equal(made, false);
  assert.equal(abortError().name, 'AbortError');
});

test('a lost connection is kind network; a body that is not a job is unexpected', async () => {
  const a = run();
  a.x.onerror?.();
  await assert.rejects(a.p, (e: unknown) => e instanceof ApiError && e.kind === 'network');
  const b = run();
  b.x.answer(202, 'not json');
  await assert.rejects(b.p, (e: unknown) => e instanceof ApiError && e.kind === 'unexpected');
  const c = run();
  c.x.answer(202, JSON.stringify({ job: { id: 1 } }));
  await assert.rejects(c.p, (e: unknown) => e instanceof ApiError && e.kind === 'unexpected');
});

/** What the fetch client throws for a response, through its public GET */
async function fetchError(status: number, body: string, headers: Record<string, string>): Promise<ApiError> {
  setRetryPause(async () => {});
  globalThis.fetch = (async () => new Response(body, { status, headers })) as typeof fetch;
  try {
    await getJson('/api/ai/import/upload');
  } catch (e) {
    return e as ApiError;
  }
  throw new Error('no error');
}

const shape = (e: ApiError): unknown => ({
  kind: e.kind, status: e.status, msg: e.serverMessage, retryAfter: e.retryAfter, code: e.code, field: e.field, limit: e.limit, body: e.body,
});

test('a refusal is exactly the ApiError of the fetch client for the same status, body and headers', async () => {
  const json = { 'Content-Type': 'application/json' };
  const cases: Array<[number, string, Record<string, string>]> = [
    [415, JSON.stringify({ error: 'x', code: 'ai.import.type', ext: 'zip' }), json],
    [413, JSON.stringify({ error: 'x', code: 'ai.import.tooLarge', maxMb: 20 }), json],
    [409, JSON.stringify({ error: 'x', code: 'ai.gen.busy', job: null }), json],
    [429, JSON.stringify({ error: 'x', code: 'ai.gen.quota', used: 3, limit: 3, resetAt: 9, retryAfter: 600 }), { ...json, 'Retry-After': '600' }],
    [403, JSON.stringify({ error: 'x', code: 'ai.featureOff', feature: 'import' }), json],
    [400, JSON.stringify({ error: '坏请求' }), json],
    [502, '<html>bad gateway</html>', { 'Content-Type': 'text/html' }],
    [429, '<html>429</html>', { 'Content-Type': 'text/html' }],
  ];
  for (const [status, body, headers] of cases) {
    const want = await fetchError(status, body, headers);
    const { x, p } = run();
    x.answer(status, body, headers['Retry-After'] ? { 'Retry-After': headers['Retry-After'] } : {});
    const got = await p.catch((e: ApiError) => e);
    assert.ok(got instanceof ApiError, `case ${status}`);
    // the fetch client reports a 5xx without JSON on /api/ai/ as `ai.unavailable`, so does errorFromResponse(path)
    assert.deepEqual(shape(got as ApiError), shape(want), `case ${status} ${body.slice(0, 20)}`);
  }
});

test('a 401 runs the sign-in handling, like the fetch client', async () => {
  const seen: AuthProblem[] = [];
  setAuthHandler((p) => seen.push(p));
  const { x, p } = run();
  x.answer(401, JSON.stringify({ error: '请先登录。' }), {});
  await assert.rejects(p, (e: unknown) => e instanceof ApiError && e.status === 401);
  assert.deepEqual(seen, ['signin']);
  assert.equal(errorFromResponse(401, '{"error":"x"}', null).status, 401);
});
