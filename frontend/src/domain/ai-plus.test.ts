import assert from 'node:assert/strict';
import { describe, it } from 'node:test';
import type { AiStatus } from '../state/ai.ts';
import {
  aiControls, aiPlusView, atDigits, basedKey, buildStudyAsk, buildSummaryAsk, busyView, chatDisplayTitle, controlState,
  popTitleKey, recordSummaryExtra, replyLang, resultExtra, type RunningInfo,
} from './ai-plus.ts';

const ready = (enabled: boolean, suspended = false): AiStatus =>
  ({ state: 'ready', enabled, allowed: enabled && !suspended, suspended }) as unknown as AiStatus;
const LOADING: AiStatus = { state: 'loading' };
const DOWN: AiStatus = { state: 'unavailable' };

describe('aiPlusView', () => {
  it('role user without the service sees nothing, whatever the status', () => {
    for (const status of [LOADING, DOWN, ready(true), ready(false), ready(true, true)]) {
      assert.deepEqual(aiPlusView({ role: 'user', service: false, status }), { show: false });
    }
  });
  it('role user with the service: usable, suspended, unavailable', () => {
    assert.deepEqual(aiPlusView({ role: 'user', service: true, status: ready(true) }), { show: true, usable: true });
    assert.deepEqual(aiPlusView({ role: 'user', service: true, status: ready(true, true) }), { show: true, usable: false, reason: 'suspended' });
    // suspended wins over unavailable, like the dark badge
    assert.deepEqual(aiPlusView({ role: 'user', service: true, status: ready(false, true) }), { show: true, usable: false, reason: 'suspended' });
    assert.deepEqual(aiPlusView({ role: 'user', service: true, status: ready(false) }), { show: true, usable: false, reason: 'unavailable' });
    assert.deepEqual(aiPlusView({ role: 'user', service: true, status: DOWN }), { show: true, usable: false, reason: 'unavailable' });
  });
  it('status still loading: a user with the service is greyed with reason loading; an administrator sees nothing', () => {
    assert.deepEqual(aiPlusView({ role: 'user', service: true, status: LOADING }), { show: true, usable: false, reason: 'loading' });
    assert.deepEqual(aiPlusView({ role: 'admin', service: true, status: LOADING }), { show: false });
    assert.deepEqual(aiPlusView({ role: 'admin', service: false, status: LOADING }), { show: false });
  });
  it('administrators follow their badge: usable while available, absent otherwise, never greyed', () => {
    for (const service of [true, false]) {
      assert.deepEqual(aiPlusView({ role: 'admin', service, status: ready(true) }), { show: true, usable: true });
      assert.deepEqual(aiPlusView({ role: 'admin', service, status: ready(false) }), { show: false });
      assert.deepEqual(aiPlusView({ role: 'admin', service, status: DOWN }), { show: false });
      // an administrator is never suspended
      assert.deepEqual(aiPlusView({ role: 'admin', service, status: ready(true, true) }), { show: true, usable: true });
    }
  });
});

describe('controlState', () => {
  it('hidden view -> hidden control', () => {
    assert.deepEqual(controlState({ show: false }, 'noWrong'), { show: false });
  });
  it('usable view without a row reason is usable', () => {
    assert.deepEqual(controlState({ show: true, usable: true }), { show: true, usable: true });
  });
  it('suspended / unavailable: greyed with the tag and the badge reason in the tooltip', () => {
    assert.deepEqual(controlState({ show: true, usable: false, reason: 'suspended' }, 'noWrong'), {
      show: true, usable: false, reason: 'suspended', tag: true, tip: { key: 'aiplus.tip.na', reasonKey: 'ai.badge.suspended' },
    });
    assert.deepEqual(controlState({ show: true, usable: false, reason: 'unavailable' }), {
      show: true, usable: false, reason: 'unavailable', tag: true, tip: { key: 'aiplus.tip.na', reasonKey: 'ai.badge.unavailable' },
    });
  });
  it('loading: greyed without a tag, own tooltip', () => {
    assert.deepEqual(controlState({ show: true, usable: false, reason: 'loading' }), {
      show: true, usable: false, reason: 'loading', tag: false, tip: { key: 'aiplus.why.loading' },
    });
  });
  it('row reasons: greyed without a tag', () => {
    assert.deepEqual(controlState({ show: true, usable: true }, 'noWrong'), {
      show: true, usable: false, reason: 'noWrong', tag: false, tip: { key: 'aiplus.why.noWrong' },
    });
    assert.deepEqual(controlState({ show: true, usable: true }, 'noDetail'), {
      show: true, usable: false, reason: 'noDetail', tag: false, tip: { key: 'aiplus.why.noDetail' },
    });
    assert.deepEqual(controlState({ show: true, usable: true }, 'notSaved'), {
      show: true, usable: false, reason: 'notSaved', tag: false, tip: { key: 'aiplus.why.notSaved' },
    });
  });
});

