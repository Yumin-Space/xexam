// AI settings (v0.8): the fields of each provider, the default address when switching, the gateway's API format, origins
// and the key re-entry rule, the header clash (typed and saved keys), the dirty flag, what a Save sends, when AI can be
// enabled, the state chip, the http:// warning, the codes a user who is not an administrator sees, and the safety limits.
// Round 2: the cause the status panel names when ai-service is not connected, and the models that are probably not chat
// models. Only example.com / example.test addresses and obviously fake keys. Run with `node --experimental-strip-types --test`.
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  aiServiceOf, barPercent, canEnable, changesFingerprint, connUpdate, DEFAULT_LIMITS, defaultUrl, fieldOfError, fieldsOf, formOf,
  CAP_KEYS, allFine, connCollapsed, convTotals, hostOf, modelCollapsed, LENGTH_KEYS, AI_TABS, tabOfHash, hashOfTab, blockTag, callsFigures, todayCell, rangeOf, headerClash, httpWarning, inferFormat, isDirty, keyAfter, LIMIT_KEYS, LIMIT_RANGES, limitIssue, limitProblem, limitsFormOf, limitsUpdate, SERVER_KEYS,
  AS_KEYS, BANK_KEYS, GEN_KEYS, IMPORT_KEYS, compBad, compTotal, genCallLive, perCall,
  modelKind, needsReentry, filterModels, orderModels, originOf, requiredKeysSaved, retentionOf, serviceCause, shownCode, stateChip,
  connectionSaved, focusAfterClear, savedToastKey, statusChips, SVC_WAIT_MS, testHint, testLineOf, urlAfterSwitch, type ConnForm,
} from './ai-settings.ts';
import type { AiService, AiSettings } from '../types/api.ts';

// (the service sends the providers' real default addresses; the test uses placeholders)
const DEFAULTS = { openai: 'https://openai.example.com/v1', anthropic: 'https://anthropic.example.com' };
const NONE = { set: false, hint: '', unreadable: false };
const SET = { set: true, hint: '…1234', unreadable: false };

const saved = (o: Partial<AiSettings> = {}): AiSettings => ({
  provider: 'openai', baseUrl: 'https://api.example.com/v1', format: 'openai', gatewayAuth: 'bearer', model: '', manualModel: false,
  enabled: false, verified: false, verifiedAt: null, retentionDays: 90, keys: { apiKey: NONE, gatewayKey: NONE }, keyProblem: null,
  updatedAt: 0, updatedBy: '', defaults: DEFAULTS, ...o,
});

const typed = (s: AiSettings, o: Partial<ConnForm> = {}): ConnForm => ({ ...formOf(s), ...o });

test('the fields of each provider', () => {
  assert.deepEqual(fieldsOf('openai'), { gateway: false, slots: ['apiKey'], required: [] });
  assert.deepEqual(fieldsOf('anthropic'), { gateway: false, slots: ['apiKey'], required: ['apiKey'] });
  assert.deepEqual(fieldsOf('gateway'), { gateway: true, slots: ['gatewayKey', 'apiKey'], required: ['gatewayKey'] });
});

test('switching the provider replaces the address only when it is empty or the old default', () => {
  assert.equal(defaultUrl('gateway', DEFAULTS), '');
  assert.equal(urlAfterSwitch('', 'openai', 'anthropic', DEFAULTS), DEFAULTS.anthropic);
  assert.equal(urlAfterSwitch('https://openai.example.com/v1/', 'openai', 'anthropic', DEFAULTS), DEFAULTS.anthropic);
  assert.equal(urlAfterSwitch('https://llm.example.com/v1', 'openai', 'anthropic', DEFAULTS), 'https://llm.example.com/v1');
  assert.equal(urlAfterSwitch(DEFAULTS.anthropic, 'anthropic', 'gateway', DEFAULTS), '');
});

test('a gateway address suggests its API format by the last path segment', () => {
  assert.equal(inferFormat('https://ai-gateway.example.com/openai'), 'openai');
  assert.equal(inferFormat('https://ai-gateway.example.com/anthropic/'), 'anthropic');
  assert.equal(inferFormat('https://ai-gateway.example.com/Anthropic'), 'anthropic');
  assert.equal(inferFormat('https://ai-gateway.example.com/route/workers-ai'), 'openai');
  assert.equal(inferFormat('not a url /anthropic'), 'anthropic');
});

test('origins: scheme, lower-case host and the default port filled in', () => {
  assert.equal(originOf('https://API.example.com/v1'), 'https://api.example.com:443');
  assert.equal(originOf('https://api.example.com:443/x'), 'https://api.example.com:443');
  assert.equal(originOf('http://ai-stub:9000/v1'), 'http://ai-stub:9000');
  assert.equal(originOf('http://localhost/v1'), 'http://localhost:80');
  assert.equal(originOf('ftp://example.com'), null);
  assert.equal(originOf('nonsense'), null);
});

