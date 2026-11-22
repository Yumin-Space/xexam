// The v2.5 dashboard client: the request it sends and how it reads the answer. fetch is replaced; no network.
import assert from 'node:assert/strict';
import { afterEach, beforeEach, test } from 'node:test';
import { fetchDashboard, type Dashboard } from './dashboard.ts';

interface Call { url: string; method: string }
let calls: Call[] = [];
let replies: Array<() => Response> = [];
const realFetch = globalThis.fetch;
const json = (status: number, body: unknown) => (): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json; charset=utf-8' } });

beforeEach(() => {
  calls = [];
  replies = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    calls.push({ url: String(input), method: init?.method ?? 'GET' });
    const next = replies.shift();
    if (!next) throw new Error('no reply queued');
    return next();
  }) as typeof fetch;
});
afterEach(() => {
  globalThis.fetch = realFetch;
});

const SAMPLE: Dashboard = {
  ranking: {
    users: 2,
    top: [{ rank: 1, name: 'a***e', answered: 30, correct: 28, self: false }, { rank: 2, name: 'mia', answered: 20, correct: 12, self: true }],
    me: { rank: 2, answered: 20, correct: 12 },
  },
  popular: { days: 30, since: '2026-10-23 14:05', items: [{ questionId: 'q1', bankId: 3, times: 4, correct: 2 }] },
};

test('fetchDashboard sends one GET and returns the body as it is', async () => {
  replies.push(json(200, SAMPLE));
  const d = await fetchDashboard();
  assert.equal(calls.length, 1);
  assert.equal(calls[0]!.method, 'GET');
  assert.ok(calls[0]!.url.endsWith('/api/dashboard'));
  assert.deepEqual(d, SAMPLE);
});

test('fetchDashboard rejects on a refusal', async () => {
  replies.push(json(403, { error: 'no' }));
  await assert.rejects(fetchDashboard());
});
