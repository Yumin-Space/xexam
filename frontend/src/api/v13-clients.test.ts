// The v1.3 API clients (activity, admin AI settings, chat branches, sign-in, generation items): the request each one sends
// and how it reads the answer. fetch is replaced; no network. Run with `npm test`.
import assert from 'node:assert/strict';
import { afterEach, beforeEach, test } from 'node:test';
import type { ActivityPage, AiSettings, GenJob } from '../types/api.ts';
import { activityQuery, fetchActivity, fetchActivitySettings, reportVisit, saveActivitySettings } from './activity.ts';
import {
  adminChat, clearAiData, deleteConnection, fetchAiSettings, fetchLimits, fetchModels, orderConnections, pickModel, removeModel,
  saveConnection, saveRetention, setEnabled, setFeatureSwitches, testConnection, testModel,
} from './ai-admin.ts';
import { fetchGenJob } from './ai-gen.ts';
import { askStudy, editMessage, regenerate, setChatModel, switchBranch } from './ai.ts';
import { loginSession } from './auth.ts';
import { clearSigninInfo, setSigninInfo, usersListOf } from './users.ts';

interface Call { url: string; method: string; body: unknown }
let calls: Call[] = [];
let replies: Array<() => Response> = [];
const realFetch = globalThis.fetch;
const json = (status: number, body: unknown) => (): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json; charset=utf-8' } });
const sse = (): Response =>
  new Response('event: done\ndata: {}\n\n', { status: 200, headers: { 'Content-Type': 'text/event-stream' } });

beforeEach(() => {
  calls = [];
  replies = [];
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit): Promise<Response> => {
    const body = typeof init?.body === 'string' ? JSON.parse(init.body) as unknown : undefined;
    calls.push({ url: String(input), method: init?.method ?? 'GET', body });
    const next = replies.shift();
    if (!next) throw new TypeError('no more replies (network down)');
    return next();
  }) as typeof fetch;
});
afterEach(() => {
  globalThis.fetch = realFetch;
});

const settings = { enabled: true, state: 'on' } as unknown as AiSettings;
const job = (o: Partial<GenJob> = {}): Record<string, unknown> => ({
  id: 5, state: 'running', phase: 'writing', source: 'bank', name: 'demo', from: [], fromCount: 0, file: null, mode: null,
  read: null, confirmBy: null, errorArg: null, skipped: null, made: 8, goal: 20, createdAt: 1, updatedAt: 2, messageCount: 0,
  running: true, startedAt: null, finishedAt: null, bankId: null, error: null, charged: false, ...o,
});

test('activityQuery: only the keys that are set, facets as 1', () => {
  assert.equal(activityQuery({}), '');
  assert.equal(activityQuery({ before: 711, limit: 100, user: 3, group: 'auth', action: 'auth.signin', result: 'fail', facets: true }),
    '?before=711&limit=100&user=3&group=auth&action=auth.signin&result=fail&facets=1');
  assert.equal(activityQuery({ after: 812, user: 'system', facets: false }), '?after=812&user=system');
});

test('activity clients: paths, bodies and answers', async () => {
  const page: ActivityPage = { entries: [], next: null, newest: null, settings: { retentionDays: 90, userVisits: false, signinInfo: true } };
  replies = [json(200, page), json(200, page.settings), json(200, page.settings)];
  assert.deepEqual(await fetchActivity({ after: 5 }), page);
  assert.deepEqual(await fetchActivitySettings(), page.settings);
  await saveActivitySettings({ retentionDays: 30 });
  assert.deepEqual(calls.map((c) => [c.method, c.url]), [['GET', '/api/activity?after=5'], ['GET', '/api/activity/settings'], ['POST', '/api/activity/settings']]);
  assert.deepEqual(calls[2]?.body, { retentionDays: 30 });
});

test('reportVisit posts the page and swallows every error', async () => {
  replies = [json(200, { ok: true })];
  await reportVisit('quiz');
  assert.deepEqual(calls[0]?.body, { page: 'quiz' });
  replies = [json(500, { code: 'x' })];
  await reportVisit('study');
  replies = [];
  await reportVisit('about'); // network down
  assert.equal(calls.length, 3);
});

test('loginSession sends remember and returns the session rule', async () => {
  const user = { id: 1, username: 'demo-admin', role: 'admin' };
  const session = { rule: 'admin', idleSecs: 86400, hardEndsAt: '2026-11-21 10:00:00' };
  replies = [json(200, { user, session })];
  const got = await loginSession('demo-admin', 'demo-pass', true);
  assert.deepEqual(calls[0]?.body, { username: 'demo-admin', password: 'demo-pass', remember: true });
  assert.deepEqual(got.session, session);
  replies = [json(200, { user })];
  assert.equal((await loginSession('demo-user', 'demo-pass')).session, null);
  assert.deepEqual(calls[1]?.body, { username: 'demo-user', password: 'demo-pass', remember: false });
});