test('key re-entry: a provider or origin change needs the saved keys typed again; a path change on one origin does not', () => {
  const s = saved({ provider: 'gateway', baseUrl: 'https://ai-gateway.example.com/openai', keys: { apiKey: SET, gatewayKey: SET } });
  assert.equal(needsReentry(s, typed(s)), false);
  assert.equal(needsReentry(s, typed(s, { baseUrl: 'https://AI-GATEWAY.example.com:443/anthropic' })), false, 'path only');
  assert.equal(needsReentry(s, typed(s, { baseUrl: 'https://other.example.com/openai' })), true);
  assert.equal(needsReentry(s, typed(s, { baseUrl: 'http://ai-gateway.example.com/openai' })), true, 'scheme');
  assert.equal(needsReentry(s, typed(s, { provider: 'openai' })), true);

  // the mirror of what the service requires: every saved key slot is in the update (typed, or null when left empty)
  const moved = typed(s, { baseUrl: 'https://other.example.com/openai' });
  assert.deepEqual(connUpdate(s, moved), { baseUrl: 'https://other.example.com/openai', apiKey: null, gatewayKey: null });
  const retyped = typed(s, { baseUrl: 'https://other.example.com/openai', keys: { apiKey: '', gatewayKey: ' gw-demo-0000-5678 ' } });
  assert.deepEqual(connUpdate(s, retyped), { baseUrl: 'https://other.example.com/openai', gatewayKey: 'gw-demo-0000-5678', apiKey: null });
  assert.equal(keyAfter(s, moved, 'gatewayKey'), false);
  assert.equal(keyAfter(s, retyped, 'gatewayKey'), true);
  // a path-only change keeps the keys and sends none
  assert.deepEqual(connUpdate(s, typed(s, { baseUrl: 'https://ai-gateway.example.com/anthropic', format: 'anthropic' })),
    { baseUrl: 'https://ai-gateway.example.com/anthropic', format: 'anthropic' });
});

test('a Save sends only what changed; "Remove this key" and a slot the new provider does not use send null', () => {
  const s = saved({ keys: { apiKey: SET, gatewayKey: NONE } });
  assert.deepEqual(connUpdate(s, typed(s)), {});
  assert.deepEqual(connUpdate(s, typed(s, { keys: { apiKey: 'sk-demo-0000-9999', gatewayKey: '' } })), { apiKey: 'sk-demo-0000-9999' });
  assert.deepEqual(connUpdate(s, typed(s, { remove: { apiKey: true, gatewayKey: false } })), { apiKey: null });
  assert.deepEqual(connUpdate(s, typed(s, { baseUrl: 'https://api.example.com/v1/' })), {}, 'a trailing slash is no change');
  // gateway -> openai: the gateway key is not used any more
  const g = saved({ provider: 'gateway', baseUrl: 'https://ai-gateway.example.com/openai', keys: { apiKey: NONE, gatewayKey: SET } });
  const toOpenai = typed(g, { provider: 'openai', baseUrl: 'https://ai-gateway.example.com/openai', keys: { apiKey: 'sk-demo-0000-1111', gatewayKey: '' } });
  assert.deepEqual(connUpdate(g, toOpenai), { provider: 'openai', apiKey: 'sk-demo-0000-1111', gatewayKey: null });
  // openai -> gateway: the format and the auth scheme are sent with the provider
  const toGateway = typed(s, { provider: 'gateway', baseUrl: 'https://ai-gateway.example.com/openai', gatewayAuth: 'cf-aig-authorization', keys: { apiKey: '', gatewayKey: 'gw-demo-0000-2222' } });
  assert.deepEqual(connUpdate(s, toGateway), {
    provider: 'gateway', baseUrl: 'https://ai-gateway.example.com/openai', format: 'openai', gatewayAuth: 'cf-aig-authorization',
    apiKey: null, gatewayKey: 'gw-demo-0000-2222',
  });
});

test('the header clash, against the typed and the saved provider key', () => {
  const g = saved({ provider: 'gateway', baseUrl: 'https://ai-gateway.example.com/openai', keys: { apiKey: SET, gatewayKey: SET } });
  const f = typed(g);
  assert.equal(headerClash(f, keyAfter(g, f, 'apiKey')), true, 'bearer + OpenAI format + a saved provider key');
  assert.equal(headerClash({ ...f, gatewayAuth: 'cf-aig-authorization' }, true), false);
  assert.equal(headerClash({ ...f, gatewayAuth: 'x-api-key' }, true), false);
  assert.equal(headerClash({ ...f, gatewayAuth: 'x-api-key', format: 'anthropic' }, true), true);
  const removed = { ...f, remove: { apiKey: true, gatewayKey: false } };
  assert.equal(headerClash(removed, keyAfter(g, removed, 'apiKey')), false, 'removing the saved provider key resolves it');
  const noKey = saved({ provider: 'gateway', baseUrl: 'https://ai-gateway.example.com/openai', keys: { apiKey: NONE, gatewayKey: SET } });
  const typedKey = typed(noKey, { keys: { apiKey: 'sk-demo-0000-3333', gatewayKey: '' } });
  assert.equal(headerClash(typedKey, keyAfter(noKey, typedKey, 'apiKey')), true, 'a typed provider key counts too');
  assert.equal(headerClash({ ...typed(saved()), provider: 'openai' }, true), false, 'only gateways');
});

test('dirty: a different provider, address, gateway format or auth, a typed key, a ticked removal', () => {
  const s = saved({ keys: { apiKey: SET, gatewayKey: NONE } });
  assert.equal(isDirty(s, typed(s)), false);
  assert.equal(isDirty(s, typed(s, { baseUrl: ' https://api.example.com/v1/ ' })), false);
  assert.equal(isDirty(s, typed(s, { baseUrl: 'https://api.example.com/v2' })), true);
  assert.equal(isDirty(s, typed(s, { provider: 'anthropic' })), true);
  assert.equal(isDirty(s, typed(s, { keys: { apiKey: 'x', gatewayKey: '' } })), true);
  assert.equal(isDirty(s, typed(s, { remove: { apiKey: true, gatewayKey: false } })), true);
  // the format and auth of a non-gateway provider do not matter
  assert.equal(isDirty(s, typed(s, { format: 'anthropic' })), false);
  const g = saved({ provider: 'gateway' });
  assert.equal(isDirty(g, typed(g, { gatewayAuth: 'x-api-key' })), true);
});

