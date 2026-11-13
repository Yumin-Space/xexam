// AI-generated banks: names (GP-9 vectors), the visibility / reason order of GB-1, the AI+题集 tip choice, card helpers.
import assert from 'node:assert/strict';
import test from 'node:test';
import type { BankAi, GenJob, GenStatus } from '../types/api.ts';
import {
  aiOfBank,
  activeJob, aiMetaLine, aiSetTip, bankModeName, cardJobs, checkName, finishedBetween, genCount, genView, isHiddenFormat, minuteText,
  pollInterval, runningLine, stampText, stripName,
} from './ai-gen.ts';

const gen = (o: Partial<GenStatus> = {}): GenStatus => ({
  perDay: 3, used: 1, left: 2, unlimited: false, resetAt: 1, minSource: 30, minWrong: 10, bank: { single: 14, multi: 6 },
  wrong: { single: 14, multi: 6 }, job: null, unseen: [], ...o,
});
const job = (o: Partial<GenJob> = {}): GenJob => ({
  id: 1, state: 'running', phase: 'writing', source: 'bank', name: 'n', from: [], fromCount: 0, file: null, mode: null, read: null,
  confirmBy: null, errorArg: null, skipped: null, made: 8, goal: 20, calls: 1, createdAt: 1, startedAt: 1, finishedAt: null, bankId: null,
  error: null, charged: true, ...o,
});

test('checkName: GP-9 vectors', () => {
  assert.deepEqual(checkName('  AI practice 11-12 09:30 '), { ok: true, name: 'AI practice 11-12 09:30' });
  assert.deepEqual(checkName('   '), { ok: false, error: 'empty' });
  assert.deepEqual(checkName('a‮b'), { ok: false, error: 'bad' });
  assert.deepEqual(checkName('a​b'), { ok: false, error: 'bad' });
  assert.deepEqual(checkName('a\nb'), { ok: false, error: 'bad' });
  // U+200D inside an emoji sequence is fine
  assert.deepEqual(checkName('👩‍🍳 chef'), { ok: true, name: '👩‍🍳 chef' });
  // 60 code points (120 UTF-16 units) pass, 61 CJK characters do not
  assert.equal(checkName('😀'.repeat(60)).ok, true);
  assert.deepEqual(checkName('题'.repeat(61)), { ok: false, error: 'long' });
  assert.equal(checkName('题'.repeat(60)).ok, true);
  // a BOM is never trimmed away (the server refuses it)
  assert.deepEqual(checkName('﻿name'), { ok: false, error: 'bad' });
  assert.equal(isHiddenFormat('‍'), false);
  assert.equal(isHiddenFormat('⁦'), true);
});

test('bankModeName: the example, stripping, and the cut that keeps the suffix', () => {
  assert.equal(bankModeName('Kitchen basics', 'alexandra_k', '20261112093000'), 'Kitchen basics_alexandr_20261112093000');
  assert.equal(bankModeName('  Kit‮chen​ \n', 'al​ex', '20261112093000'), 'Kitchen_alex_20261112093000');
  assert.equal(bankModeName('Set', 'bo', '20261112093000'), 'Set_bo_20261112093000');
  const long = bankModeName('题'.repeat(200), 'alexandra_k', '20261112093000');
  assert.equal([...long].length, 120);
  assert.ok(long.endsWith('_alexandr_20261112093000'));
  assert.equal([...bankModeName('x'.repeat(96), 'alexandra_k', '20261112093000')].length, 120);
  assert.equal(stripName('a\u0007b⁠c'), 'abc');
});

test('stampText and minuteText', () => {
  assert.equal(stampText(new Date(2026, 10, 12, 9, 5)), '11-12 09:05');
  assert.equal(minuteText('2026-11-12 09:30:00'), '2026-11-12 09:30');
  assert.equal(minuteText('weird'), 'weird');
});

test('genView: hidden / service reasons carry the tag and win over everything', () => {
  const ctx = { mode: 'bank' as const, have: 100 };
  assert.deepEqual(genView({ view: 'hidden' }, gen(), ctx), { show: false });
  assert.deepEqual(genView({ view: 'ok' }, null, ctx), { show: false });
  const paused = genView({ view: 'paused' }, gen({ left: 0 }), ctx);
  assert.ok(paused.show && !paused.usable && paused.reason === 'paused' && paused.tag && paused.key === 'ai.badge.tip.suspended');
  const off = genView({ view: 'unavailable', reason: 'off' }, gen(), ctx);
  assert.ok(off.show && !off.usable && off.reason === 'off' && off.tag && off.key === 'ai.badge.tip.off');
  const loading = genView({ view: 'unavailable', reason: 'loading' }, gen(), ctx);
  assert.ok(loading.show && !loading.usable && loading.reason === 'loading' && !loading.tag);
  const fo = genView({ view: 'featureOff' }, gen(), ctx);
  assert.ok(fo.show && !fo.usable && fo.reason === 'featureOff' && fo.tag && fo.key === 'ai.err.featureOff');
});