test('setSigninInfo posts the eye state; usersListOf normalises the v1.3 fields', async () => {
  replies = [json(200, { users: [{ id: 2, username: 'demo-user' }], aiAvailable: true, signinInfo: true })];
  const got = await setSigninInfo(true);
  assert.deepEqual(calls[0]?.body, { enabled: true });
  assert.equal(calls[0]?.url, '/api/users/signin-info');
  assert.equal(got.signinInfo, true);
  assert.equal(got.users[0]?.lastActiveAt, null);
  assert.equal(got.users[0]?.lastLoginIp, null);
  assert.equal(got.users[0]?.lastLoginLocation, null);
  const keep = usersListOf({ users: [{ id: 2, lastActiveAt: '2026-11-14 07:20:00', lastLoginIp: '203.0.113.7', lastLoginLocation: { country: 'ZZ', region: null, city: null } } as never], aiAvailable: true });
  assert.equal(keep.users[0]?.lastActiveAt, '2026-11-14 07:20:00');
  assert.equal(keep.users[0]?.lastLoginIp, '203.0.113.7');
  assert.equal(keep.users[0]?.lastLoginLocation?.country, 'ZZ');
  assert.equal(keep.signinInfo, false);
});

test('clearSigninInfo posts to the clear route and answers with the users list', async () => {
  replies = [json(200, { users: [{ id: 2, username: 'demo-user', lastLoginIp: null, lastLoginLocation: null }], aiAvailable: true, signinInfo: true })];
  const before = calls.length;
  const got = await clearSigninInfo();
  assert.equal(calls[before]?.url, '/api/users/signin-info/clear');
  assert.deepEqual(calls[before]?.body, {});
  assert.equal(got.signinInfo, true, 'the eye state is untouched by the clear');
  assert.equal(got.users[0]?.lastLoginIp, null);
  assert.equal(got.users[0]?.lastLoginLocation, null);
});

test('chat branch clients: regenerate / edit / branch / model', async () => {
  const seen: unknown[] = [];
  const opts = { onEvent: (e: unknown) => seen.push(e) };
  replies = [sse, sse, sse, sse];
  await regenerate(4, 20, 'en', opts);
  await regenerate(4, 20, 'zh', opts, { connectionId: 2, model: 'demo:x' });
  await editMessage(4, 19, 'new text', 'ja', opts);
  await editMessage(4, 19, 'new text', 'en', opts, { connectionId: 1, model: 'm' });
  assert.deepEqual(calls.map((c) => c.url), ['/api/ai/regenerate', '/api/ai/regenerate', '/api/ai/edit', '/api/ai/edit']);
  assert.deepEqual(calls[0]?.body, { chatId: 4, messageId: 20, lang: 'en' });
  assert.deepEqual(calls[1]?.body, { chatId: 4, messageId: 20, lang: 'zh', model: { connectionId: 2, model: 'demo:x' } });
  assert.deepEqual(calls[2]?.body, { chatId: 4, messageId: 19, text: 'new text', lang: 'ja' });
  assert.deepEqual(calls[3]?.body, { chatId: 4, messageId: 19, text: 'new text', lang: 'en', model: { connectionId: 1, model: 'm' } });
  assert.equal(seen.length, 4);
  replies = [json(200, { chat: { id: 4 }, messages: [] }), json(200, { chat: { id: 4 } })];
  await switchBranch(4, 641);
  await setChatModel(4, null);
  assert.deepEqual(calls[4]?.body, { chatId: 4, messageId: 641 });
  assert.deepEqual(calls[5]?.body, { chatId: 4, model: null });
  assert.equal(calls[5]?.url, '/api/ai/chat/model');
});

test('askStudy adds fresh only when asked', async () => {
  replies = [sse, sse];
  const body = { x: 1 } as never;
  await askStudy(body, { onEvent: () => {} });
  await askStudy(body, { onEvent: () => {} }, true);
  assert.deepEqual(calls[0]?.body, { x: 1 });
  assert.deepEqual(calls[1]?.body, { x: 1, fresh: true });
});

