// AI settings: the pure rules of the AI settings page. v1.3: the form of one connection and what a save sends (test before
// save; key fields absent = keep, null = remove), the host a key reuse is confirmed for (a plain http address only gets a note),
// the four status cards, the six model slots (an unusable default is a warning, nothing is switched on its own), the merged
// model list, the limits (33 keys in groups). Also: the fields of each provider, the default address when switching, the
// gateway's API format, origins, the header clash, the codes a user who is not an administrator sees, the cause the status
// panel names when ai-service is not connected, and the models that are probably not chat models. Only example.com /
// example.test addresses and obviously fake keys. Run with `node --experimental-strip-types --test`.
import assert from 'node:assert/strict';
import test from 'node:test';
import {
  jobTime, aiServiceOf, AI_TABS, AS_KEYS, barPercent, BANK_KEYS, blankForm, CALLS_KEYS, CAP_KEYS, CHAT_KEYS, compBad, compTotal, CONTENT_KEYS,
  confirmHostOf, connLabel, connPayload, connResult, DEFAULT_LIMITS, defaultUnusable, defaultUrl, fieldOfError, fieldsOf, filterModels,
  firstChoice, focusAfterClear, formOf, formProblem, genCallLive, hashOfTab, headerClash, hostOf, httpWarning, inferFormat, isDirty,
  isRotation, keyAfter, keyLine, LENGTH_KEYS, limitIssue, limitProblem, LIMIT_KEYS, LIMIT_RANGES, limitsFormOf, limitsUpdate,
  modelGroups, modelKind, movesKeys, nameProblem, newDefaultChoices, orderModels, originOf, parseRefKey, perCall, rangeOf, refKey,
  refOf, retentionOf, reusableFrom, serviceCause, SERVER_KEYS, SHARE_KEYS, shownCode, slotModel, slotNote, slotUsing, SLOTS,
  FORMATS, statusCards, suggestName, SUMMARY_KEYS, SVC_WAIT_MS, tabOfHash, testHint, urlAfterSwitch, blockTag, callsFigures, todayCell, convTotals,
} from './ai-settings.ts';
import type { AiConnection, AiModelList, AiPoolModel, AiService, AiSettings } from '../types/api.ts';

// (the service sends the providers' real default addresses; the test uses placeholders)
const DEFAULTS = { openai: 'https://openai.example.com/v1', anthropic: 'https://anthropic.example.com' };
const NONE = { set: false, hint: '', unreadable: false };

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
  assert.equal(inferFormat('https://ai-gateway.example.com/responses'), 'openai', 'Responses is only picked by hand');
  assert.deepEqual([...FORMATS], ['openai', 'anthropic', 'responses']);
});

