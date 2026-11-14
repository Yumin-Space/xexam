// AI file import (IM-1 .. IM-9): the view order, the file checks, names, sizes, the card model of every IM-6 row, the confirm text.
import assert from 'node:assert/strict';
import test from 'node:test';
import type { GenJob, GenStatus } from '../types/api.ts';
import {
  ACCEPT, acceptFile, approxOf, countLabels, foundOf, awaitsConfirm, cardLines, confirmText, cutOf, extOf, fileStem, fileTypeOf, fmtSize, IMPORT_EXTS, importName, importView,
} from './ai-import.ts';

const gen = (o: Partial<GenStatus> = {}): GenStatus => ({
  perDay: 3, used: 1, left: 2, unlimited: false, resetAt: 1, minSource: 30, minWrong: 10, bank: { single: 14, multi: 6 },
  wrong: { single: 14, multi: 6 }, job: null, unseen: [], ...o,
});
const job = (o: Partial<GenJob> = {}): GenJob => ({
  id: 9, state: 'running', phase: 'reading', source: 'file', name: 'Kitchen notes_alexandr_20261112093000', from: [], fromCount: 0,
  file: { name: 'Kitchen notes.pdf', type: 'pdf', size: 2411520 }, mode: null, read: null, confirmBy: null, errorArg: null, skipped: null,
  made: 0, goal: 0, calls: 1, createdAt: 1, startedAt: null, finishedAt: null, bankId: null, error: null, charged: false, ...o,
});
const ok = { view: 'ok' } as const;

test('importView: hidden / service reasons (tag) / feature off (tag) / busy and quota (no tag) / own import', () => {
  assert.deepEqual(importView({ view: 'hidden' }, gen(), false), { show: false });
  assert.deepEqual(importView(ok, null, false), { show: false });
  assert.deepEqual(importView(ok, gen(), false), { show: true, usable: true });
  const paused = importView({ view: 'paused' }, gen(), false);
  assert.ok(paused.show && !paused.usable && paused.reason === 'paused' && paused.tag);
  const off = importView({ view: 'featureOff' }, gen(), false);
  assert.ok(off.show && !off.usable && off.reason === 'featureOff' && off.tag && off.key === 'ai.err.featureOff');
  const down = importView({ view: 'unavailable', reason: 'down' }, gen(), false);
  assert.ok(down.show && !down.usable && down.reason === 'down' && down.tag);
  const loading = importView({ view: 'unavailable', reason: 'loading' }, gen(), false);
  assert.ok(loading.show && !loading.usable && !loading.tag);
  const busy = importView(ok, gen({ job: job({ source: 'bank', phase: 'writing' }) }), false);
  assert.ok(busy.show && !busy.usable && busy.reason === 'busy' && !busy.tag && busy.key === 'aigen.why.busy');
  const own = importView(ok, gen({ job: job() }), false);
  assert.ok(own.show && !own.usable && own.reason === 'ownImport' && own.key === 'aigen.imp.busyShort');
  const up = importView(ok, gen(), true);
  assert.ok(up.show && !up.usable && up.reason === 'ownImport');
  const quota = importView(ok, gen({ left: 0 }), false);
  assert.ok(quota.show && !quota.usable && quota.reason === 'quota' && !quota.tag && quota.params.limit === 3);
  assert.ok(importView(ok, gen({ left: null, perDay: null, unlimited: true }), false).show);
  // the paused reason wins over a running import
  assert.equal((importView({ view: 'paused' }, gen({ job: job() }), false) as { reason: string }).reason, 'paused');
});