test('genView: the own reasons in their order, no tag', () => {
  const ok = { view: 'ok' } as const;
  const reason = (g: GenStatus, c: Parameters<typeof genView>[2]): string => {
    const v = genView(ok, g, c);
    return v.show && !v.usable ? `${v.reason}:${v.key}:${v.tag}` : 'usable';
  };
  // busy first, even with quota used up and nothing checked
  assert.equal(reason(gen({ job: job(), left: 0 }), { mode: 'scope', have: 0 }), 'busy:aigen.why.busy:false');
  assert.equal(reason(gen({ job: job({ state: 'done' }) }), { mode: 'bank', have: 99 }), 'usable');
  // quota
  assert.equal(reason(gen({ left: 0 }), { mode: 'scope', have: 0 }), 'quota:aigen.why.quota:false');
  assert.equal(reason(gen({ left: 0, perDay: 0 }), { mode: 'bank', have: 99 }), 'quota:aigen.why.quotaZero:false');
  assert.equal(reason(gen({ left: null, perDay: null, unlimited: true }), { mode: 'bank', have: 99 }), 'usable');
  // Home: nothing checked, then only AI banks, then the counts
  assert.equal(reason(gen(), { mode: 'scope', checkedSheets: 0, have: 0 }), 'noScope:aigen.why.noScope:false');
  assert.equal(reason(gen(), { mode: 'wrong', checkedSheets: 0, onlyAi: true, have: 0 }), 'aiSource:aigen.why.aiSource:false');
  assert.equal(reason(gen(), { mode: 'scope', checkedSheets: 2, have: 29 }), 'fewSource:aigen.why.fewSource:false');
  assert.equal(reason(gen(), { mode: 'scope', checkedSheets: 2, have: 30 }), 'usable');
  assert.equal(reason(gen(), { mode: 'wrong', checkedSheets: 2, have: 9 }), 'fewWrong:aigen.why.fewWrong:false');
  assert.equal(reason(gen(), { mode: 'wrong', checkedSheets: 2, have: 10 }), 'usable');
  // A-OWNER-52b: any AI bank in the ticked scope blocks generation, every mode
  assert.equal(reason(gen(), { mode: 'scope', checkedSheets: 3, hasAi: true, have: 99 }), 'aiSource:aigen.why.aiInScope:false');
  assert.equal(reason(gen(), { mode: 'wrong', checkedSheets: 3, hasAi: true, have: 99 }), 'aiSource:aigen.why.aiInScope:false');
  assert.equal(reason(gen(), { mode: 'scope', checkedSheets: 3, hasAi: false, have: 99 }), 'usable');
  // Bank management: an AI bank is never a source
  assert.equal(reason(gen(), { mode: 'bank', aiBank: true, have: 99 }), 'aiSource:aigen.why.aiSource:false');
  assert.equal(reason(gen(), { mode: 'bank', have: 12 }), 'fewSource:aigen.why.fewSource:false');
});

test('genView: params and the skipped-AI-banks note', () => {
  const v = genView({ view: 'ok' }, gen({ minSource: 40 }), { mode: 'scope', checkedSheets: 1, skippedAi: true, have: 12 });
  assert.ok(v.show && !v.usable && v.reason === 'fewSource');
  if (v.show && !v.usable) {
    assert.deepEqual(v.params, { need: 40, have: 12 });
    assert.equal(v.skippedNote, true);
  }
  const q = genView({ view: 'ok' }, gen({ perDay: 3, left: 0 }), { mode: 'bank', have: 99 });
  if (q.show && !q.usable) assert.deepEqual(q.params, { limit: 3 });
  const u = genView({ view: 'ok' }, gen(), { mode: 'bank', have: 31 });
  assert.deepEqual(u, { show: true, usable: true, need: 30, have: 31 });
  assert.equal(genCount(gen(), 'scope'), 20);
  assert.equal(genCount(gen({ wrong: { single: 3, multi: 2 } }), 'wrong'), 5);
});

test('aiSetTip and aiMetaLine: generated and imported banks', () => {
  const base: BankAi = { source: 'bank', from: ['Kitchen basics'], author: 'alexandra_k', made: 20, goal: 20, at: '2026-11-12 09:30:00' };
  assert.deepEqual(aiSetTip(base), { key: 'aigen.badge.tip', params: { name: 'alexandra_k', time: '2026-11-12 09:30' } });
  assert.equal(aiSetTip({ ...base, author: null }).key, 'aigen.badge.tipNoAuthor');
  const file: BankAi = { ...base, source: 'file', from: [], file: 'Kitchen notes.pdf', mode: 'questions' };
  assert.deepEqual(aiSetTip(file), { key: 'aigen.badge.tipFile', params: { name: 'alexandra_k', file: 'Kitchen notes.pdf', time: '2026-11-12 09:30' } });
  assert.equal(aiSetTip({ ...file, author: null }).key, 'aigen.badge.tipFileNoAuthor');
  assert.equal(aiSetTip({ ...file, mode: 'material' }).key, 'aigen.badge.tipFileMaterial');
  assert.equal(aiSetTip({ ...file, mode: 'material', author: null }).key, 'aigen.badge.tipFileMaterialNoAuthor');
  assert.deepEqual(aiMetaLine(base), { key: 'aigen.bank.metaBank', params: { from: 'Kitchen basics' } });
  assert.equal(aiMetaLine({ ...base, source: 'scope' }).key, 'aigen.bank.metaScope');
  assert.equal(aiMetaLine({ ...base, source: 'wrong' }).key, 'aigen.bank.metaWrong');
  assert.deepEqual(aiMetaLine(file), { key: 'aigen.bank.metaFile', params: { file: 'Kitchen notes.pdf' } });
  assert.equal(aiMetaLine({ ...file, mode: 'material' }).key, 'aigen.bank.metaFileMaterial');
});