describe('recordSummaryExtra (the noDetail / noWrong button states)', () => {
  it('noDetail comes before noWrong', () => {
    assert.equal(recordSummaryExtra({ detail: false, answered: 10, correct: 2 }), 'noDetail');
    assert.equal(recordSummaryExtra({ detail: false, answered: null, correct: 0 }), 'noDetail');
  });
  it('answered null (saved before v0.5) is noWrong', () => {
    assert.equal(recordSummaryExtra({ detail: true, answered: null, correct: 0 }), 'noWrong');
  });
  it('answered - correct <= 0 is noWrong (unanswered questions do not count)', () => {
    assert.equal(recordSummaryExtra({ answered: 8, correct: 8 }), 'noWrong');
    assert.equal(recordSummaryExtra({ answered: 0, correct: 0 }), 'noWrong');
    assert.equal(recordSummaryExtra({ answered: 3, correct: 4 }), 'noWrong');
  });
  it('some wrong answers: usable', () => {
    assert.equal(recordSummaryExtra({ detail: true, answered: 8, correct: 7 }), null);
    assert.equal(recordSummaryExtra({ answered: 20, correct: 0 }), null);
  });
});

describe('resultExtra', () => {
  it('only an exam result without a record is notSaved', () => {
    assert.equal(resultExtra({ mode: 'exam', recordId: null }), 'notSaved');
    assert.equal(resultExtra({ mode: 'exam', recordId: 4 }), null);
    assert.equal(resultExtra({ mode: 'practice', recordId: null }), null);
  });
});

describe('aiControls', () => {
  it('practice: hint and explain, each with an icon and a button', () => {
    const c = aiControls('practice');
    assert.deepEqual(c.map((x) => [x.kind, x.source]), [['hint', 'practice'], ['explain', 'practice']]);
    assert.deepEqual(c.map((x) => x.focus), [{ icon: 'q-bulb', button: 'q-aihint' }, { icon: 'q-book', button: 'q-aiexplain' }]);
    assert.deepEqual(c.map((x) => x.shortKey), ['aiplus.hint.short', 'aiplus.explain.short']);
  });
  it('the exam while it runs has no AI control at all', () => {
    assert.deepEqual(aiControls('exam'), []);
  });
  it('result pages and the record detail page: the explanation only, called AI 解析, from the result place', () => {
    for (const page of ['result', 'detail'] as const) {
      const c = aiControls(page);
      assert.equal(c.length, 1);
      assert.equal(c[0]!.kind, 'explain');
      assert.equal(c[0]!.source, 'result');
      assert.equal(c[0]!.labelKey, 'aiplus.analyze');
    }
  });
  it('popover title keys', () => {
    assert.equal(popTitleKey('hint', 'practice'), 'aiplus.pop.title.hint');
    assert.equal(popTitleKey('explain', 'practice'), 'aiplus.pop.title.explain');
    assert.equal(popTitleKey('explain', 'result'), 'aiplus.pop.title.analyze');
  });
});

describe('request bodies', () => {
  const base = { kind: 'hint', mode: 'practice', key: 'a'.repeat(32), startedAt: 1_700_000_000, questionId: 'abc123', no: 3, qlang: 'ja', ui: 'zh', source: 'practice' } as const;
  it('a practice hint', () => {
    assert.deepEqual(buildStudyAsk(base), {
      kind: 'hint', mode: 'practice', key: 'a'.repeat(32), startedAt: 1_700_000_000, questionId: 'abc123', no: 3, qlang: 'ja', lang: 'zh-CN', source: 'practice',
    });
  });
  it('the reply language is the Settings language: zh goes out as zh-CN', () => {
    assert.equal(replyLang('zh'), 'zh-CN');
    assert.equal(replyLang('en'), 'en');
    assert.equal(replyLang('ja'), 'ja');
    assert.equal(buildSummaryAsk(7, 'zh').lang, 'zh-CN');
  });
  it('the server rules are enforced before sending', () => {
    assert.throws(() => buildStudyAsk({ ...base, source: 'result', recordId: 5 }));
    assert.throws(() => buildStudyAsk({ ...base, mode: 'exam', source: 'result', recordId: 5 }));
    assert.throws(() => buildStudyAsk({ ...base, kind: 'explain', mode: 'exam', source: 'result' }));
    assert.throws(() => buildStudyAsk({ ...base, kind: 'explain', key: null }));
    assert.throws(() => buildStudyAsk({ ...base, kind: 'explain', startedAt: null }));
  });
  it('a result-page explanation carries its record; no is bounded', () => {
    const b = buildStudyAsk({ ...base, kind: 'explain', mode: 'exam', source: 'result', recordId: 9, no: 0 });
    assert.equal(b.recordId, 9);
    assert.equal(b.no, 1);
    assert.equal(buildStudyAsk({ ...base, no: 1_000_000 }).no, 100000);
  });
  it('with a record id the key and start time are optional and then left out of the body', () => {
    const b = buildStudyAsk({ ...base, kind: 'explain', key: null, startedAt: null, mode: 'practice', source: 'result', recordId: 2 });
    assert.equal('key' in b, false);
    assert.equal('startedAt' in b, false);
    assert.equal(b.recordId, 2);
    // a record that has them sends them
    const c = buildStudyAsk({ ...base, kind: 'explain', mode: 'practice', source: 'result', recordId: 2 });
    assert.equal(c.key, 'a'.repeat(32));
    assert.equal(c.startedAt, 1_700_000_000);
  });
  it('the record detail page asks like a result page: explain, source result, mode = the record kind', () => {
    const b = buildStudyAsk({ ...base, kind: 'explain', mode: 'exam', key: null, startedAt: null, source: 'result', recordId: 8 });
    assert.deepEqual([b.kind, b.mode, b.source, b.recordId], ['explain', 'exam', 'result', 8]);
  });
});