test('origins: scheme, lower-case host and the default port filled in', () => {
  assert.equal(originOf('https://API.example.com/v1'), 'https://api.example.com:443');
  assert.equal(originOf('https://api.example.com:443/x'), 'https://api.example.com:443');
  assert.equal(originOf('http://ai-stub:9000/v1'), 'http://ai-stub:9000');
  assert.equal(originOf('http://localhost/v1'), 'http://localhost:80');
  assert.equal(originOf('ftp://example.com'), null);
  assert.equal(originOf('nonsense'), null);
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

test('filterModels: substring in any case, everything when the query is empty', () => {
  const all = ['openai/gpt-oss-20b', 'meta/llama-3.1-8b-instruct', 'mistralai/mistral-7b-instruct-v0.3'];
  assert.deepEqual(filterModels(all, ''), all);
  assert.deepEqual(filterModels(all, '  '), all);
  assert.deepEqual(filterModels(all, 'INSTRUCT'), ['meta/llama-3.1-8b-instruct', 'mistralai/mistral-7b-instruct-v0.3']);
  assert.deepEqual(filterModels(all, 'gpt'), ['openai/gpt-oss-20b']);
  assert.deepEqual(filterModels(all, 'nothing'), []);
});

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

test('the conversation figures that go with the usage', () => {
  assert.deepEqual(convTotals([]), { chats: 0, messages: 0, memories: 0 });
  assert.deepEqual(convTotals([{ chats: 2, messages: 10, memories: 1 }, { chats: 3, messages: 5, memories: 0 }]), { chats: 5, messages: 15, memories: 1 });
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


// ---------------------------------------------------------------- v1.3: connections, slots, status cards, merged list, limits

const KEY = (hint = '…ab12') => ({ set: true, hint, unreadable: false });
const conn = (o: Partial<AiConnection> = {}): AiConnection => ({
  id: 1, rev: 3, name: 'main', provider: 'openai', baseUrl: 'https://api.example.com/v1', host: 'api.example.com', format: 'openai',
  gatewayAuth: 'bearer', keys: { apiKey: KEY(), gatewayKey: NONE }, status: 'ok', errorCode: null, testedAt: 1, listsModels: true,
  insecureHttp: false, usedBy: [], ...o,
});
const pool = (id: number, connectionId: number, model: string, o: Partial<AiPoolModel> = {}): AiPoolModel => ({
  id, connectionId, connectionName: connectionId === 1 ? 'main' : 'second', model, manual: false, status: 'ok', errorCode: null,
  testedAt: 1, latencyMs: 400, context: 128000, usedBy: [], ...o,
});
const slot = (modelId: number | null, using: number | null, fallback: 'none' | 'default' = 'none') =>
  ({ modelId, using: using == null ? null : { modelId: using, fallback } });
const v13 = (o: Partial<AiSettings> = {}): AiSettings => ({
  enabled: true, effective: true, state: 'on', pauseReason: null, keyProblem: null, retentionDays: 90, updatedAt: 0, updatedBy: 'admin',
  defaults: DEFAULTS, maxConnections: 8, connections: [conn()], pool: [pool(4, 1, 'demo-chat-small')],
  slots: {
    default: slot(4, 4), chat: slot(null, 4, 'default'), study: slot(null, 4, 'default'), summary: slot(null, 4, 'default'),
    generate: slot(null, 4, 'default'), import: slot(null, 4, 'default'),
  },
  featureSwitches: { chat: true, hint: true, explain: true, summary: true, generate: true, import: true }, ...o,
});

test('a connection form: from a saved one, a blank one, and the name that follows the host', () => {
  const c = conn();
  const f = formOf(c);
  assert.deepEqual(f, {
    name: 'main', provider: 'openai', baseUrl: 'https://api.example.com/v1', format: 'openai', gatewayAuth: 'bearer',
    keys: { apiKey: '', gatewayKey: '' }, remove: { apiKey: false, gatewayKey: false }, reuseFrom: null,
  });
  const b = blankForm(DEFAULTS);
  assert.equal(b.provider, 'openai');
  assert.equal(b.baseUrl, DEFAULTS.openai);
  assert.equal(b.name, 'openai.example.com');
  assert.equal(suggestName('https://gw.example.com:8443/openai'), 'gw.example.com', 'the port is not part of the name');
  assert.equal(suggestName('not an address'), '');
  assert.equal(suggestName(`https://${'a'.repeat(60)}.example.com`).length, 40);
});

test('the name rule: 1 to 40 characters, no control characters', () => {
  assert.equal(nameProblem('main'), false);
  assert.equal(nameProblem('  '), true);
  assert.equal(nameProblem(''), true);
  assert.equal(nameProblem('x'.repeat(40)), false);
  assert.equal(nameProblem('x'.repeat(41)), true);
  assert.equal(nameProblem('a\u0007b'), true);
  assert.equal(formProblem({ name: '' }), 'name');
  assert.equal(formProblem({ name: 'ok' }), null);
});

test('the host a key reuse is confirmed for: host[:port], the port only when it is not the default', () => {
  assert.equal(confirmHostOf('https://API.example.com/v1'), 'api.example.com');
  assert.equal(confirmHostOf('https://api.example.com:443/v1'), 'api.example.com');
  assert.equal(confirmHostOf('http://api.example.com:80'), 'api.example.com');
  assert.equal(confirmHostOf('https://api.example.com:8443/x'), 'api.example.com:8443');
  assert.equal(confirmHostOf('http://gw.example.test:8080'), 'gw.example.test:8080');
  assert.equal(confirmHostOf('ftp://api.example.com'), null);
  assert.equal(confirmHostOf('nonsense'), null);
});

test('a plain http address only gets a note: nothing about it blocks saving or reusing a key', () => {
  const c = conn({ baseUrl: 'http://gw.example.com/v1', host: 'gw.example.com', insecureHttp: true });
  const f = { ...formOf(c), keys: { apiKey: 'sk-demo-0000-1111', gatewayKey: '' } };
  assert.equal(httpWarning(f.baseUrl), true);
  const body = connPayload(c, f, 'gw.example.com');
  assert.equal(body.baseUrl, 'http://gw.example.com/v1');
  assert.equal(body.confirmHost, 'gw.example.com');
  assert.equal(confirmHostOf(f.baseUrl), 'gw.example.com');
});

test('moving keys: a provider or origin change, not a path change on one origin', () => {
  const c = conn();
  assert.equal(movesKeys(c, formOf(c)), false);
  assert.equal(movesKeys(c, { ...formOf(c), baseUrl: 'https://api.example.com/v2' }), false);
  assert.equal(movesKeys(c, { ...formOf(c), baseUrl: 'https://other.example.com/v1' }), true);
  assert.equal(movesKeys(c, { ...formOf(c), baseUrl: 'http://api.example.com/v1' }), true, 'the scheme is part of the origin');
  assert.equal(movesKeys(c, { ...formOf(c), provider: 'anthropic' }), true);
});

test('dirty: a name, provider, address, gateway format or auth, a typed key, a ticked removal; a new connection always', () => {
  const c = conn();
  assert.equal(isDirty(c, formOf(c)), false);
  assert.equal(isDirty(c, { ...formOf(c), name: 'other' }), true);
  assert.equal(isDirty(c, { ...formOf(c), baseUrl: 'https://api.example.com/v1/' }), false, 'a trailing slash is not a change');
  assert.equal(isDirty(c, { ...formOf(c), baseUrl: 'https://api.example.com/v2' }), true);
  assert.equal(isDirty(c, { ...formOf(c), keys: { apiKey: 'sk-demo-0000-2222', gatewayKey: '' } }), true);
  assert.equal(isDirty(c, { ...formOf(c), remove: { apiKey: true, gatewayKey: false } }), true);
  const g = conn({ provider: 'gateway', baseUrl: 'https://ai-gateway.example.com/openai' });
  assert.equal(isDirty(g, { ...formOf(g), gatewayAuth: 'x-api-key' }), true);
  assert.equal(isDirty(null, blankForm(DEFAULTS)), true);
});

test('what a save sends: a new connection, a rotation, a removal, a slot the provider does not use, reuse and its confirmation', () => {
  const c = conn();
  // new: only what is typed, no id / rev
  const n = { ...blankForm(DEFAULTS), name: ' fresh ', keys: { apiKey: ' sk-demo-0000-4444 ', gatewayKey: '' } };
  assert.deepEqual(connPayload(null, n), { name: 'fresh', provider: 'openai', baseUrl: DEFAULTS.openai, apiKey: 'sk-demo-0000-4444' });
  // a saved connection: id + rev; an untouched key is absent (= keep)
  assert.deepEqual(connPayload(c, { ...formOf(c), name: 'renamed' }), { name: 'renamed', provider: 'openai', baseUrl: 'https://api.example.com/v1', id: 1, rev: 3 });
  // a rotation: the new key is a string; removal: null
  const rot = { ...formOf(c), keys: { apiKey: 'sk-demo-0000-5555', gatewayKey: '' } };
  assert.equal(connPayload(c, rot).apiKey, 'sk-demo-0000-5555');
  assert.equal(isRotation(c, rot), true);
  assert.equal(isRotation(c, formOf(c)), false);
  assert.equal(isRotation(null, rot), false);
  assert.equal(isRotation(conn({ keys: { apiKey: NONE, gatewayKey: NONE } }), rot), false, 'a first key is not a rotation');
  assert.equal(connPayload(c, { ...formOf(c), remove: { apiKey: true, gatewayKey: false } }).apiKey, null);
  // a key slot the new provider does not use is removed
  const toG = conn({ provider: 'gateway', keys: { apiKey: KEY(), gatewayKey: KEY() } });
  const toAnth = { ...formOf(toG), provider: 'anthropic' as const };
  const body = connPayload(toG, toAnth);
  assert.equal(body.gatewayKey, null);
  assert.ok(!('apiKey' in body), 'the one key the provider uses stays');
  assert.ok(!('format' in body), 'format and auth only go with a gateway');
  // a gateway sends its format and auth
  const gw = connPayload(null, { ...blankForm(DEFAULTS), provider: 'gateway', baseUrl: 'https://ai-gateway.example.com/openai', format: 'openai', gatewayAuth: 'cf-aig-authorization' });
  assert.equal(gw.format, 'openai');
  assert.equal(gw.gatewayAuth, 'cf-aig-authorization');
  // reuse: only for a new connection, with the confirmed host
  const reuse = { ...blankForm(DEFAULTS), reuseFrom: 2 };
  assert.equal(connPayload(null, reuse, 'openai.example.com').reuseKeysFrom, 2);
  assert.equal(connPayload(null, reuse, 'openai.example.com').confirmHost, 'openai.example.com');
  assert.ok(!('reuseKeysFrom' in connPayload(c, { ...formOf(c), reuseFrom: 2 })));
  assert.ok(!('confirmHost' in connPayload(c, formOf(c))));
});

test('the header clash, against the typed and the saved provider key', () => {
  const g = conn({ provider: 'gateway', baseUrl: 'https://ai-gateway.example.com/openai', keys: { apiKey: KEY(), gatewayKey: KEY() } });
  const f = formOf(g);
  assert.equal(headerClash(f, keyAfter(g, f, 'apiKey')), true, 'bearer + OpenAI format + a saved provider key');
  assert.equal(headerClash({ ...f, gatewayAuth: 'cf-aig-authorization' }, true), false);
  assert.equal(headerClash({ ...f, gatewayAuth: 'x-api-key', format: 'anthropic' }, true), true);
  assert.equal(headerClash({ ...f, format: 'responses' }, true), true, 'bearer + Responses clashes like OpenAI');
  assert.equal(headerClash({ ...f, format: 'responses', gatewayAuth: 'x-api-key' }, true), false);
  assert.equal(headerClash({ ...f, format: 'responses', gatewayAuth: 'cf-aig-authorization' }, true), false);
  const removed = { ...f, remove: { apiKey: true, gatewayKey: false } };
  assert.equal(headerClash(removed, keyAfter(g, removed, 'apiKey')), false, 'removing the saved provider key resolves it');
  const noKey = conn({ provider: 'gateway', baseUrl: 'https://ai-gateway.example.com/openai', keys: { apiKey: NONE, gatewayKey: KEY() } });
  const typedKey = { ...formOf(noKey), keys: { apiKey: 'sk-demo-0000-3333', gatewayKey: '' } };
  assert.equal(headerClash(typedKey, keyAfter(noKey, typedKey, 'apiKey')), true, 'a typed provider key counts too');
  assert.equal(keyAfter(null, blankForm(DEFAULTS), 'apiKey'), false);
  assert.equal(headerClash({ ...formOf(conn()), provider: 'openai' }, true), false, 'only gateways');
});

test('the result line of a save or a test: found, found without a list, rotated, failed, rotation failed', () => {
  assert.deepEqual(connResult({ ok: true, listed: true, modelCount: 42 }, false), { ok: true, key: 'ai.set.conn.ok', n: 42 });
  assert.deepEqual(connResult({ ok: true, listed: false }, false), { ok: true, key: 'ai.set.conn.okNoList', n: 0 });
  assert.equal(connResult({ ok: true, listed: true, modelCount: 3 }, true).key, 'ai.set.conn.rotated');
  assert.deepEqual(connResult({ ok: false, code: 'ai.up.auth' }, false), { ok: false, key: 'ai.set.conn.fail', code: 'ai.up.auth' });
  assert.deepEqual(connResult({ ok: false, code: 'ai.up.timeout' }, true), { ok: false, key: 'ai.set.conn.rotateFail', code: 'ai.up.timeout' });
  assert.equal((connResult({ ok: false }, false) as { code: string }).code, 'ai.internal');
});

test('the key state a connection card shows, per key slot the provider uses', () => {
  assert.deepEqual(keyLine(conn()), [{ slot: 'apiKey', state: 'saved', hint: '…ab12' }]);
  assert.deepEqual(keyLine(conn({ keys: { apiKey: NONE, gatewayKey: NONE } })), [{ slot: 'apiKey', state: 'none', hint: '' }]);
  assert.equal(keyLine(conn({ keys: { apiKey: { set: true, hint: '', unreadable: true }, gatewayKey: NONE } }))[0]?.state, 'unreadable');
  assert.deepEqual(keyLine(conn({ provider: 'gateway', keys: { apiKey: NONE, gatewayKey: KEY('…cd34') } })).map((k) => k.slot), ['gatewayKey', 'apiKey']);
});

test('the four status cards: AI+ switch, connections, default model, features', () => {
  const on = statusCards(v13());
  assert.deepEqual(on.map((c) => [c.id, c.tone, c.key]), [
    ['main', 'ok', 'ai.set.card.main.on'], ['conn', 'ok', 'ai.set.card.conn.allOk'],
    ['model', 'ok', 'ai.set.card.model.value'], ['features', 'ok', 'ai.set.card.features.allOn'],
  ]);
  assert.deepEqual(on[1]?.params, { n: 1 });
  assert.deepEqual(on[2]?.params, { conn: 'main', model: 'demo-chat-small' });
  assert.equal(statusCards(v13({ state: 'setup', enabled: false, connections: [], pool: [] }))[0]?.key, 'ai.set.card.main.setup');
  assert.equal(statusCards(v13({ state: 'off', enabled: false }))[0]?.key, 'ai.set.card.main.off');
  const noKey = statusCards(v13({ state: 'paused', pauseReason: 'keyProblem' }))[0];
  assert.deepEqual([noKey?.tone, noKey?.key], ['error', 'ai.set.card.main.pausedKey']);
  const noRoute = statusCards(v13({ state: 'paused', pauseReason: 'noRoute' }))[0];
  assert.deepEqual([noRoute?.tone, noRoute?.key], ['warn', 'ai.set.card.main.pausedNoRoute']);
  assert.equal(statusCards(v13({ connections: [] }))[1]?.key, 'ai.set.card.conn.none');
  const bad = statusCards(v13({ connections: [conn(), conn({ id: 2, name: 'second', status: 'broken', errorCode: 'ai.up.auth' })] }))[1];
  assert.deepEqual([bad?.tone, bad?.key, bad?.params], ['warn', 'ai.set.card.conn.someBad', { n: 2, bad: 1 }]);
  const unreadable = statusCards(v13({ connections: [conn({ keys: { apiKey: { set: true, hint: '', unreadable: true }, gatewayKey: NONE } })] }))[1];
  assert.equal(unreadable?.key, 'ai.set.card.conn.someBad', 'a key that cannot be read counts as a problem');
  const off = statusCards(v13({ featureSwitches: { chat: true, hint: true, explain: true, summary: true, generate: false, import: false } }))[3];
  assert.deepEqual([off?.tone, off?.key, off?.offFeatures], ['warn', 'ai.set.card.features.someOff', ['generate', 'import']]);
});

test('the default model that cannot be used is a warning only: nothing is switched to another model', () => {
  const s = v13({
    pool: [pool(4, 1, 'demo-chat-small'), pool(5, 2, 'demo-chat-large')], connections: [conn({ status: 'broken' }), conn({ id: 2, name: 'second' })],
    slots: { default: slot(4, null), chat: slot(null, null), study: slot(null, null), summary: slot(null, null), generate: slot(null, null), import: slot(null, null) },
  });
  assert.equal(defaultUnusable(s), true);
  const m = statusCards(s)[2];
  assert.deepEqual([m?.tone, m?.key, m?.params], ['warn', 'ai.set.card.model.bad', { conn: 'main', model: 'demo-chat-small' }]);
  assert.equal(slotModel(s, 'default')?.model, 'demo-chat-small');
  assert.equal(slotUsing(s, 'default'), null, 'no route: the page never names another model as being used');
  for (const k of SLOTS) assert.deepEqual(slotNote(s, k), { kind: 'noRoute' }, k);
  assert.equal(defaultUnusable(v13()), false);
});

test('slot notes: a row on the default stays quiet; an unusable own model says the default stands in', () => {
  const s = v13({
    pool: [pool(4, 1, 'demo-chat-small'), pool(5, 2, 'demo-chat-large', { status: 'failed' })],
    connections: [conn(), conn({ id: 2, name: 'second', status: 'broken' })],
    slots: { default: slot(4, 4), chat: slot(5, 4, 'default'), study: slot(null, 4, 'default'), summary: slot(null, 4, 'default'), generate: slot(null, 4, 'default'), import: slot(null, 4, 'default') },
  });
  assert.deepEqual(slotNote(s, 'chat'), { kind: 'fallback', conn: 'second' });
  assert.equal(slotNote(s, 'study'), null, 'following the default is not a fallback');
  assert.equal(slotNote(s, 'default'), null);
  assert.equal(slotModel(s, 'chat')?.id, 5);
  assert.equal(slotUsing(s, 'chat')?.id, 4);
  // nothing set up yet: no warnings at all
  const empty = v13({ state: 'setup', enabled: false, connections: [], pool: [], slots: { default: slot(null, null), chat: slot(null, null), study: slot(null, null), summary: slot(null, null), generate: slot(null, null), import: slot(null, null) } });
  for (const k of SLOTS) assert.equal(slotNote(empty, k), null, k);
  assert.equal(defaultUnusable(empty), false);
});

test('the merged model list: one group per connection, listed models (chat first) plus pool models the list does not show', () => {
  const s = v13({
    connections: [conn(), conn({ id: 2, name: 'second' })],
    pool: [pool(4, 1, 'demo-chat-small'), pool(5, 2, 'typed-by-hand', { manual: true })],
  });
  const list: AiModelList = {
    connections: [
      { id: 1, name: 'main', provider: 'openai', status: 'ok', listed: true, error: null, models: [
        { id: 'demo-embed-text-embedding', context: null, poolId: null, status: null },
        { id: 'demo-chat-small', context: 128000, poolId: 4, status: 'ok' },
        { id: 'demo-chat-large', context: 200000, poolId: null, status: null },
      ] },
      { id: 2, name: 'second', provider: 'gateway', status: 'ok', listed: false, error: null, models: [] },
    ],
  };
  const g = modelGroups(list, s, '');
  assert.deepEqual(g.map((x) => x.name), ['main', 'second']);
  assert.deepEqual(g[0]?.models.map((m) => m.id), ['demo-chat-small', 'demo-chat-large', 'demo-embed-text-embedding'], 'likely chat models first, in list order');
  assert.deepEqual(g[0]?.models.map((m) => m.inPool), [true, false, false]);
  assert.deepEqual(g[1]?.models.map((m) => [m.id, m.inPool]), [['typed-by-hand', true]], 'a service without a list still shows its pool models');
  // a query keeps only the groups with a match
  assert.deepEqual(modelGroups(list, s, 'LARGE').map((x) => [x.name, x.models.map((m) => m.id)]), [['main', ['demo-chat-large']]]);
  assert.deepEqual(modelGroups(list, s, 'zzz'), []);
  // no list read yet: the pool is still offered
  assert.deepEqual(modelGroups(null, s, '').map((x) => x.models.length), [1, 1]);
  // a failed pool row is not ticked
  const failed = v13({ pool: [pool(4, 1, 'demo-chat-small', { status: 'failed' })] });
  assert.equal(modelGroups(null, failed, '')[0]?.models[0]?.inPool, false);
});

test('the model preselected for the first switch-on, the new-default choices and the key donors', () => {
  const s = v13({ connections: [conn({ status: 'broken' }), conn({ id: 2, name: 'second' })], pool: [pool(4, 1, 'demo-chat-small'), pool(5, 2, 'demo-chat-large'), pool(6, 2, 'demo-other', { status: 'untested' })] });
  const list: AiModelList = { connections: [
    { id: 2, name: 'second', provider: 'openai', status: 'ok', listed: true, error: null, models: [
      { id: 'demo-rerank-v1', context: null, poolId: null, status: null }, { id: 'demo-chat-large', context: 1, poolId: 5, status: 'ok' }] },
  ] };
  assert.deepEqual(firstChoice(list, s), { connectionId: 2, model: 'demo-chat-large' }, 'a broken connection is skipped; a likely chat model wins');
  assert.equal(firstChoice(null, v13({ connections: [], pool: [] })), null);
  assert.deepEqual(newDefaultChoices(s, 1).map((m) => m.id), [5], 'tested models of other working connections');
  assert.deepEqual(newDefaultChoices(s, 2).map((m) => m.id), [], 'the only working connection is the one being deleted');
  assert.deepEqual(reusableFrom(s).map((c) => c.id), [1, 2]);
  assert.deepEqual(reusableFrom(v13({ connections: [conn({ keys: { apiKey: NONE, gatewayKey: NONE } })] })), []);
  assert.deepEqual(parseRefKey(refKey({ connectionId: 2, model: 'a|b:c' })), { connectionId: 2, model: 'a|b:c' }, 'the model may contain the separator');
  assert.equal(parseRefKey('nonsense'), null);
  assert.equal(parseRefKey('0|x'), null);
  assert.deepEqual(refOf(pool(5, 2, 'demo-chat-large')), { connectionId: 2, model: 'demo-chat-large' });
  assert.equal(connLabel(null, '—'), '—');
  assert.equal(connLabel('  ', '—'), '—');
  assert.equal(connLabel('main', '—'), 'main');
});

test('an error code about the name belongs to the name field', () => {
  assert.equal(fieldOfError({ code: 'ai.badName' }), 'name');
  assert.equal(fieldOfError({ code: 'ai.nameTaken' }), 'name');
  assert.equal(fieldOfError({ code: 'ai.badUrl' }), 'baseUrl');
  assert.equal(fieldOfError({ field: 'gatewayKey' }), 'gatewayKey');
});

test('retention days', () => {
  assert.equal(retentionOf('90'), 90);
  assert.equal(retentionOf(' 1 '), 1);
  assert.equal(retentionOf('3650'), 3650);
  for (const bad of ['0', '3651', '', '1.5', '-3', 'abc']) assert.equal(retentionOf(bad), null, bad);
  assert.equal(barPercent(0, 10), 0);
  assert.equal(barPercent(10, 10), 100);
  assert.equal(barPercent(1, 1000), 2, 'a sliver for anything above 0');
});

test('v1.3 limits: 33 keys in groups, the new ones, new defaults and ranges', () => {
  assert.equal(LIMIT_KEYS.length, 33);
  assert.equal(new Set(LIMIT_KEYS).size, 33);
  assert.deepEqual([...LIMIT_KEYS].sort(), Object.keys(DEFAULT_LIMITS).sort(), 'every key has a default');
  assert.deepEqual([...LIMIT_KEYS].sort(), Object.keys(LIMIT_RANGES).sort(), 'and a range');
  assert.deepEqual([...CAP_KEYS], ['callsPerMinute', 'callsPerHour', 'callsPerDay']);
  assert.deepEqual([...CALLS_KEYS], [...CAP_KEYS, 'callsAdminsExempt']);
  assert.deepEqual([...CHAT_KEYS], ['chatsPerUser', 'messagesPerChat', 'memoriesPerUser', 'memoryTokens']);
  assert.deepEqual([...LENGTH_KEYS], ['replyMaxTokens'], 'one answer length for chat, hint, explanation and summary');
  assert.deepEqual([...SHARE_KEYS], ['studyShare', 'studyShareDays']);
  assert.deepEqual([...SUMMARY_KEYS], ['summaryMaxItems']);
  assert.deepEqual([...SERVER_KEYS], ['perUserRunning', 'globalRunning', 'genRunning']);
  assert.deepEqual([...CONTENT_KEYS], ['genMinSource', 'genMinWrong', 'genBankSingle', 'genBankMulti', 'genWrongSingle', 'genWrongMulti']);
  assert.deepEqual([...BANK_KEYS], [...CONTENT_KEYS], 'the AI banks tab keeps the content rules only');
  assert.deepEqual([...LIMIT_KEYS], [...AS_KEYS, ...CONTENT_KEYS]);
  // the safety tab shows every key the AI banks tab does not
  for (const k of CONTENT_KEYS) assert.ok(!AS_KEYS.includes(k), k);
  assert.ok(!('hintMaxTokens' in DEFAULT_LIMITS) && !('explainMaxTokens' in DEFAULT_LIMITS) && !('summaryMaxTokens' in DEFAULT_LIMITS));
  assert.equal(DEFAULT_LIMITS.callsAdminsExempt, 1, 'administrators are exempt from the request caps by default');
  assert.equal(DEFAULT_LIMITS.memoryTokens, 200);
  assert.equal(DEFAULT_LIMITS.memoriesPerUser, 30);
  assert.equal(DEFAULT_LIMITS.importMaxQuestions, 100);
  assert.deepEqual(LIMIT_RANGES.importMaxQuestions, [1, 300]);
  assert.deepEqual([DEFAULT_LIMITS.studyShare, DEFAULT_LIMITS.studyShareDays, DEFAULT_LIMITS.genMaxChunks, DEFAULT_LIMITS.summaryMaxItems, DEFAULT_LIMITS.dailyTokenBudget], [1, 180, 8, 40, 0]);
  assert.deepEqual(LIMIT_RANGES.studyShareDays, [7, 3650]);
  assert.deepEqual(LIMIT_RANGES.genMaxChunks, [1, 50]);
  assert.deepEqual(LIMIT_RANGES.summaryMaxItems, [10, 70]);
  assert.deepEqual(LIMIT_RANGES.dailyTokenBudget, [0, 100000000]);
  assert.deepEqual(LIMIT_RANGES.callsAdminsExempt, [0, 1]);
});

test('the limits form: ranges, order of the caps, what a save sends', () => {
  const form = limitsFormOf(DEFAULT_LIMITS);
  assert.equal(form.replyMaxTokens, '3000');
  assert.equal(form.memoryTokens, '200');
  assert.equal(limitIssue(form, LIMIT_RANGES), null);
  assert.equal(limitProblem({ ...form, chatsPerUser: '9' }, LIMIT_RANGES), 'chatsPerUser');
  assert.equal(limitProblem({ ...form, globalRunning: '65' }, LIMIT_RANGES), 'globalRunning');
  assert.equal(limitProblem({ ...form, memoryTokens: '2.5' }, LIMIT_RANGES), 'memoryTokens');
  assert.equal(limitProblem({ ...form, replyMaxTokens: '0' }, LIMIT_RANGES), 'replyMaxTokens');
  assert.equal(limitProblem({ ...form, replyMaxTokens: '250000' }, LIMIT_RANGES), null);
  // the new keys
  assert.equal(limitProblem({ ...form, studyShareDays: '6' }, LIMIT_RANGES), 'studyShareDays');
  assert.equal(limitProblem({ ...form, studyShareDays: '7' }, LIMIT_RANGES), null);
  assert.equal(limitProblem({ ...form, genMaxChunks: '0' }, LIMIT_RANGES), 'genMaxChunks');
  assert.equal(limitProblem({ ...form, genMaxChunks: '51' }, LIMIT_RANGES), 'genMaxChunks');
  assert.equal(limitProblem({ ...form, summaryMaxItems: '9' }, LIMIT_RANGES), 'summaryMaxItems');
  assert.equal(limitProblem({ ...form, summaryMaxItems: '71' }, LIMIT_RANGES), 'summaryMaxItems');
  assert.equal(limitProblem({ ...form, dailyTokenBudget: '0' }, LIMIT_RANGES), null, '0 = no budget');
  assert.equal(limitProblem({ ...form, dailyTokenBudget: '100000001' }, LIMIT_RANGES), 'dailyTokenBudget');
  assert.equal(limitProblem({ ...form, callsAdminsExempt: '2' }, LIMIT_RANGES), 'callsAdminsExempt');
  assert.equal(limitProblem({ ...form, importMaxQuestions: '300' }, LIMIT_RANGES), null);
  assert.equal(limitProblem({ ...form, importMaxQuestions: '301' }, LIMIT_RANGES), 'importMaxQuestions');
  // the three caps: ranges first, then the order (equal is fine)
  assert.deepEqual(limitIssue({ ...form, callsPerHour: '2001' }, LIMIT_RANGES), { key: 'callsPerHour', kind: 'range' });
  assert.deepEqual(limitIssue({ ...form, callsPerMinute: '30' }, LIMIT_RANGES), { key: 'callsPerHour', kind: 'order' });
  assert.deepEqual(limitIssue({ ...form, callsPerHour: '200', callsPerDay: '150' }, LIMIT_RANGES), { key: 'callsPerDay', kind: 'order' });
  assert.equal(limitIssue({ ...form, callsPerMinute: '20', callsPerHour: '20', callsPerDay: '20' }, LIMIT_RANGES), null);
  assert.deepEqual(limitIssue({ ...form, callsPerMinute: '30', chatsPerUser: '9' }, LIMIT_RANGES), { key: 'chatsPerUser', kind: 'range' });
  // a save sends only what changed
  assert.deepEqual(limitsUpdate(DEFAULT_LIMITS, form), {});
  assert.deepEqual(limitsUpdate(DEFAULT_LIMITS, { ...form, callsAdminsExempt: '0', studyShareDays: '30', dailyTokenBudget: '500000' }), { callsAdminsExempt: 0, studyShareDays: 30, dailyTokenBudget: 500000 });
  assert.deepEqual(limitsUpdate({ ...DEFAULT_LIMITS, importMaxQuestions: 50 }, form), { importMaxQuestions: 100 });
  // a key the service did not send falls back to its default
  assert.equal(limitsFormOf({} as never).dailyTokenBudget, '0');
  assert.equal(limitsFormOf({} as never).callsAdminsExempt, '1');
  assert.ok(!('messageChars' in DEFAULT_LIMITS));
});

test('the host of an address, and the old tab id stays', () => {
  assert.equal(hostOf('https://api.example.com/v1'), 'api.example.com');
  assert.equal(hostOf('http://gw.example.test:8080/x/'), 'gw.example.test:8080');
  assert.equal(hostOf('nonsense/'), 'nonsense');
  assert.equal(tabOfHash('#/ai-settings'), 'feature');
  assert.equal(hashOfTab('feature'), '#/ai-settings?tab=feature');
});

test('jobTime: the full local date and time YYYY-MM-DD HH:mm, also for today', () => {
  const d = new Date(2026, 10, 5, 7, 4, 59);
  assert.equal(jobTime(Math.floor(d.getTime() / 1000)), '2026-11-05 07:04');
  const now = new Date();
  assert.match(jobTime(Math.floor(now.getTime() / 1000)), /^\d{4}-\d{2}-\d{2} \d{2}:\d{2}$/);
});
