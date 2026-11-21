// AI banks tab, recent jobs: the detail block lists only the fields a job carries; the state sort order; the history request asks
// for the server maximum. fetch is replaced; no network. Run with `node --experimental-strip-types --test`.
import assert from 'node:assert/strict';
import test from 'node:test';
import { fetchAdminGen } from '../api/ai-gen.ts';
import { setUiLang, t, type MsgKey } from '../i18n/index.ts';
import type { GenAdminJob } from '../types/api.ts';
import { jobDetailRows, stateRank, stateText } from './gen-jobs.ts';

const job = (o: Partial<GenAdminJob> = {}): GenAdminJob => ({
  id: 1, state: 'done', phase: null, source: 'bank', name: 'Sample bank', from: [{ id: 4, name: 'Source A' }], fromCount: 1, file: null, mode: null,
  read: null, confirmBy: null, errorArg: null, skipped: null, made: 20, goal: 20, calls: 3, createdAt: 1_700_000_000, startedAt: 1_700_000_010,
  finishedAt: 1_700_000_090, bankId: 9, error: null, charged: true, usage: { calls: 3, tokensIn: 1200, tokensOut: 800, tokensCached: 500, tokensCacheWrite: 100 },
  userId: 2, username: 'sample', tokensIn: 1200, tokensOut: 800, ...o,
});

const keys = (j: GenAdminJob): MsgKey[] => jobDetailRows(j).map(([k]) => k);

test('a done generation job lists source, counts, tokens, cache, times and the bank made', () => {
  setUiLang('en');
  const ks = keys(job());
  for (const k of ['ai.set.gen.d.kind', 'ai.set.gen.d.from', 'ai.set.gen.d.made', 'ai.set.gen.d.calls', 'ai.set.gen.d.tin', 'ai.set.gen.d.tout',
    'ai.set.gen.d.cached', 'ai.set.gen.d.created', 'ai.set.gen.d.started', 'ai.set.gen.d.finished', 'ai.set.gen.d.bank'] as MsgKey[]) assert.ok(ks.includes(k), k);
  assert.ok(!ks.includes('ai.set.gen.d.file'));
  assert.ok(!ks.includes('ai.set.gen.d.reason'));
  const rows = new Map(jobDetailRows(job()));
  assert.equal(rows.get('ai.set.gen.d.kind'), t('ai.set.gen.d.kind.gen'));
  assert.equal(rows.get('ai.set.gen.d.bank'), 'Sample bank (#9)');
});

test('an import job shows file, mode and found / taken; a failed job shows the reason and no bank', () => {
  setUiLang('en');
  const j = job({ source: 'file', state: 'failed', error: 'ai.err.internal', file: { name: 'f.xlsx', type: 'xlsx', size: 2048 }, mode: 'questions', total: 30, taken: 20, approx: true, usage: undefined });
  const ks = keys(j);
  for (const k of ['ai.set.gen.d.file', 'ai.set.gen.d.mode', 'ai.set.gen.d.found', 'ai.set.gen.d.taken', 'ai.set.gen.d.reason'] as MsgKey[]) assert.ok(ks.includes(k), k);
  assert.ok(!ks.includes('ai.set.gen.d.bank'));
  assert.ok(!ks.includes('ai.set.gen.d.cached'));
  assert.equal(new Map(jobDetailRows(j)).get('ai.set.gen.d.kind'), t('ai.set.gen.d.kind.import'));
  assert.match(new Map(jobDetailRows(j)).get('ai.set.gen.d.file') ?? '', /f\.xlsx \(XLSX, 2 KB\)/);
});

test('a job that has not started shows no started / finished time; extra source banks are counted', () => {
  setUiLang('en');
  const j = job({ state: 'queued', startedAt: null, finishedAt: null, fromCount: 14, bankId: null });
  const ks = keys(j);
  assert.ok(!ks.includes('ai.set.gen.d.started') && !ks.includes('ai.set.gen.d.finished'));
  assert.match(new Map(jobDetailRows(j)).get('ai.set.gen.d.from') ?? '', /Source A · /);
});

test('every detail label exists in all three languages', () => {
  const all = [...keys(job()), ...keys(job({ source: 'file', state: 'failed', error: 'ai.err.internal', mode: 'material', total: 3, taken: 3, chunks: 2, prefixTokens: 10, skipped: [2] }))];
  for (const lang of ['zh', 'en', 'ja'] as const) {
    setUiLang(lang);
    for (const k of new Set(all)) assert.notEqual(t(k), k, `${lang} ${k}`);
    for (const k of ['ai.set.gen.d.show', 'ai.set.gen.d.hide', 'ai.set.gen.d.aria', 'ai.set.gen.d.kind.gen', 'ai.set.gen.d.kind.import', 'ai.set.gen.d.mode.questions', 'ai.set.gen.d.about'] as MsgKey[]) {
      assert.notEqual(t(k), k, `${lang} ${k}`);
    }
  }
  setUiLang('en');
});

test('running sorts first, then waiting, done, failed, cancelled', () => {
  const order = ['cancelled', 'done', 'running', 'failed', 'queued'].map((state) => ({ state })).sort((a, b) => stateRank(a) - stateRank(b)).map((x) => x.state);
  assert.deepEqual(order, ['running', 'queued', 'done', 'failed', 'cancelled']);
  setUiLang('en');
  assert.equal(stateText(job({ state: 'queued', waiting: true })), `${t('ai.set.gen.state.queued')} · ${t('ai.set.gen.phase.waiting')}`);
});

test('the history request asks for the server maximum (200) and clamps larger values', async () => {
  const urls: string[] = [];
  const real = globalThis.fetch;
  globalThis.fetch = (async (input: string | URL | Request): Promise<Response> => {
    urls.push(String(input));
    return new Response(JSON.stringify({ jobs: [] }), { status: 200, headers: { 'Content-Type': 'application/json; charset=utf-8' } });
  }) as typeof fetch;
  try {
    await fetchAdminGen();
    await fetchAdminGen(5000);
  } finally {
    globalThis.fetch = real;
  }
  assert.deepEqual(urls, ['/api/ai/admin/gen?limit=200', '/api/ai/admin/gen?limit=200']);
});