describe('atDigits', () => {
  it('turns a record time into 12 digits', () => {
    assert.equal(atDigits('2026-11-11 21:58'), '202611112158');
  });
  it('anything else is null (no lookup, generate)', () => {
    for (const s of ['', '2026-11-11', '2026-11-11 21:58:01', '2026/11/11 21:58', ' 2026-11-11 21:58']) assert.equal(atDigits(s), null);
  });
});

describe('basedKey', () => {
  it('all four cases', () => {
    assert.equal(basedKey({ used: 12, usable: 12, wrong: 12 }), 'aiplus.sum.basedAll');
    assert.equal(basedKey({ used: 70, usable: 85, wrong: 85 }), 'aiplus.sum.basedSome');
    assert.equal(basedKey({ used: 65, usable: 65, wrong: 75 }), 'aiplus.sum.basedGone');
    assert.equal(basedKey({ used: 70, usable: 75, wrong: 80 }), 'aiplus.sum.basedSomeGone');
  });
});

describe('chatDisplayTitle', () => {
  it('study chats get a suffix key, normal chats do not', () => {
    const t = '2026-11-11 21:58';
    assert.deepEqual(chatDisplayTitle({ title: t, kind: 'practice' }), { text: t, suffixKey: 'aiplus.chat.suffix.practice' });
    assert.deepEqual(chatDisplayTitle({ title: t, kind: 'exam' }), { text: t, suffixKey: 'aiplus.chat.suffix.exam' });
    assert.deepEqual(chatDisplayTitle({ title: t, kind: 'summary' }), { text: t, suffixKey: 'aiplus.chat.suffix.summary' });
    assert.deepEqual(chatDisplayTitle({ title: 'Hello', kind: 'chat' }), { text: 'Hello', suffixKey: null });
    assert.deepEqual(chatDisplayTitle({ title: 'Old' }), { text: 'Old', suffixKey: null });
  });
});

describe('busyView', () => {
  const run = (o: Partial<RunningInfo>): RunningInfo => ({
    chatId: 4, messageId: 9, chatKind: 'practice', kind: 'hint', questionId: 'q1', questionNo: 3, source: 'practice', recordId: null, ...o,
  });
  it('null -> other', () => {
    assert.deepEqual(busyView(null, 4), { kind: 'other', noteKey: 'aiplus.busy.other', canShow: false });
  });
  it('a hint or explanation of this session: named, [查看] offered', () => {
    assert.deepEqual(busyView(run({}), 4), { kind: 'study', labelKey: 'aiplus.hint', n: 3, noteKey: 'aiplus.busy.study', canShow: true });
    const e = busyView(run({ kind: 'explain', source: 'result', chatKind: 'exam' }), 4);
    assert.equal(e.kind === 'study' && e.labelKey, 'aiplus.analyze');
    assert.equal(e.canShow, true);
    const p = busyView(run({ kind: 'explain' }), 4);
    assert.equal(p.kind === 'study' && p.labelKey, 'aiplus.explain');
  });
  it('another chat (or an unknown session chat): named but not offered', () => {
    assert.equal(busyView(run({}), 5).canShow, false);
    assert.equal(busyView(run({}), null).canShow, false);
  });
  it('summary, AI Chat and a follow-up typed in a study chat', () => {
    assert.equal(busyView(run({ kind: 'summary', chatKind: 'summary', questionId: null }), 4).kind, 'summary');
    assert.equal(busyView(run({ kind: 'chat', chatKind: 'chat', questionId: null }), 4).kind, 'chat');
    assert.equal(busyView(run({ kind: 'chat', chatKind: 'practice', questionId: null }), 4).kind, 'chat');
  });
});