test('acceptFile: legacy, image, type, empty, too large in that order', () => {
  assert.deepEqual(acceptFile('old.DOC', 10, 20), { ok: false, error: 'legacy', ext: 'doc' });
  assert.deepEqual(acceptFile('scan.png', 0, 20), { ok: false, error: 'image', ext: 'png' });
  assert.deepEqual(acceptFile('notes.zip', 10, 20), { ok: false, error: 'type', ext: 'zip' });
  assert.deepEqual(acceptFile('noext', 10, 20), { ok: false, error: 'type', ext: '' });
  assert.deepEqual(acceptFile('a.pdf', 0, 20), { ok: false, error: 'empty', ext: 'pdf' });
  assert.deepEqual(acceptFile('a.pdf', 20 * 1_048_576 + 1, 20), { ok: false, error: 'tooLarge', ext: 'pdf' });
  assert.deepEqual(acceptFile('a.pdf', 20 * 1_048_576, 20), { ok: true, type: 'pdf' });
  assert.deepEqual(acceptFile('a.pdf', 5 * 1_048_576 + 1, 5), { ok: false, error: 'tooLarge', ext: 'pdf' });
  // a server setting above the hard ceiling never raises it
  assert.deepEqual(acceptFile('a.pdf', 21 * 1_048_576, 99), { ok: false, error: 'tooLarge', ext: 'pdf' });
  assert.deepEqual(acceptFile('Notes.Markdown', 10, 20), { ok: true, type: 'md' });
  for (const e of ['pdf', 'docx', 'xlsx', 'pptx', 'md', 'txt', 'csv']) assert.deepEqual(acceptFile(`f.${e}`, 1, 20), { ok: true, type: e });
});

test('extOf, fileTypeOf, ACCEPT, fmtSize', () => {
  assert.equal(extOf('A.b.PDF'), 'pdf');
  assert.equal(extOf('plain'), '');
  assert.equal(extOf('.md'), 'md');
  assert.equal(fileTypeOf('markdown'), 'md');
  assert.equal(fileTypeOf('doc'), null);
  for (const e of IMPORT_EXTS) assert.ok(ACCEPT.includes(`.${e}`));
  assert.ok(ACCEPT.includes('application/pdf') && ACCEPT.includes('text/csv'));
  assert.equal(ACCEPT.split(',').length, 8 + 7);
  assert.equal(fmtSize(850 * 1024), '850 KB');
  assert.equal(fmtSize(850 * 1024 - 1), '850 KB');
  assert.equal(fmtSize(1), '1 KB');
  assert.equal(fmtSize(2_411_520), '2.3 MB');
  assert.equal(fmtSize(20 * 1_048_576), '20.0 MB');
});

test('fileStem and importName: SEC-11 vectors', () => {
  assert.equal(fileStem('Kitchen notes.pdf'), 'Kitchen notes');
  assert.equal(fileStem('a.b.docx'), 'a.b');
  assert.equal(fileStem('.pdf'), 'file');
  assert.equal(fileStem('noext'), 'noext');
  assert.equal(importName('Kitchen notes', 'alexandra_k', '20261112093000'), 'Kitchen notes_alexandr_20261112093000');
  assert.equal(importName('', 'bo', '20261112093000'), 'file_bo_20261112093000');
  // ../../x.pdf: the stem is "../../x" -> separators become "_", leading dots are trimmed
  assert.equal(importName(fileStem('../../x.pdf'), 'bo', '20261112093000'), '_.._x_bo_20261112093000');
  assert.ok(!importName('a/b\\c:d', 'bo', 'T').includes('/'));
  assert.equal(importName('a/b\\c:d', 'bo', 'T'), 'a_b_c_d_bo_T');
  // a U+202E name loses the override
  assert.equal(importName('rep‮ort', 'bo', 'T'), 'report_bo_T');
  assert.equal(importName('...hidden', 'bo', 'T'), 'hidden_bo_T');
  // a 200-character CJK name fits the 120-character bank-name limit with the suffix
  const long = importName('题'.repeat(200), 'alexandra_k', '20261112093000');
  assert.equal([...long].length, 120);
  assert.ok(long.endsWith('_alexandr_20261112093000'));
});

