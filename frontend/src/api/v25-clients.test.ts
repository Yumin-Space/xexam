// The v2.5 dashboard client: the request it sends and how it reads the answer. fetch is replaced; no network.
import assert from 'node:assert/strict';
import { afterEach, beforeEach, test } from 'node:test';
import { fetchAssessLatest } from './ai.ts';
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
  wrongTop: { items: [{ questionId: 'q1', bankId: 3, wrong: 4, attempts: 6 }] },
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

test('fetchAssessLatest sends one GET and reads the saved assessment', async () => {
  replies.push(json(200, {
    assess: { text: 'Keep a steady rhythm.', day: '2026-11-22', createdAt: 1790000000, latestAt: '2026-11-22 09:10', records: 5, answered: 80, model: 'sample-model' },
    today: '2026-11-22', perDay: 3, remaining: 0,
  }));
  const r = await fetchAssessLatest();
  assert.equal(calls.length, 1);
  assert.equal(calls[0]!.method, 'GET');
  assert.ok(calls[0]!.url.endsWith('/api/ai/assess/latest'));
  assert.equal(r.assess?.text, 'Keep a steady rhythm.');
  assert.equal(r.assess?.model, 'sample-model');
  assert.deepEqual([r.perDay, r.remaining], [3, 0]);
});

test('fetchAssessLatest: no assessment yet, and an answer of another shape rejects', async () => {
  replies.push(json(200, { assess: null, today: '2026-11-22', perDay: null, remaining: null }));
  const r = await fetchAssessLatest();
  assert.equal(r.assess, null);
  assert.deepEqual([r.perDay, r.remaining], [null, null], 'null = no limit for this viewer');
  replies.push(json(200, { nope: 1 }));
  await assert.rejects(fetchAssessLatest());
});