test('which saves switch AI off; required keys; enabling; the state chip', () => {
  assert.equal(changesFingerprint({ apiKey: 'k' }, 'openai'), true);
  assert.equal(changesFingerprint({ model: 'demo-chat-small' }, 'openai'), true);
  assert.equal(changesFingerprint({ retentionDays: 30 }, 'openai'), false);
  assert.equal(changesFingerprint({ gatewayKey: null }, 'openai'), false, 'an unused gateway key');
  assert.equal(changesFingerprint({ gatewayKey: 'k' }, 'gateway'), true);

  assert.equal(requiredKeysSaved(saved()), true, 'OpenAI-compatible needs no key');
  assert.equal(requiredKeysSaved(saved({ provider: 'anthropic' })), false);
  assert.equal(requiredKeysSaved(saved({ provider: 'anthropic', keys: { apiKey: SET, gatewayKey: NONE } })), true);
  assert.equal(requiredKeysSaved(saved({ provider: 'anthropic', keys: { apiKey: { ...SET, unreadable: true }, gatewayKey: NONE } })), false);
  assert.equal(requiredKeysSaved(saved({ provider: 'gateway', keys: { apiKey: NONE, gatewayKey: SET } })), true);

  assert.equal(canEnable(saved({ verified: true })), true);
  assert.equal(canEnable(saved({ verified: false })), false);
  assert.equal(canEnable(saved({ verified: true, keyProblem: 'mismatch' })), false);

  assert.equal(stateChip(saved({ enabled: true, verified: true })), 'on');
  assert.equal(stateChip(saved()), 'off', 'nothing saved yet');
  assert.equal(stateChip(saved({ model: 'demo-chat-small' })), 'unverified');
  assert.equal(stateChip(saved({ keys: { apiKey: SET, gatewayKey: NONE } })), 'unverified');
  assert.equal(stateChip(saved({ model: 'demo-chat-small', verified: true })), 'off', 'tested, switched off');
});

test('the http:// warning: not for this computer or a single-label name', () => {
  assert.equal(httpWarning('http://llm.example.com/v1'), true);
  assert.equal(httpWarning('http://192.0.2.10:11434/v1'), true);
  assert.equal(httpWarning('https://llm.example.com/v1'), false);
  assert.equal(httpWarning('http://localhost:11434/v1'), false);
  assert.equal(httpWarning('http://127.0.0.1:9000/v1'), false);
  assert.equal(httpWarning('http://[::1]:9000/v1'), false);
  assert.equal(httpWarning('http://ai-stub:9000/v1'), false);
  assert.equal(httpWarning('not a url'), false);
});

test('a user who is not an administrator sees setup problems as "not set up correctly"', () => {
  for (const code of ['ai.up.auth', 'ai.up.quota', 'ai.up.notFound', 'ai.up.badRequest', 'ai.up.protocol', 'ai.keyProblem']) {
    assert.equal(shownCode(code, false), 'ai.setup', code);
    assert.equal(shownCode(code, true), code, code);
  }
  for (const code of ['ai.up.timeout', 'ai.up.rateLimit', 'ai.busy', 'ai.disabled']) assert.equal(shownCode(code, false), code);
});

test('test hints and the field of a refused save', () => {
  assert.equal(testHint('ai.up.auth'), 'auth');
  assert.equal(testHint('ai.up.notFound'), 'model');
  assert.equal(testHint('ai.up.badRequest'), 'model');
  assert.equal(testHint('ai.up.connect'), 'connect');
  assert.equal(testHint('ai.up.timeout'), 'slow');
  assert.equal(testHint(null), null);
  assert.equal(fieldOfError({ code: 'ai.badKey', field: 'gatewayKey' }), 'gatewayKey');
  assert.equal(fieldOfError({ code: 'ai.headerClash', field: 'apiKey' }), 'apiKey');
  assert.equal(fieldOfError({ code: 'ai.badUrl' }), 'baseUrl');
  assert.equal(fieldOfError({ code: 'ai.badRetention' }), 'retention');
  assert.equal(fieldOfError({ code: 'ai.internal' }), null);
});