test('cardLines: every row of IM-6', () => {
  const at = (n: number): string => `t${n}`;
  const up = cardLines(null, { name: 'Kitchen notes.pdf', size: 100, type: 'pdf', loaded: 25, total: 100 });
  assert.deepEqual(up?.kind, 'uploading');
  assert.deepEqual(up?.bar, { mode: 'determinate', pct: 25 });
  assert.deepEqual(up?.lines, [{ key: 'aigen.imp.card.uploading', params: { pct: 25 } }]);
  assert.deepEqual(up?.actions, ['cancelUpload']);
  assert.equal(up?.file, 'Kitchen notes.pdf');

  const q = cardLines(job({ state: 'queued' }), null);
  assert.equal(q?.kind, 'queuedRead');
  assert.deepEqual(q?.bar, { mode: 'indeterminate' });
  assert.equal(q?.cancelBodyKey, 'aigen.imp.card.cancelBodyRead');

  const r1 = cardLines(job(), null);
  assert.equal(r1?.kind, 'reading');
  assert.deepEqual(r1?.lines.map((l) => l.key), ['aigen.imp.card.reading', 'aigen.card.leave']);
  assert.equal(cardLines(job({ read: { done: 1, total: 0 } }), null)?.kind, 'reading');

  const r2 = cardLines(job({ read: { done: 2, total: 4 } }), null);
  assert.equal(r2?.kind, 'readingStep');
  assert.deepEqual(r2?.bar, { mode: 'determinate', pct: 50 });
  assert.deepEqual(r2?.lines[0], { key: 'aigen.imp.card.readingStep', params: { done: 2, total: 4 } });

  const cq = cardLines(job({ phase: 'confirm', mode: 'questions', goal: 45, confirmBy: 7 }), null, at);
  assert.equal(cq?.kind, 'confirm');
  assert.deepEqual(cq?.lines, [{ key: 'aigen.imp.card.confirmQ', params: { n: 45 } }, { key: 'aigen.imp.card.confirmBy', params: { time: 't7' }, muted: true }]);
  assert.deepEqual(cq?.actions, ['import', 'decline']);
  assert.equal(cq?.bar.mode, 'none');
  assert.equal(cardLines(job({ phase: 'confirm', mode: 'material', goal: 20 }), null)?.lines[0]?.key, 'aigen.imp.card.confirmM');

  const qw = cardLines(job({ state: 'queued', phase: 'writing', mode: 'questions', goal: 45 }), null);
  assert.equal(qw?.kind, 'queuedWrite');
  assert.deepEqual(qw?.bar, { mode: 'determinate', pct: 0 });
  assert.equal(qw?.cancelBodyKey, 'aigen.imp.card.cancelBodyQueued');

  const wq = cardLines(job({ phase: 'writing', mode: 'questions', made: 10, goal: 40 }), null);
  assert.equal(wq?.kind, 'writing');
  assert.deepEqual(wq?.bar, { mode: 'determinate', pct: 25 });
  assert.equal(wq?.lines[0]?.key, 'aigen.imp.card.writingQ');
  assert.equal(wq?.cancelBodyKey, 'aigen.card.cancelBody');
  // A-OWNER-66: finished questions -> the cancel offers save / discard; none yet -> the plain confirm
  assert.equal(wq?.cancelKeep, 10);
  assert.equal(cardLines(job({ phase: 'writing', mode: 'questions', made: 0, goal: 40 }), null)?.cancelKeep, undefined);
  assert.equal(cardLines(job({ phase: 'writing', mode: 'material', made: 4, goal: 20 }), null)?.lines[0]?.key, 'aigen.card.writing');
  assert.equal(cardLines(job({ phase: 'writing', mode: 'material', made: 4, goal: 20 }), null)?.cancelKeep, undefined);

  const sv = cardLines(job({ phase: 'saving' }), null);
  assert.equal(sv?.kind, 'saving');
  assert.deepEqual(sv?.actions, ['cancelDisabled']);

  const done = cardLines(job({ state: 'done', phase: null, made: 45, goal: 45, skipped: [] }), null);
  assert.equal(done?.kind, 'done');
  assert.equal(done?.lines[0]?.key, 'aigen.imp.card.done');
  assert.deepEqual(done?.actions, ['practice', 'dismiss']);
  const part = cardLines(job({ state: 'done', phase: null, made: 48, goal: 50, skipped: [3, 17] }), null);
  assert.deepEqual(part?.lines[0], {
    key: 'aigen.imp.card.donePartial', params: { file: 'Kitchen notes.pdf', name: 'Kitchen notes_alexandr_20261112093000', made: 48, goal: 50, n: 2, list: '3,17' },
  });

  const failed = cardLines(job({ state: 'failed', phase: null, error: 'ai.import.noText' }), null);
  assert.deepEqual(failed?.lines.map((l) => l.key), ['aigen.imp.card.failed', 'aigen.card.notCounted']);
  assert.deepEqual(failed?.actions, ['dismiss']);
  const expired = cardLines(job({ state: 'cancelled', phase: null, error: 'ai.import.expired' }), null);
  assert.equal(expired?.kind, 'expired');
  assert.equal(cardLines(job({ state: 'cancelled', phase: null }), null), null);
  assert.equal(cardLines(job({ source: 'bank' }), null), null);
  assert.equal(cardLines(null, null), null);
});

