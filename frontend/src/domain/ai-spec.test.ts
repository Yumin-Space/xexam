import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import { buildStudyAsk } from './ai-plus.ts';
import { studySavedQuery, studySpec, summarySpec, type StudyTransport } from './ai-spec.ts';

const calls: string[] = [];
const tr: StudyTransport = {
  savedStudy: async (q) => { calls.push('savedStudy ' + JSON.stringify(q)); return { chatId: null, latest: null, lastGood: null }; },
  askStudy: async (b) => { calls.push('askStudy ' + JSON.stringify(b)); },
  savedSummary: async (q) => { calls.push('savedSummary ' + JSON.stringify(q)); return { chatId: null, latest: null, lastGood: null }; },
  askSummary: async (b) => { calls.push('askSummary ' + JSON.stringify(b)); },
  stopChat: async (id) => { calls.push('stop ' + id); },
};
const ac = new AbortController();

describe('studySavedQuery (answer A7: exactly one of key / record)', () => {
  const base = { mode: 'exam', source: 'result', kind: 'explain', questionId: 'q1' } as const;
  it('the practice page and the result pages ask by key', () => {
    assert.deepEqual(studySavedQuery({ ...base, page: 'result', key: 'a'.repeat(32), recordId: 4 }), { mode: 'exam', source: 'result', question: 'q1', kind: 'explain', key: 'a'.repeat(32) });
    assert.deepEqual(studySavedQuery({ ...base, page: 'practice', mode: 'practice', source: 'practice', kind: 'hint', key: 'b'.repeat(32), recordId: null }), { mode: 'practice', source: 'practice', question: 'q1', kind: 'hint', key: 'b'.repeat(32) });
  });
  it('the record detail page always asks by record, even when the record has a key', () => {
    const q = studySavedQuery({ ...base, page: 'detail', key: 'a'.repeat(32), recordId: 4 });
    assert.deepEqual(q, { mode: 'exam', source: 'result', question: 'q1', kind: 'explain', record: 4 });
    assert.equal(q !== null && 'key' in q, false);
  });
  it('nothing to look up without the needed id', () => {
    assert.equal(studySavedQuery({ ...base, page: 'result', key: null, recordId: 4 }), null);
    assert.equal(studySavedQuery({ ...base, page: 'detail', key: null, recordId: null }), null);
  });
});

describe('spec builders', () => {
  it('a study spec reads the card language when the ask starts', async () => {
    let qlang: 'en' | 'ja' = 'en';
    const body = () => buildStudyAsk({ kind: 'hint', mode: 'practice', key: 'a'.repeat(32), startedAt: 10, questionId: 'q1', no: 2, qlang, ui: 'en', source: 'practice' });
    const s = studySpec(tr, { mode: 'practice', key: 'a'.repeat(32), source: 'practice', question: 'q1', kind: 'hint' }, body);
    await s.loadSaved();
    qlang = 'ja';
    await s.ask(ac.signal, () => {});
    await s.stopChat(5);
    assert.match(calls.join('\n'), /savedStudy \{"mode":"practice","key":"a{32}","source":"practice","question":"q1","kind":"hint"\}/);
    assert.match(calls.join('\n'), /askStudy .*"qlang":"ja"/);
    assert.ok(calls.includes('stop 5'));
  });
  it('a study spec without a saved query has nothing to look up', async () => {
    const s = studySpec(tr, null, () => buildStudyAsk({ kind: 'explain', mode: 'practice', key: null, startedAt: null, questionId: 'q', no: 1, qlang: 'en', ui: 'en', source: 'result', recordId: 3 }));
    assert.equal(await s.loadSaved(), null);
  });
  it('a summary spec sends at as 12 digits and the Settings language', async () => {
    calls.length = 0;
    const s = summarySpec(tr, { id: 42, at: '2026-11-11 21:58' }, () => 'zh');
    await s.loadSaved();
    await s.ask(ac.signal, () => {});
    assert.deepEqual(calls, ['savedSummary {"record":42,"at":"202611112158"}', 'askSummary {"recordId":42,"lang":"zh-CN"}']);
  });
  it('a summary spec with an unreadable time has no lookup', async () => {
    const s = summarySpec(tr, { id: 1, at: 'soon' }, () => 'en');
    assert.equal(await s.loadSaved(), null);
  });
});