test('retention days and the safety limits form', () => {
  assert.equal(retentionOf('90'), 90);
  assert.equal(retentionOf(' 1 '), 1);
  assert.equal(retentionOf('3650'), 3650);
  for (const bad of ['0', '3651', '', '1.5', '-3', 'abc']) assert.equal(retentionOf(bad), null, bad);

  assert.equal(LIMIT_KEYS.length, 27);
  assert.equal(AS_KEYS.length, 10);
  assert.deepEqual([...CAP_KEYS], ['callsPerMinute', 'callsPerHour', 'callsPerDay']);
  assert.equal(SERVER_KEYS.length, 5);
  assert.deepEqual([...LENGTH_KEYS], ['replyMaxTokens', 'memoryTokens']);
  // FB-5 revised: ten keys, no messageChars; the answer length 3000 tokens, the memory length 1000 tokens
  assert.deepEqual(Object.fromEntries(AS_KEYS.map((k) => [k, DEFAULT_LIMITS[k]])), {
    callsPerMinute: 5, callsPerHour: 20, callsPerDay: 100, replyMaxTokens: 3000, memoryTokens: 1000,
    perUserRunning: 1, globalRunning: 16, chatsPerUser: 1000, messagesPerChat: 400, memoriesPerUser: 50,
  });
  assert.ok(!('messageChars' in DEFAULT_LIMITS));
  assert.deepEqual(LIMIT_RANGES.replyMaxTokens, [1, 250000]);
  assert.deepEqual(LIMIT_RANGES.memoryTokens, [50, 250000]);
  assert.deepEqual(LIMIT_RANGES.genCallTokens, [1000, 250000]);
  assert.equal(DEFAULT_LIMITS.genCallTokens, 20000);
  const form = limitsFormOf(DEFAULT_LIMITS);
  assert.equal(form.replyMaxTokens, '3000');
  assert.equal(form.memoryTokens, '1000');
  assert.equal(limitProblem(form, LIMIT_RANGES), null);
  assert.equal(limitProblem({ ...form, chatsPerUser: '9' }, LIMIT_RANGES), 'chatsPerUser');
  assert.equal(limitProblem({ ...form, globalRunning: '65' }, LIMIT_RANGES), 'globalRunning');
  assert.equal(limitProblem({ ...form, memoryTokens: '2.5' }, LIMIT_RANGES), 'memoryTokens');
  assert.equal(limitProblem({ ...form, perUserRunning: '' }, LIMIT_RANGES), 'perUserRunning');
  assert.deepEqual(limitsUpdate(DEFAULT_LIMITS, { ...form, chatsPerUser: '2000', replyMaxTokens: '2000' }), { chatsPerUser: 2000, replyMaxTokens: 2000 });
  assert.equal(limitProblem({ ...form, replyMaxTokens: '0' }, LIMIT_RANGES), 'replyMaxTokens');
  assert.equal(limitProblem({ ...form, replyMaxTokens: '250001' }, LIMIT_RANGES), 'replyMaxTokens');
  assert.equal(limitProblem({ ...form, replyMaxTokens: '250000' }, LIMIT_RANGES), null);
  assert.equal(limitProblem({ ...form, memoryTokens: '49' }, LIMIT_RANGES), 'memoryTokens');
  assert.deepEqual(limitsUpdate(DEFAULT_LIMITS, form), {});
  // v1.0 caps: ranges 1-100 / 1-2000 / 1-20000
  assert.equal(limitProblem({ ...form, callsPerMinute: '0' }, LIMIT_RANGES), 'callsPerMinute');
  assert.equal(limitProblem({ ...form, callsPerMinute: '101' }, LIMIT_RANGES), 'callsPerMinute');
  assert.equal(limitProblem({ ...form, callsPerDay: '20001' }, LIMIT_RANGES), 'callsPerDay');
  assert.deepEqual(limitIssue({ ...form, callsPerHour: '2001' }, LIMIT_RANGES), { key: 'callsPerHour', kind: 'range' });
  assert.deepEqual(limitsUpdate(DEFAULT_LIMITS, { ...form, callsPerDay: '250' }), { callsPerDay: 250 });
  // order: hour >= minute, day >= hour (equal is fine)
  assert.deepEqual(limitIssue({ ...form, callsPerMinute: '30' }, LIMIT_RANGES), { key: 'callsPerHour', kind: 'order' });
  assert.deepEqual(limitIssue({ ...form, callsPerHour: '200', callsPerDay: '150' }, LIMIT_RANGES), { key: 'callsPerDay', kind: 'order' });
  assert.equal(limitIssue({ ...form, callsPerMinute: '20', callsPerHour: '20', callsPerDay: '20' }, LIMIT_RANGES), null);
  // a range problem is reported before an order problem
  assert.deepEqual(limitIssue({ ...form, callsPerMinute: '30', chatsPerUser: '9' }, LIMIT_RANGES), { key: 'chatsPerUser', kind: 'range' });
  assert.equal(barPercent(0, 10), 0);
  assert.equal(barPercent(10, 10), 100);
  assert.equal(barPercent(1, 1000), 2, 'a sliver for anything above 0');
});

test('filterModels: substring in any case, everything when the query is empty', () => {
  const all = ['openai/gpt-oss-20b', 'meta/llama-3.1-8b-instruct', 'mistralai/mistral-7b-instruct-v0.3'];
  assert.deepEqual(filterModels(all, ''), all);
  assert.deepEqual(filterModels(all, '  '), all);
  assert.deepEqual(filterModels(all, 'INSTRUCT'), ['meta/llama-3.1-8b-instruct', 'mistralai/mistral-7b-instruct-v0.3']);
  assert.deepEqual(filterModels(all, 'gpt'), ['openai/gpt-oss-20b']);
  assert.deepEqual(filterModels(all, 'nothing'), []);
});

test('connectionSaved: only after a save or a saved key', () => {
  assert.equal(connectionSaved(saved()), false);
  assert.equal(connectionSaved(saved({ updatedBy: 'admin' })), true);
  assert.equal(connectionSaved(saved({ keys: { apiKey: SET, gatewayKey: NONE } })), true);
  // A save of the chat retention alone also records who saved: the factory connection (OpenAI-compatible at its default
  // address, no key) still does not count, so nothing is listed from the default address by itself
  assert.equal(connectionSaved(saved({ updatedBy: 'admin', baseUrl: DEFAULTS.openai })), false);
  assert.equal(connectionSaved(saved({ updatedBy: 'admin', baseUrl: DEFAULTS.openai + '/' })), false);
  assert.equal(connectionSaved(saved({ updatedBy: 'admin', baseUrl: DEFAULTS.openai, keys: { apiKey: SET, gatewayKey: NONE } })), true);
  assert.equal(connectionSaved(saved({ updatedBy: 'admin', provider: 'anthropic', baseUrl: DEFAULTS.anthropic })), true);
});