test('confirmText: questions (n drives the _one form) and material with the composition', () => {
  const g = gen({ bank: { single: 14, multi: 6 } });
  const q = confirmText(job({ phase: 'confirm', mode: 'questions', goal: 1 }), g);
  assert.equal(q.bodyKey, 'aigen.imp.cfm.questions');
  assert.equal(q.n, 1);
  assert.deepEqual(q.params, { file: 'Kitchen notes.pdf', n: 1, name: 'Kitchen notes_alexandr_20261112093000' });
  assert.equal(q.left, 2);
  const m = confirmText(job({ phase: 'confirm', mode: 'material', goal: 20 }), gen({ unlimited: true, left: null, perDay: null }));
  assert.equal(m.bodyKey, 'aigen.imp.cfm.material');
  assert.deepEqual(m.params, { file: 'Kitchen notes.pdf', n: 20, single: 14, multi: 6, name: 'Kitchen notes_alexandr_20261112093000' });
  assert.equal(m.left, null);
});

test('awaitsConfirm: only a running file job in phase confirm', () => {
  assert.equal(awaitsConfirm(job({ phase: 'confirm' })), true);
  assert.equal(awaitsConfirm(job({ phase: 'writing' })), false);
  assert.equal(awaitsConfirm(job({ phase: 'confirm', source: 'bank' })), false);
  assert.equal(awaitsConfirm(null), false);
});

test('A-OWNER-47: a file over the limit takes the first questions (confirm, card, result)', () => {
  const c = confirmText(job({ phase: 'confirm', mode: 'questions', goal: 50, found: 135 }), gen());
  assert.equal(c.bodyKey, 'aigen.imp.cfm.questionsCut');
  assert.deepEqual(c.params, { file: 'Kitchen notes.pdf', n: 50, found: 135, name: 'Kitchen notes_alexandr_20261112093000' });
  const card = cardLines(job({ phase: 'confirm', mode: 'questions', goal: 50, found: 135 }), null);
  assert.deepEqual(card?.lines[0], { key: 'aigen.imp.card.confirmQCut', params: { n: 50, found: 135 } });
  const done = cardLines(job({ state: 'done', phase: null, mode: 'questions', made: 50, goal: 50, found: 135, skipped: [] }), null);
  assert.equal(done?.lines[0]?.key, 'aigen.imp.card.doneCut');
  assert.equal(done?.lines[0]?.params.found, 135);
  const part = cardLines(job({ state: 'done', phase: null, mode: 'questions', made: 48, goal: 50, found: 135, skipped: [3, 17] }), null);
  assert.equal(part?.lines[1]?.key, 'aigen.imp.card.skippedNote');
  assert.equal(cutOf(job({ mode: 'questions', goal: 50, found: 50 })), false);
  assert.equal(cutOf(job({ mode: 'material', goal: 20, found: 99 })), false);
  assert.equal(cutOf(job({ mode: 'questions', goal: 20 })), false);
});

test('A-OWNER-66: texts of the partial import and the keep dialog exist in zh / en / ja', async () => {
  const { hasKey } = await import('../i18n/index.ts');
  for (const lang of ['en', 'zh', 'ja'] as const) {
    for (const k of ['aigen.imp.card.donePartial', 'aigen.imp.card.skippedNote', 'aigen.imp.card.keepBody', 'aigen.imp.card.keepOk', 'aigen.imp.card.keepDiscard', 'ai.set.gen.skipped']) {
      assert.equal(hasKey(lang, k), true, `${lang} ${k}`);
    }
  }
});

