// User management's API wrappers (v0.8 round 2): every answer that carries the list also says whether AI+ is available,
// and every wrapper hands both to the page, so the AI+ switches always follow the newest answer. A refused AI+ switch
// (409 ai.serviceOff) is an error, never a list. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import { afterEach, beforeEach, test } from 'node:test';
import { errorText } from '../i18n/errors.ts';
import { setUiLang } from '../i18n/index.ts';
import { isServiceOff, MSG_AI_SERVICE_OFF } from '../domain/user-admin.ts';
import type { UserRow } from '../types/api.ts';
import { ApiError } from './client.ts';
import { createUser, deleteUser, fetchUsers, resetUserPassword, unlockUser, updateUser, usersListOf, type UsersList } from './users.ts';

interface Call {
  url: string;
  method: string;
  body: unknown;
}

let calls: Call[] = [];
let replies: Array<() => Response> = [];
const realFetch = globalThis.fetch;

const json = (status: number, body: unknown) => (): Response =>
  new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json; charset=utf-8' } });

const row = (o: Partial<UserRow> = {}): UserRow => ({
  id: 2, username: 'demo-user', role: 'user', disabled: false, createdAt: '2026-01-01 10:00:00', lastLoginAt: null,
  mustChangePassword: false, records: 0, banks: [], newBanks: false, canManageBanks: false, ai: false, ownedBanks: [], lock: null,
  aiSuspended: false, ...o,
});

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
  setUiLang('en');
});

test('usersListOf: aiAvailable and aiSuspended count only when exactly true', () => {
  assert.deepEqual(usersListOf({ users: [row()], aiAvailable: true }), { users: [row()], aiAvailable: true });
  // An answer without the round-2 fields (an older main service): no AI+ switches, nobody suspended
  const old = usersListOf(JSON.parse('{"users":[{"id":2,"username":"demo-user","role":"user","ai":true}]}'));
  assert.equal(old.aiAvailable, false);
  assert.equal(old.users[0]?.aiSuspended, false);
  assert.equal(old.users[0]?.ai, true);
  const odd = usersListOf(JSON.parse('{"users":[{"id":2,"aiSuspended":"yes"}],"aiAvailable":1}'));
  assert.equal(odd.aiAvailable, false);
  assert.equal(odd.users[0]?.aiSuspended, false);
  assert.equal(usersListOf({ users: [row({ aiSuspended: true })], aiAvailable: false }).users[0]?.aiSuspended, true);
});

test('every wrapper takes aiAvailable from its own answer', async () => {
  const runs: Array<[string, () => Promise<UsersList>, string, unknown]> = [
    ['fetch', () => fetchUsers(), '/api/users', undefined],
    ['create', () => createUser({ username: 'demo-new', password: 'demo-pass-1', role: 'user', banks: [], newBanks: false, canManageBanks: false, ai: true }),
      '/api/users/create', { username: 'demo-new', password: 'demo-pass-1', role: 'user', banks: [], newBanks: false, canManageBanks: false, ai: true }],
    ['update', () => updateUser({ id: 2, ai: true }), '/api/users/update', { id: 2, ai: true }],
    ['password', () => resetUserPassword(2, 'demo-pass-2'), '/api/users/password', { id: 2, password: 'demo-pass-2' }],
    ['unlock', () => unlockUser(2), '/api/users/unlock', { id: 2 }],
    ['delete', () => deleteUser(2), '/api/users/delete', { id: 2 }],
  ];
  for (const [name, run, path, body] of runs) {
    for (const available of [true, false]) {
      calls = [];
      const users = [row({ ai: true, aiSuspended: !available })];
      replies = [json(200, { users, aiAvailable: available })];
      const got = await run();
      assert.equal(got.aiAvailable, available, `${name}: aiAvailable ${available}`);
      assert.deepEqual(got.users, users, `${name}: the list`);
      assert.equal(calls.length, 1, `${name}: one request`);
      assert.equal(calls[0]?.url, path, `${name}: path`);
      assert.equal(calls[0]?.method, body === undefined ? 'GET' : 'POST', `${name}: method`);
      if (body !== undefined) assert.deepEqual(calls[0]?.body, body, `${name}: body`);
    }
  }
});

test('a refused AI+ switch (409 ai.serviceOff) rejects with the refusal, in every language', async () => {
  const refusal = { error: MSG_AI_SERVICE_OFF, code: 'ai.serviceOff' };
  for (const [lang, text] of [
    ['en', 'Turn on the AI+ feature in AI settings first.'],
    ['zh', '请先在 AI 设置里开启 AI+ 功能。'],
    ['ja', '先に AI 設定で AI+ 機能をオンにしてください。'],
  ] as const) {
    setUiLang(lang);
    for (const on of [true, false]) {
      replies = [json(409, refusal)];
      const e = await updateUser({ id: 2, ai: on }).then(() => null, (x: unknown) => x);
      assert.ok(e instanceof ApiError, 'an ApiError');
      assert.equal(e.status, 409);
      assert.equal(e.code, 'ai.serviceOff');
      assert.equal(isServiceOff(e), true);
      assert.equal(errorText(e), text, `${lang}: the text of the refusal`);
    }
  }
  setUiLang('en');
  // Creating a user with the AI+ service while AI+ is not available: refused the same way
  replies = [json(409, refusal)];
  const e = await createUser({ username: 'demo-new', password: 'demo-pass-1', role: 'user', banks: [], newBanks: false, canManageBanks: false, ai: true })
    .then(() => null, (x: unknown) => x);
  assert.ok(isServiceOff(e));
  // A taken username is a 409 too, but not this refusal
  replies = [json(409, { error: '用户名已被使用。' })];
  const taken = await createUser({ username: 'demo-user', password: 'demo-pass-1', role: 'user', banks: [], newBanks: false, canManageBanks: false, ai: false })
    .then(() => null, (x: unknown) => x);
  assert.ok(taken instanceof ApiError);
  assert.equal(isServiceOff(taken), false);
});