test('statusChips: connection, model and test each on their own, then AI on / off', () => {
  const keys = { apiKey: { set: true, hint: '1234', unreadable: false }, gatewayKey: { set: false, hint: '', unreadable: false } };
  const base = { provider: 'openai' as const, keys, baseUrl: 'https://api.example.com/v1', model: 'm', verified: false, enabled: false, updatedBy: 'admin', defaults: DEFAULTS };
  assert.deepEqual(statusChips(base, false), { conn: 'ok', model: 'ok', test: 'none', ai: 'off' });
  assert.deepEqual(statusChips(base, true), { conn: 'ok', model: 'ok', test: 'fail', ai: 'off' });
  assert.deepEqual(statusChips({ ...base, verified: true, enabled: true }, true), { conn: 'ok', model: 'ok', test: 'ok', ai: 'on' });
  assert.equal(statusChips({ ...base, model: '' }, false).model, 'none');
  assert.equal(statusChips({ ...base, baseUrl: '' }, false).conn, 'none');
  // A fresh instance (nobody saved, no key) shows the defaults but not a saved connection
  assert.equal(statusChips({ ...base, updatedBy: '', keys: { ...keys, apiKey: { set: false, hint: '', unreadable: false } } }, false).conn, 'none');
  assert.equal(statusChips({ ...base, updatedBy: '' }, false).conn, 'ok', 'a saved key counts');
  // Anthropic needs its key (an OpenAI-compatible server may run without one)
  assert.equal(statusChips({ ...base, provider: 'anthropic', keys: { ...keys, apiKey: { set: false, hint: '', unreadable: false } } }, false).conn, 'none');
});

// ---------------------------------------------------------------- round 2

test('modelKind: the likely non-chat models of the contract, and the chat models it must leave alone', () => {
  const marked: Array<[string, string]> = [
    ['text-embedding-3-small', 'embedding'], ['@cf/baai/bge-base-en-v1.5', 'embedding'], ['nomic-embed-text', 'embedding'],
    ['bge-reranker-v2-m3', 'rerank'], ['@cf/meta/llama-guard-3-8b', 'guard'], ['omni-moderation-latest', 'guard'],
    ['@cf/black-forest-labs/flux-1-schnell', 'image'], ['@cf/openai/whisper', 'audio'], ['@cf/microsoft/resnet-50', 'vision'],
    ['demo-embedding', 'embedding'],
    // more of the rules (made-up ids)
    ['Text-Embedding-Large', 'embedding'], ['gte-small', 'embedding'], ['multilingual-e5-large', 'embedding'],
    ['demo-safety-check', 'guard'], ['demo-shield-2b', 'guard'], ['demo-classifier', 'guard'],
    ['stable-diffusion-xl-base', 'image'], ['dall-e-3', 'image'], ['demo-dalle', 'image'], ['imagen-4', 'image'], ['demo:sdxl', 'image'],
    ['tts-1', 'audio'], ['demo/text-to-speech', 'audio'], ['demo-transcribe', 'audio'],
    ['demo-pdf-parser', 'parse'], ['demo@ocr', 'parse'],
    ['google/vit-base-patch16', 'vision'], ['demo-yolo-v8', 'vision'], ['clip-vit-large', 'vision'], ['demo/detr-resnet-50', 'vision'],
    ['demo-image-to-text', 'vision'],
  ];
  for (const [id, kind] of marked) assert.equal(modelKind(id), kind, id);
  const chat = [
    'gpt-4o', 'gpt-oss-20b', '@cf/openai/gpt-oss-20b', '@cf/meta/llama-3.2-11b-vision-instruct', 'qwen2.5-vl-72b-instruct',
    'claude-sonnet-4-5', 'deepseek-r1', 'mistral-small-3.1-24b-instruct', 'demo-chat-small', 'demo-reasoning',
    // a segment rule needs a whole segment: these only contain the letters
    'demo-fluxion', 'demo-vitamin', 'gte5', 'ttsx', 'demo-clipper-chat', 'demo-vision-chat',
  ];
  for (const id of chat) assert.equal(modelKind(id), null, id);
});

test('orderModels: chat models first, then the marked ones, each group in the order of the list; nothing is dropped', () => {
  const list = ['text-embedding-3-small', 'gpt-4o', '@cf/meta/llama-guard-3-8b', 'demo-chat-small', 'bge-reranker-v2-m3', 'demo-reasoning'];
  assert.deepEqual(orderModels(list), [
    'gpt-4o', 'demo-chat-small', 'demo-reasoning', 'text-embedding-3-small', '@cf/meta/llama-guard-3-8b', 'bge-reranker-v2-m3',
  ]);
  assert.deepEqual(orderModels([]), []);
  assert.equal(orderModels(list).length, list.length);
  // filtering by typing works on the ordered list as before
  assert.deepEqual(filterModels(orderModels(list), 'demo'), ['demo-chat-small', 'demo-reasoning']);
});

const svc = (o: Partial<AiService> = {}): AiService => ({
  connected: false, stopped: false, lastSeen: null, version: null, enabled: false, link: true, ...o,
});