test('A-OWNER-67: the live usage text exists in zh / en / ja and jobOf reads the sums (zeros when missing)', async () => {
  const { hasKey } = await import('../i18n/index.ts');
  for (const lang of ['en', 'zh', 'ja'] as const) assert.equal(hasKey(lang, 'aigen.card.usage'), true, lang);
  assert.equal(hasKey('en', 'aigen.card.usage_one'), true);
  const { jobOf } = await import('../state/ai.ts');
  const base = { id: 1, state: 'running', source: 'file' };
  assert.deepEqual(jobOf({ ...base, usage: { calls: 12, tokensIn: 18432, tokensOut: 41207 } })?.usage, { calls: 12, tokensIn: 18432, tokensOut: 41207 });
  assert.deepEqual(jobOf(base)?.usage, { calls: 0, tokensIn: 0, tokensOut: 0 });
});

test('A-OWNER-70 / 72: the service counts (total / taken) drive the cut text and the two labels', () => {
  const j = job({ phase: 'confirm', mode: 'questions', goal: 50, total: 69, taken: 50 });
  assert.equal(foundOf(j), 69);
  assert.equal(cutOf(j), true);
  assert.deepEqual(countLabels(j), { fileKey: 'aigen.imp.cnt.file', total: 69, limit: 50 });
  assert.deepEqual(cardLines(j, null)?.counts, { fileKey: 'aigen.imp.cnt.file', total: 69, limit: 50 });
  assert.deepEqual(cardLines(j, null)?.lines[0], { key: 'aigen.imp.card.confirmQCut', params: { n: 50, found: 69 } });
  assert.deepEqual(confirmText(j, gen()).counts, { fileKey: 'aigen.imp.cnt.file', total: 69, limit: 50 });
  const done = cardLines(job({ state: 'done', phase: null, mode: 'questions', made: 50, goal: 50, total: 69, taken: 50 }), null);
  assert.equal(done?.lines[0]?.key, 'aigen.imp.card.doneCut');
  assert.equal(done?.lines[0]?.params.found, 69);
});

test('A-OWNER-70 / 72: within the limit no limit label; uncertain counts say "about"; material and unknown have no labels', () => {
  assert.deepEqual(countLabels(job({ mode: 'questions', goal: 45, total: 45 })), { fileKey: 'aigen.imp.cnt.file', total: 45, limit: null });
  const ap = job({ phase: 'confirm', mode: 'questions', goal: 50, total: 69, approx: true });
  assert.equal(approxOf(ap), true);
  assert.equal(countLabels(ap)?.fileKey, 'aigen.imp.cnt.fileApprox');
  assert.equal(cardLines(ap, null)?.lines[0]?.key, 'aigen.imp.card.confirmQCutApprox');
  assert.equal(confirmText(ap, gen()).bodyKey, 'aigen.imp.cfm.questionsCutApprox');
  assert.equal(countLabels(job({ mode: 'material', goal: 20, total: 99 })), null);
  assert.equal(countLabels(job({ mode: 'questions', goal: 20 })), null);
});

test('A-OWNER-84: an import job cancels with import words, generation keeps its own; the token text keys exist in zh / en / ja', async () => {
  const { t, tn, setUiLang } = await import('../i18n/index.ts');
  const want = {
    zh: ['取消导入', '要取消导入吗？', '已取消导入。', '本轮调用 12 次', '缓存写入/读取:', '取消生成'],
    en: ['Cancel import', 'Cancel the import?', 'Import cancelled.', '12 calls this run', 'Cache write/read:', 'Stop generating'],
    ja: ['インポートを中止', 'インポートを中止しますか？', 'インポートを中止しました。', '今回の呼び出し 12 回', 'キャッシュ書き込み/読み取り:', '作成を中止'],
  } as const;
  for (const lang of ['zh', 'en', 'ja'] as const) {
    setUiLang(lang);
    const w = want[lang];
    assert.deepEqual([t('aigen.imp.card.cancel'), t('aigen.imp.card.cancelAsk'), t('aigen.imp.toast.cancelled'), tn('aigen.card.calls', 12, { calls: 12 }), t('aigen.card.cacheLabel'), t('aigen.card.cancel')], [...w], lang);
  }
  setUiLang('en');
  assert.equal(tn('aigen.card.calls', 1, { calls: 1 }), '1 call this run');
  setUiLang('zh');
});
