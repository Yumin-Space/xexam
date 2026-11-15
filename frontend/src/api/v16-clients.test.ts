// v1.6 API client: setEffort posts the exact body to the effort route (object and null forms). fetch is replaced; no network.
import assert from 'node:assert/strict';
import { afterEach, beforeEach, test } from 'node:test';
import { setEffort } from './ai-admin.ts';

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

test('setEffort posts the slot and the choice, or null for the default', async () => {
  await setEffort({ slot: 'chat', effort: { family: 'openai', level: 'high' } });
  await setEffort({ slot: 'study', effort: null });
  assert.deepEqual(calls.map((c) => `${c.method} ${c.url}`), ['POST /api/ai/admin/models/effort', 'POST /api/ai/admin/models/effort']);
  assert.deepEqual(calls[0]?.body, { slot: 'chat', effort: { family: 'openai', level: 'high' } });
  assert.deepEqual(calls[1]?.body, { slot: 'study', effort: null });
});