test('serviceCause: every branch of contracts 2.9', () => {
  // 1 the main service failed
  assert.equal(serviceCause({ state: 'failed', probe: null, waitedMs: 0 }), 'backendDown');
  assert.equal(serviceCause({ state: 'failed', probe: 'ok', waitedMs: 0 }), 'backendDown');
  // 2 connected (whatever the probe says)
  assert.equal(serviceCause({ state: { aiService: svc({ connected: true, lastSeen: 1, version: '0.8', enabled: true }) }, probe: null, waitedMs: 0 }), 'connected');
  // 3 no internal token in the main service
  assert.equal(serviceCause({ state: { aiService: svc({ link: false }) }, probe: 'down', waitedMs: 0 }), 'linkOff');
  // 4 the probe found nothing: stopped (its last report said so) or not running
  assert.equal(serviceCause({ state: { aiService: svc({ stopped: true, lastSeen: 100 }) }, probe: 'down', waitedMs: 0 }), 'stopped');
  assert.equal(serviceCause({ state: { aiService: svc({ lastSeen: 100 }) }, probe: 'down', waitedMs: 0 }), 'notRunning');
  assert.equal(serviceCause({ state: { aiService: svc() }, probe: 'down', waitedMs: 0 }), 'notRunning');
  // 5 ai-service refuses (it cannot verify the main service's statement)
  assert.equal(serviceCause({ state: { aiService: svc() }, probe: 'refused', waitedMs: 0 }), 'notConnected');
  // 6 ai-service answers: no report yet and less than a minute → waiting; else not connected
  assert.equal(serviceCause({ state: { aiService: svc() }, probe: 'ok', waitedMs: 0 }), 'waiting');
  assert.equal(serviceCause({ state: { aiService: svc() }, probe: 'ok', waitedMs: SVC_WAIT_MS - 1 }), 'waiting');
  assert.equal(serviceCause({ state: { aiService: svc() }, probe: 'ok', waitedMs: SVC_WAIT_MS }), 'notConnected');
  assert.equal(serviceCause({ state: { aiService: svc({ lastSeen: 100 }) }, probe: 'ok', waitedMs: 0 }), 'notConnected');
  // not probed yet
  assert.equal(serviceCause({ state: { aiService: svc({ stopped: true }) }, probe: null, waitedMs: 0 }), 'stopped');
  assert.equal(serviceCause({ state: { aiService: svc() }, probe: null, waitedMs: 0 }), 'notConnected');
});

test('aiServiceOf: the aiService of an auth state answer, or null (not an administrator, an older main service)', () => {
  const full = { connected: true, stopped: false, lastSeen: 1_800_000_000, version: '0.8', enabled: true, link: true };
  assert.deepEqual(aiServiceOf({ user: { id: 1 }, library: 'x', aiAvailable: true, aiService: full }), full);
  assert.equal(aiServiceOf({ user: { id: 2 }, library: 'x', aiAvailable: false }), null);
  assert.equal(aiServiceOf(null), null);
  assert.equal(aiServiceOf({ aiService: { connected: 'yes' } }), null);
  assert.deepEqual(aiServiceOf({ aiService: { connected: false } }), svc({ link: false }));
  assert.deepEqual(aiServiceOf({ aiService: { connected: false, lastSeen: 'x', version: '', link: true } }), svc());
});

test('testLineOf: the result line follows the AI+ feature switch (never "turn it on" next to a switch that is on)', () => {
  const ok = { ok: true, code: null, latencyMs: 81 };
  assert.deepEqual(testLineOf(null, false), { kind: 'none' });
  assert.deepEqual(testLineOf(ok, false), { kind: 'ok', key: 'ai.set.testOk', ms: 81 });
  // switched on after the test: the line changes by itself
  assert.deepEqual(testLineOf(ok, true), { kind: 'ok', key: 'ai.set.testOkOn', ms: 81 });
  // and back when it is switched off again
  assert.deepEqual(testLineOf(ok, false), { kind: 'ok', key: 'ai.set.testOk', ms: 81 });
  // a failed test: "stays on" only while it is on
  const bad = { ok: false, code: 'ai.up.timeout', latencyMs: 30000 };
  assert.deepEqual(testLineOf(bad, true), { kind: 'fail', code: 'ai.up.timeout', hint: 'slow', keepOn: true });
  assert.deepEqual(testLineOf(bad, false), { kind: 'fail', code: 'ai.up.timeout', hint: 'slow', keepOn: false });
  assert.deepEqual(testLineOf({ ok: false, code: null, latencyMs: 0 }, false), { kind: 'fail', code: null, hint: null, keepOn: false });
});

test('savedToastKey: a save that switched the AI+ feature off says so', () => {
  assert.equal(savedToastKey(true), 'ai.set.savedOff');
  assert.equal(savedToastKey(false), 'ai.set.saved');
});

test('focusAfterClear: the focus stays inside the viewer dialog after a delete', () => {
  // chats left: the first one; none left: the Chats tab
  assert.equal(focusAfterClear('chat', [7, 9]), 'vw-c-7');
  assert.equal(focusAfterClear('chat', []), 'vw-tab-chats');
  assert.equal(focusAfterClear('chats', []), 'vw-tab-chats');
  assert.equal(focusAfterClear('chats', [3]), 'vw-c-3');
  // memories: their tab, whatever is left
  assert.equal(focusAfterClear('memories', [3]), 'vw-tab-memories');
  assert.equal(focusAfterClear('memories', []), 'vw-tab-memories');
});

test('lengths are tokens: no language factor, one range for the three length settings', () => {
  assert.deepEqual(rangeOf(undefined, 'replyMaxTokens'), [1, 250000]);
  assert.deepEqual(rangeOf({ replyMaxTokens: [1, 500] }, 'replyMaxTokens'), [1, 500]);
});