test('cards: the active job first, unseen jobs newest first (max 3), cancelled only when expired', () => {
  const g = gen({
    job: job({ id: 9 }),
    unseen: [job({ id: 8, state: 'done' }), job({ id: 7, state: 'cancelled' }), job({ id: 6, state: 'cancelled', error: 'ai.import.expired' }), job({ id: 5, state: 'failed' }), job({ id: 4, state: 'done' })],
  });
  assert.deepEqual(cardJobs(g).map((j) => j.id), [9, 8, 6]);
  assert.deepEqual(cardJobs(null), []);
  assert.deepEqual(cardJobs(gen({ job: job({ id: 3, state: 'done' }), unseen: [] })), []);
  assert.equal(activeJob(gen({ job: job({ state: 'queued' }) }))?.id, 1);
  assert.equal(activeJob(gen({ job: job({ state: 'failed' }) })), null);
});

test('runningLine: queued / writing / saving and the percentage', () => {
  assert.deepEqual(runningLine(job({ state: 'queued', phase: null })), { key: 'aigen.card.queued', pct: 0 });
  assert.deepEqual(runningLine(job({ made: 5, goal: 20 })), { key: 'aigen.card.writing', pct: 25 });
  assert.deepEqual(runningLine(job({ phase: 'saving', made: 20 })), { key: 'aigen.card.saving', pct: 100 });
  assert.equal(runningLine(job({ goal: 0, made: 0 })).pct, 0);
});

test('pollInterval: 2 s on Home / Bank management while visible, 10 s elsewhere, never on the quiz', () => {
  assert.equal(pollInterval('home', true, job()), 2000);
  assert.equal(pollInterval('banks', true, job()), 2000);
  assert.equal(pollInterval('banks', false, job()), 10000);
  assert.equal(pollInterval('records', true, job()), 10000);
  assert.equal(pollInterval('quiz', true, job()), null);
  assert.equal(pollInterval('home', true, null), null);
  assert.equal(pollInterval('home', true, job({ state: 'done' })), null);
  assert.equal(pollInterval('home', true, job({ source: 'file', phase: 'confirm' })), 10000);
});

test('finishedBetween: active -> done / failed only', () => {
  const prev = job({ id: 4 });
  assert.equal(finishedBetween(prev, gen({ unseen: [job({ id: 4, state: 'done' })] }))?.state, 'done');
  assert.equal(finishedBetween(prev, gen({ unseen: [job({ id: 4, state: 'failed' })] }))?.state, 'failed');
  assert.equal(finishedBetween(prev, gen({ unseen: [job({ id: 4, state: 'cancelled' })] })), null);
  assert.equal(finishedBetween(prev, gen({ job: job({ id: 4 }) })), null);
  assert.equal(finishedBetween(null, gen()), null);
  assert.equal(finishedBetween(job({ id: 4, state: 'done' }), gen({ unseen: [job({ id: 4, state: 'done' })] })), null);
});

test('aiOfBank: the marker follows the question bank id', () => {
  const ai = { by: 'sam', at: 1, source: 'bank' } as unknown as BankAi;
  const banks = [{ id: 1 }, { id: 2, ai }];
  assert.equal(aiOfBank(banks, 2), ai);
  assert.equal(aiOfBank(banks, 1), null);
  assert.equal(aiOfBank(banks, 9), null);
});

test('badge word by origin (A-OWNER-53d)', async () => {
  const { aiSetWordKey } = await import('./ai-gen.ts');
  const ai = (source: 'bank' | 'scope' | 'wrong' | 'file') => ({ source, from: [], author: null, made: 1, goal: 1, at: '2026-01-01 00:00:00' });
  assert.equal(aiSetWordKey(ai('bank')), 'aigen.badge.wordPractice');
  assert.equal(aiSetWordKey(ai('scope')), 'aigen.badge.wordPractice');
  assert.equal(aiSetWordKey(ai('wrong')), 'aigen.badge.wordWrong');
  assert.equal(aiSetWordKey(ai('file')), 'aigen.badge.wordImport');
});
