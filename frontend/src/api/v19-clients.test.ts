// v1.9 API client: the connection switch posts its exact body, and a feature switch sends one key. fetch is replaced; no network.
import assert from 'node:assert/strict';
import { afterEach, beforeEach, test } from 'node:test';
import { setConnectionEnabled, setFeatureSwitches } from './ai-admin.ts';

interface Call { url: string; method: string; body: unknown }
let calls: Call[] = [];
const realFetch = globalThis.fetch;

beforeEach(() => {
  calls = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    calls.push({ url: String(input), method: init?.method ?? 'GET', body: typeof init?.body === 'string' ? JSON.parse(init.body) as unknown : undefined });
    return new Response(JSON.stringify({ settings: { enabled: true } }), { status: 200, headers: { 'Content-Type': 'application/json; charset=utf-8' } });
  }) as typeof fetch;
});
afterEach(() => { globalThis.fetch = realFetch; });

test('setConnectionEnabled posts the id and the flag to the enabled route', async () => {
  await setConnectionEnabled(3, false);
  await setConnectionEnabled(3, true);
  assert.deepEqual(calls.map((c) => `${c.method} ${c.url}`), ['POST /api/ai/admin/connections/enabled', 'POST /api/ai/admin/connections/enabled']);
  assert.deepEqual(calls[0]?.body, { id: 3, enabled: false });
  assert.deepEqual(calls[1]?.body, { id: 3, enabled: true });
});

test('setFeatureSwitches({ hint: false }) sends exactly one key', async () => {
  await setFeatureSwitches({ hint: false });
  assert.equal(calls[0]?.url, '/api/ai/admin/feature-switches');
  assert.deepEqual(calls[0]?.body, { hint: false });
});