test('the category tabs live in the hash; a missing or unknown value is the first tab', () => {
  assert.deepEqual([...AI_TABS], ['feature', 'users', 'usage', 'safety', 'gen']);
  assert.equal(tabOfHash('#/ai-settings?tab=gen'), 'gen');
  assert.equal(tabOfHash('#/ai-settings'), 'feature');
  assert.equal(tabOfHash('#/ai-settings?tab=users'), 'users');
  assert.equal(tabOfHash('#/ai-settings?tab=usage'), 'usage');
  assert.equal(tabOfHash('#/ai-settings?tab=safety'), 'safety');
  assert.equal(tabOfHash('#/ai-settings?tab=generate'), 'feature');
  assert.equal(tabOfHash('#/ai-settings?tab='), 'feature');
  assert.equal(tabOfHash('#/ai-settings?x=1&tab=users'), 'users');
  for (const k of AI_TABS) assert.equal(tabOfHash(hashOfTab(k)), k);
  assert.equal(hashOfTab('feature'), '#/ai-settings?tab=feature');
});

test('the calls line, the block tag and the today cell of an AI+ user', () => {
  const calls = { used: { minute: 2, hour: 7, day: 37 }, blocked: null };
  assert.deepEqual(callsFigures(calls, { callsPerMinute: 5, callsPerHour: 20, callsPerDay: 100 }), { minute: '2/5', hour: '7/20', day: '37/100' });
  assert.deepEqual(callsFigures(calls, undefined), { minute: '2/5', hour: '7/20', day: '37/100' });
  assert.equal(callsFigures(undefined, undefined), null);
  assert.equal(blockTag(calls), null);
  assert.equal(blockTag(undefined), null);
  assert.deepEqual(blockTag({ blocked: { cap: 'minute', limit: 5, retryAt: 100 } }), { kind: 'timed', retryAt: 100 });
  assert.deepEqual(blockTag({ blocked: { cap: 'hour', limit: 20, retryAt: 200 } }), { kind: 'timed', retryAt: 200 });
  assert.deepEqual(blockTag({ blocked: { cap: 'day', limit: 100, retryAt: 300 } }), { kind: 'today', retryAt: 300 });
  assert.equal(todayCell(37, 100), '37 / 100');
  assert.equal(todayCell(0, undefined), '0 / 100');
  assert.equal(todayCell(undefined, 100), '–');
});

test('summaries once saved and tested (AS-4): the connection, the model, the status line', () => {
  assert.equal(hostOf('https://llm.example.com:8443/v1/'), 'llm.example.com:8443');
  assert.equal(hostOf('https://llm.example.com/v1'), 'llm.example.com');
  assert.equal(hostOf('not an address/'), 'not an address');
  const ready = saved({ keys: { apiKey: SET, gatewayKey: NONE }, verified: true, model: 'm-1', enabled: true });
  assert.equal(connCollapsed(ready, false, false), true);
  assert.equal(connCollapsed(ready, true, false), false, 'typed changes open it');
  assert.equal(connCollapsed(ready, false, true), false, 'the pencil opens it');
  assert.equal(connCollapsed({ ...ready, verified: false }, false, false), false, 'not tested');
  assert.equal(connCollapsed({ ...ready, keyProblem: 'mismatch' }, false, false), false);
  assert.equal(connCollapsed(saved({ verified: true }), false, false), false, 'the untouched factory connection is not a saved one');
  assert.equal(modelCollapsed({ verified: true, model: 'm-1' }, 'm-1', false), true);
  assert.equal(modelCollapsed({ verified: true, model: 'm-1' }, '', false), true);
  assert.equal(modelCollapsed({ verified: true, model: 'm-1' }, 'm-2', false), false, 'an unsaved model is being edited');
  assert.equal(modelCollapsed({ verified: true, model: '' }, '', false), false);
  assert.equal(modelCollapsed({ verified: false, model: 'm-1' }, 'm-1', false), false);
  assert.equal(modelCollapsed({ verified: true, model: 'm-1' }, 'm-1', true), false);
  const chips = { conn: 'ok', model: 'ok', test: 'ok', ai: 'on' } as const;
  assert.equal(allFine(chips, false), true);
  assert.equal(allFine(chips, true), false);
  assert.equal(allFine({ ...chips, ai: 'off' }, false), false);
  assert.equal(allFine({ ...chips, test: 'fail' }, false), false);
});

test('the conversation figures that go with the usage', () => {
  assert.deepEqual(convTotals([]), { chats: 0, messages: 0, memories: 0 });
  assert.deepEqual(convTotals([{ chats: 2, messages: 10, memories: 1 }, { chats: 3, messages: 5, memories: 0 }]), { chats: 5, messages: 15, memories: 1 });
});