test('fetchGenJob: the job alone, or the job with items after a number', async () => {
  replies = [json(200, { job: job() })];
  assert.equal((await fetchGenJob(5)).id, 5);
  assert.equal(calls[0]?.url, '/api/ai/gen/job?id=5');
  const items = [{ no: 6, kind: 'single', stem: 'demo stem', options: [{ text: 'a', correct: true }], explanation: 'e' }];
  replies = [json(200, { job: job({ phase: 'planning' as never }), items, last: 8, gone: [3, 'x'] })];
  const a = await fetchGenJob(5, 5, 'ja');
  assert.equal(calls[1]?.url, '/api/ai/gen/job?id=5&after=5&lang=ja');
  assert.deepEqual(a.items, items);
  assert.equal(a.last, 8);
  assert.deepEqual(a.gone, [3]);
  replies = [json(200, { job: job() })];
  const b = await fetchGenJob(5, 0);
  assert.equal(calls[2]?.url, '/api/ai/gen/job?id=5&after=0');
  assert.equal(b.items, undefined);
});

test('admin AI settings clients: methods, paths and bodies', async () => {
  replies = Array.from({ length: 16 }, () => json(200, { ok: true, settings }));
  await fetchAiSettings();
  await saveRetention(30);
  await saveConnection({ name: 'main', provider: 'openai', baseUrl: 'https://api.example.com/v1', apiKey: 'demo-key', reuseKeysFrom: 2, confirmHost: 'api.example.com' });
  await testConnection(1);
  await deleteConnection(1);
  await deleteConnection(1, { force: true, newDefault: { connectionId: 2, model: 'm' } });
  await orderConnections([2, 1]);
  await fetchModels();
  await fetchModels(true);
  await pickModel({ slot: 'chat', connectionId: 1, model: 'm', manual: false });
  await pickModel({ slot: 'chat', clear: true });
  await testModel(1, 'm');
  await removeModel(4);
  await setEnabled(true, { connectionId: 1, model: 'm' });
  await setEnabled(false);
  await setFeatureSwitches({ hint: false });
  const got = calls.map((c) => `${c.method} ${c.url}`);
  assert.deepEqual(got, [
    'GET /api/ai/admin/settings', 'POST /api/ai/admin/settings', 'POST /api/ai/admin/connections/save',
    'POST /api/ai/admin/connections/test', 'POST /api/ai/admin/connections/delete', 'POST /api/ai/admin/connections/delete',
    'POST /api/ai/admin/connections/order', 'GET /api/ai/admin/models?refresh=0', 'GET /api/ai/admin/models?refresh=1',
    'POST /api/ai/admin/models/pick', 'POST /api/ai/admin/models/pick', 'POST /api/ai/admin/models/test',
    'POST /api/ai/admin/models/remove', 'POST /api/ai/admin/enable', 'POST /api/ai/admin/enable',
    'POST /api/ai/admin/feature-switches',
  ]);
  assert.deepEqual(calls[1]?.body, { retentionDays: 30 });
  assert.deepEqual(calls[2]?.body, { name: 'main', provider: 'openai', baseUrl: 'https://api.example.com/v1', apiKey: 'demo-key', reuseKeysFrom: 2, confirmHost: 'api.example.com' });
  assert.deepEqual(calls[3]?.body, { id: 1 });
  assert.deepEqual(calls[4]?.body, { id: 1, force: false });
  assert.deepEqual(calls[5]?.body, { id: 1, force: true, newDefault: { connectionId: 2, model: 'm' } });
  assert.deepEqual(calls[6]?.body, { ids: [2, 1] });
  assert.deepEqual(calls[9]?.body, { slot: 'chat', connectionId: 1, model: 'm', manual: false });
  assert.deepEqual(calls[10]?.body, { slot: 'chat', clear: true });
  assert.deepEqual(calls[11]?.body, { connectionId: 1, model: 'm' });
  assert.deepEqual(calls[12]?.body, { poolId: 4 });
  assert.deepEqual(calls[13]?.body, { enabled: true, default: { connectionId: 1, model: 'm' } });
  assert.deepEqual(calls[14]?.body, { enabled: false });
  assert.deepEqual(calls[15]?.body, { hint: false });
});

test('admin limits / clear / chat viewer', async () => {
  replies = [json(200, { limits: {} }), json(200, { deleted: 3 }), json(200, { chat: {} }), json(200, { chat: {} })];
  await fetchLimits();
  assert.deepEqual(await clearAiData({ what: 'shared' }), { deleted: 3 });
  await adminChat(2, 9);
  await adminChat(2, 9, 641);
  assert.deepEqual(calls.map((c) => c.url), ['/api/ai/admin/limits', '/api/ai/admin/clear', '/api/ai/admin/chat?user=2&id=9', '/api/ai/admin/chat?user=2&id=9&at=641']);
  assert.deepEqual(calls[1]?.body, { what: 'shared' });
});