test('v1.1: the 25 keys, the ten + two of the AI banks tab, defaults and ranges of GS-1 / IS-1', () => {
  assert.deepEqual([...GEN_KEYS], ['genPerDay', 'genAdminsExempt', 'genMinSource', 'genMinWrong', 'genBankSingle', 'genBankMulti', 'genWrongSingle', 'genWrongMulti', 'genCallTokens', 'genRunning', 'genMaxCalls', 'genUnlimited']);
  assert.deepEqual([...IMPORT_KEYS], ['importMaxMb', 'importMaxQuestions', 'importMaxCalls', 'recognizeMaxCalls', 'importUnlimited']);
  assert.equal(BANK_KEYS.length, 17);
  assert.deepEqual([...LIMIT_KEYS], [...AS_KEYS, ...GEN_KEYS, ...IMPORT_KEYS]);
  assert.equal(new Set(LIMIT_KEYS).size, 27);
  assert.deepEqual(GEN_KEYS.map((k) => DEFAULT_LIMITS[k]), [3, 1, 30, 10, 14, 6, 14, 6, 20000, 2, 15, 1]);
  assert.deepEqual(IMPORT_KEYS.map((k) => DEFAULT_LIMITS[k]), [20, 50, 25, 20, 1]);
  assert.deepEqual(GEN_KEYS.map((k) => LIMIT_RANGES[k]), [[0, 100], [0, 1], [5, 2000], [1, 500], [0, 100], [0, 100], [0, 100], [0, 100], [1000, 250000], [1, 8], [8, 40], [0, 1]]);
  assert.deepEqual(IMPORT_KEYS.map((k) => LIMIT_RANGES[k]), [[1, 20], [1, 50], [8, 40], [8, 40], [0, 1]]);
  assert.equal(limitIssue({ ...limitsFormOf(DEFAULT_LIMITS), genUnlimited: '2' }, LIMIT_RANGES)?.key, 'genUnlimited');
  assert.equal(limitIssue({ ...limitsFormOf({ ...DEFAULT_LIMITS, importUnlimited: 1, genUnlimited: 1 }) }, LIMIT_RANGES), null);
  const form = limitsFormOf(DEFAULT_LIMITS);
  assert.equal(limitIssue(form, LIMIT_RANGES), null);
  assert.deepEqual(limitIssue({ ...form, importMaxMb: '21' }, LIMIT_RANGES), { key: 'importMaxMb', kind: 'range' });
  assert.deepEqual(limitIssue({ ...form, genCallTokens: '999' }, LIMIT_RANGES), { key: 'genCallTokens', kind: 'range' });
  // the tab saves only what changed, of the twelve
  assert.deepEqual(limitsUpdate(DEFAULT_LIMITS, { ...form, genPerDay: '5', importMaxQuestions: '30' }), { genPerDay: 5, importMaxQuestions: 30 });
});

test('v1.1: the composition rule (A0): each mode 1 .. 100 in total, the bank mode first, the field is the multi key', () => {
  const form = limitsFormOf(DEFAULT_LIMITS);
  assert.deepEqual(limitIssue({ ...form, genBankSingle: '0', genBankMulti: '0' }, LIMIT_RANGES), { key: 'genBankMulti', kind: 'comp' });
  assert.deepEqual(limitIssue({ ...form, genWrongSingle: '60', genWrongMulti: '41' }, LIMIT_RANGES), { key: 'genWrongMulti', kind: 'comp' });
  assert.deepEqual(limitIssue({ ...form, genBankSingle: '0', genBankMulti: '0', genWrongSingle: '0', genWrongMulti: '0' }, LIMIT_RANGES), { key: 'genBankMulti', kind: 'comp' });
  assert.equal(limitIssue({ ...form, genBankSingle: '0', genBankMulti: '1' }, LIMIT_RANGES), null);
  assert.equal(limitIssue({ ...form, genBankSingle: '100', genBankMulti: '0' }, LIMIT_RANGES), null);
  assert.equal(limitIssue({ ...form, genBankSingle: '100', genBankMulti: '1' }, LIMIT_RANGES)?.kind, 'comp');
  assert.equal(compTotal('14', '6'), 20);
  assert.ok(Number.isNaN(compTotal('14', 'x')));
  assert.equal(compBad('14', '6'), false);
  assert.equal(compBad('0', '0'), true);
  assert.equal(compBad('101', '0'), true);
  assert.equal(compBad('', '5'), true);
});

test('v1.1: perCall vectors are the Rust ones; the live line of the length field', () => {
  assert.equal(perCall(20000, null), 5);
  assert.equal(perCall(20000, 4096), 1);
  assert.equal(perCall(20000, 8192), 2);
  assert.equal(perCall(1000, null), 1);
  assert.equal(perCall(250000, null), 5);
  assert.equal(perCall(7200, null), 3);
  assert.deepEqual(genCallLive('20000', LIMIT_RANGES), { q: 5 });
  assert.deepEqual(genCallLive('1000', LIMIT_RANGES), { q: 1 });
  assert.equal(genCallLive('999', LIMIT_RANGES), null);
  assert.equal(genCallLive('abc', LIMIT_RANGES), null);
});

test('A-OWNER-48: the two unlimited switches are on by default', () => {
  assert.equal(DEFAULT_LIMITS.genUnlimited, 1);
  assert.equal(DEFAULT_LIMITS.importUnlimited, 1);
  assert.equal(limitsFormOf({} as never).genUnlimited, '1');
});

test('every length key is sent under the token name the server knows', () => {
  const form = limitsFormOf(DEFAULT_LIMITS);
  assert.deepEqual(limitsUpdate(DEFAULT_LIMITS, { ...form, replyMaxTokens: '4000' }), { replyMaxTokens: 4000 });
  assert.deepEqual(limitsUpdate(DEFAULT_LIMITS, { ...form, memoryTokens: '1500' }), { memoryTokens: 1500 });
  assert.deepEqual(limitsUpdate(DEFAULT_LIMITS, { ...form, genCallTokens: '30000' }), { genCallTokens: 30000 });
  for (const k of Object.keys(DEFAULT_LIMITS)) assert.ok(!/Chars$/.test(k), k);
});
