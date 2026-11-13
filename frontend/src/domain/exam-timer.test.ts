import assert from 'node:assert/strict';
import test from 'node:test';
import {
  UNTIMED,
  attemptTail,
  autoTick,
  countdownText,
  defaultFilter,
  examAnswer,
  isEmptyAnswer,
  isTimeUp,
  markUnanswered,
  noticeDue,
  parseLimitMinutes,
  readLimitControls,
  readTimedFields,
  recordSeconds,
  remaining,
  resultCounts,
  resumeView,
  srDue,
  submitConfirm,
  timerTone,
  writeTimedFields,
} from './exam-timer.ts';

test('parseLimitMinutes accepts whole minutes 1 ... 600', () => {
  assert.deepEqual(parseLimitMinutes('1'), { ok: true, minutes: 1, seconds: 60 });
  assert.deepEqual(parseLimitMinutes('90'), { ok: true, minutes: 90, seconds: 5400 });
  assert.deepEqual(parseLimitMinutes('600'), { ok: true, minutes: 600, seconds: 36000 });
  assert.deepEqual(parseLimitMinutes('  45 '), { ok: true, minutes: 45, seconds: 2700 });
  assert.deepEqual(parseLimitMinutes('007'), { ok: true, minutes: 7, seconds: 420 });
});

test('parseLimitMinutes converts full-width digits', () => {
  assert.deepEqual(parseLimitMinutes('６０'), { ok: true, minutes: 60, seconds: 3600 });
  assert.deepEqual(parseLimitMinutes('１２a'), { ok: false, reason: 'notInt' });
});

test('parseLimitMinutes reports empty, notInt and range', () => {
  assert.deepEqual(parseLimitMinutes(''), { ok: false, reason: 'empty' });
  assert.deepEqual(parseLimitMinutes('   '), { ok: false, reason: 'empty' });
  for (const bad of ['1.5', '-3', '3e2', '12a', '+5', '1 2', '1,5']) {
    assert.deepEqual(parseLimitMinutes(bad), { ok: false, reason: 'notInt' }, bad);
  }
  assert.deepEqual(parseLimitMinutes('0'), { ok: false, reason: 'range' });
  assert.deepEqual(parseLimitMinutes('000'), { ok: false, reason: 'range' });
  assert.deepEqual(parseLimitMinutes('601'), { ok: false, reason: 'range' });
  assert.deepEqual(parseLimitMinutes('99999999999999999999'), { ok: false, reason: 'range' });
});

test('readLimitControls ignores the text while unticked', () => {
  assert.deepEqual(readLimitControls(false, ''), { ok: true, limit: null });
  assert.deepEqual(readLimitControls(false, 'garbage'), { ok: true, limit: null });
  assert.deepEqual(readLimitControls(false, '30'), { ok: true, limit: null });
});

test('readLimitControls validates the text when ticked', () => {
  assert.deepEqual(readLimitControls(true, '30'), { ok: true, limit: 1800 });
  assert.deepEqual(readLimitControls(true, '１'), { ok: true, limit: 60 });
  assert.deepEqual(readLimitControls(true, ''), { ok: false, reason: 'empty' });
  assert.deepEqual(readLimitControls(true, 'x'), { ok: false, reason: 'notInt' });
  assert.deepEqual(readLimitControls(true, '601'), { ok: false, reason: 'range' });
});

test('autoTick fires only for the first digit into an empty, unticked field', () => {
  assert.equal(autoTick('', '3', false), true);
  assert.equal(autoTick('  ', '３', false), true);
  assert.equal(autoTick('', '', false), false);
  assert.equal(autoTick('', 'abc', false), false);
  assert.equal(autoTick('3', '30', false), false);
  assert.equal(autoTick('', '3', true), false);
  // Unticked by hand with digits kept: more typing does not tick again.
  assert.equal(autoTick('30', '305', false), false);
});

test('remaining and isTimeUp clamp at zero and floor the elapsed time', () => {
  assert.equal(remaining(600, 0), 600);
  assert.equal(remaining(600, 0.9), 600);
  assert.equal(remaining(600, 1.2), 599);
  assert.equal(remaining(600, 600), 0);
  assert.equal(remaining(600, 99999), 0);
  assert.equal(remaining(600, -5), 600);
  assert.equal(remaining(600, Number.NaN), 600);
  assert.equal(isTimeUp(600, 599.9), false);
  assert.equal(isTimeUp(600, 600), true);
  assert.equal(isTimeUp(600, 86400), true);
});

test('countdownText formats mm:ss and h:mm:ss', () => {
  assert.equal(countdownText(0), '00:00');
  assert.equal(countdownText(59), '00:59');
  assert.equal(countdownText(60), '01:00');
  assert.equal(countdownText(3599), '59:59');
  assert.equal(countdownText(3600), '1:00:00');
  assert.equal(countdownText(3661), '1:01:01');
  assert.equal(countdownText(36000), '10:00:00');
  assert.equal(countdownText(-4), '00:00');
  assert.equal(countdownText(59.9), '00:59');
});

test('timerTone: warn from 30 minutes of a longer limit, danger at 5 minutes and 1 minute', () => {
  assert.equal(timerTone(3600, 3600), 'normal');
  assert.equal(timerTone(3600, 1801), 'normal');
  assert.equal(timerTone(3600, 1800), 'warn');
  assert.equal(timerTone(3600, 301), 'warn');
  assert.equal(timerTone(3600, 300), 'danger');
  assert.equal(timerTone(3600, 0), 'danger');
  // A limit of exactly 30 minutes never warns.
  assert.equal(timerTone(1800, 1800), 'normal');
  assert.equal(timerTone(1800, 301), 'normal');
  assert.equal(timerTone(1800, 300), 'danger');
  // A limit of 5 minutes or less is not danger from the start, except the last minute.
  assert.equal(timerTone(300, 300), 'normal');
  assert.equal(timerTone(300, 61), 'normal');
  assert.equal(timerTone(300, 60), 'danger');
  assert.equal(timerTone(60, 60), 'danger');
});

test('noticeDue: the 30-minute notice and its boundaries', () => {
  assert.equal(noticeDue(1800, 1800, false), false, 'limit of exactly 30 minutes never');
  assert.equal(noticeDue(1200, 900, false), false, 'shorter limit never');
  assert.equal(noticeDue(1801, 1801, false), false);
  assert.equal(noticeDue(1801, 1800, false), true);
  assert.equal(noticeDue(3600, 1800, false), true);
  assert.equal(noticeDue(3600, 1799, false), true, 'late first tick still shows it');
  assert.equal(noticeDue(3600, 1, false), true);
  assert.equal(noticeDue(3600, 0, false), false, 'time up, no notice');
  assert.equal(noticeDue(3600, 1500, true), false, 'already warned');
  assert.equal(noticeDue(3600, 2000, false), false);
});

test('srDue: crossings of 300 s and 60 s', () => {
  assert.equal(srDue(3600, 301, 300), 'm5');
  assert.equal(srDue(3600, 300, 299), null);
  assert.equal(srDue(3600, 61, 60), 'm1');
  assert.equal(srDue(3600, 60, 59), null);
  assert.equal(srDue(3600, 1000, 900), null);
  assert.equal(srDue(3600, 500, 100), 'm5', 'jump over 300 only');
  assert.equal(srDue(3600, 400, 30), 'm1', 'jump over both gives m1 only');
  assert.equal(srDue(3600, 61, 0), null, 'time up: nothing');
});

test('srDue: short limits', () => {
  assert.equal(srDue(300, 300, 299), null, 'limit of 5 minutes has no 5-minute line');
  assert.equal(srDue(300, 61, 60), 'm1');
  assert.equal(srDue(60, 60, 59), null, 'limit of 1 minute has no 1-minute line');
  assert.equal(srDue(301, 301, 300), 'm5');
});

test('recordSeconds never exceeds the limit', () => {
  assert.equal(recordSeconds(null, 1234.9), 1234);
  assert.equal(recordSeconds(600, 599.5), 599);
  assert.equal(recordSeconds(600, 600), 600);
  assert.equal(recordSeconds(600, 5000), 600);
  assert.equal(recordSeconds(null, -3), 0);
});

test('readTimedFields accepts a valid limit with its flags', () => {
  assert.deepEqual(readTimedFields({ limit: 3600, warned: true, timeUp: false }), { limit: 3600, warned: true, timeUp: false });
  assert.deepEqual(readTimedFields({ limit: 60 }), { limit: 60, warned: false, timeUp: false });
  assert.deepEqual(readTimedFields({ limit: 36000, timeUp: true }), { limit: 36000, warned: false, timeUp: true });
  assert.deepEqual(readTimedFields({ timeUp: true, limit: 600 }), { limit: 600, warned: false, timeUp: true });
});

test('readTimedFields drops everything for a bad limit', () => {
  const bad: unknown[] = [
    { limit: 59, warned: true, timeUp: true },
    { limit: 36001, warned: true },
    { limit: 3600.5, timeUp: true },
    { limit: '3600', warned: true },
    { limit: null, warned: true },
    { limit: Number.NaN },
    { limit: Infinity },
    { warned: true },
    { timeUp: true },
    {},
    null,
    undefined,
    'text',
    42,
  ];
  for (const b of bad) assert.deepEqual(readTimedFields(b), { limit: null, warned: false, timeUp: false }, JSON.stringify(b));
});

test('readTimedFields treats non-boolean flags as false and returns fresh objects', () => {
  assert.deepEqual(readTimedFields({ limit: 600, warned: 'yes', timeUp: 1 }), { limit: 600, warned: false, timeUp: false });
  const a = readTimedFields({});
  a.warned = true;
  assert.equal(UNTIMED.warned, false);
  assert.equal(readTimedFields({}).warned, false);
});

test('writeTimedFields writes nothing for an untimed exam and round-trips a timed one', () => {
  assert.deepEqual(writeTimedFields(UNTIMED), {});
  assert.deepEqual(writeTimedFields({ limit: null, warned: true, timeUp: true }), {});
  const f = { limit: 2400, warned: true, timeUp: false };
  assert.deepEqual(writeTimedFields(f), { limit: 2400, warned: true, timeUp: false });
  assert.deepEqual(readTimedFields(writeTimedFields(f)), f);
});

test('attemptTail: practice sends nothing, every exam sends end', () => {
  const timed = { limit: 3600, warned: false, timeUp: false };
  assert.deepEqual(attemptTail('practice', UNTIMED, 'submit'), {});
  assert.deepEqual(attemptTail('practice', timed, 'timeUp'), {});
  assert.deepEqual(attemptTail('exam', UNTIMED, 'submit'), { end: 'submit' });
  assert.deepEqual(attemptTail('exam', UNTIMED, 'timeUp'), { end: 'submit' });
  assert.deepEqual(attemptTail('exam', timed, 'submit'), { limit: 3600, end: 'submit' });
  assert.deepEqual(attemptTail('exam', timed, 'timeUp'), { limit: 3600, end: 'timeUp' });
});

test('isEmptyAnswer', () => {
  assert.equal(isEmptyAnswer(null), true);
  assert.equal(isEmptyAnswer(undefined), true);
  assert.equal(isEmptyAnswer([]), true);
  assert.equal(isEmptyAnswer([null, null]), true);
  assert.equal(isEmptyAnswer([undefined]), true);
  assert.equal(isEmptyAnswer(['A']), false);
  assert.equal(isEmptyAnswer([0, null]), false);
  assert.equal(isEmptyAnswer([null, 2]), false);
  assert.equal(isEmptyAnswer('A'), false);
  assert.equal(isEmptyAnswer(0), false);
  assert.equal(isEmptyAnswer(''), false);
});

const row = (ans: unknown, correct: boolean) => ({ ans, correct });

test('markUnanswered marks empty, not-correct rows of an exam, timed or not', () => {
  const rows = [row(null, false), row([], false), row([null, null], false), row(undefined, false)];
  for (const r of markUnanswered(rows, 'exam')) assert.equal(r.unanswered, true);
});

test('markUnanswered leaves answered, partly answered and correct rows alone', () => {
  const rows = [row(['A'], false), row([0, null], false), row(null, true), row([], true), row('B', false)];
  for (const r of markUnanswered(rows, 'exam')) assert.equal(r.unanswered, undefined);
});

test('markUnanswered keeps practice rows unchanged and does not mutate the input', () => {
  const rows = [row(null, false), row([], false), row([null], false)];
  const out = markUnanswered(rows, 'practice');
  assert.deepEqual(out, rows);
  assert.notEqual(out, rows);
  for (const r of out) assert.equal('unanswered' in r, false);
  markUnanswered(rows, 'exam');
  for (const r of rows) assert.equal('unanswered' in r, false);
});

test('examAnswer sends null only for a not-correct empty answer', () => {
  assert.equal(examAnswer(null, false), null);
  assert.equal(examAnswer(undefined, false), null);
  assert.equal(examAnswer([], false), null);
  assert.equal(examAnswer([null, null], false), null);
  assert.deepEqual(examAnswer([0, null], false), [0, null]);
  assert.deepEqual(examAnswer(['A'], false), ['A']);
  assert.equal(examAnswer('A', false), 'A');
  assert.deepEqual(examAnswer([], true), []);
  assert.equal(examAnswer(null, true), null);
});

test('submitConfirm picks the dialog kind', () => {
  assert.deepEqual(submitConfirm(0, 0), { kind: 'confirm', partial: 0 });
  assert.deepEqual(submitConfirm(3, 3), { kind: 'empty', partial: 0 });
  assert.deepEqual(submitConfirm(3, 2), { kind: 'empty', partial: 1 });
  assert.deepEqual(submitConfirm(1, 0), { kind: 'partial', partial: 1 });
  assert.deepEqual(submitConfirm(5, 0), { kind: 'partial', partial: 5 });
  assert.deepEqual(submitConfirm(1, 4), { kind: 'empty', partial: 0 }, 'empty larger than open never goes negative');
});

test('resultCounts: 40 questions, 30 right, 6 wrong, 4 not answered', () => {
  const rows = [
    ...Array.from({ length: 30 }, () => ({ correct: true })),
    ...Array.from({ length: 6 }, () => ({ correct: false })),
    ...Array.from({ length: 4 }, () => ({ correct: false, unanswered: true })),
  ];
  const c = resultCounts(rows);
  assert.deepEqual(c, { good: 30, wrong: 6, open: 4 });
  assert.equal(Math.round((c.good / rows.length) * 100), 75);
  assert.deepEqual(resultCounts([]), { good: 0, wrong: 0, open: 0 });
  assert.deepEqual(resultCounts([{ correct: true, unanswered: true }]), { good: 1, wrong: 0, open: 0 });
  assert.deepEqual(resultCounts([{ correct: false, unanswered: false }]), { good: 0, wrong: 1, open: 0 });
});

test('defaultFilter', () => {
  assert.equal(defaultFilter({ wrong: 2, open: 3 }), 'wrong');
  assert.equal(defaultFilter({ wrong: 1, open: 0 }), 'wrong');
  assert.equal(defaultFilter({ wrong: 0, open: 3 }), 'open');
  assert.equal(defaultFilter({ wrong: 0, open: 0 }), 'all');
});

test('resumeView', () => {
  assert.deepEqual(resumeView(UNTIMED, 5000), { left: null, timeUp: false });
  const f = { limit: 600, warned: false, timeUp: false };
  assert.deepEqual(resumeView(f, 100), { left: 500, timeUp: false });
  assert.deepEqual(resumeView(f, 600), { left: 0, timeUp: true });
  assert.deepEqual(resumeView(f, 9999), { left: 0, timeUp: true });
  assert.deepEqual(resumeView({ ...f, timeUp: true }, 100), { left: 500, timeUp: true }, 'saved flag wins');
});

test('a hidden tab: far past the limit is time up, one tick of remaining time is not', () => {
  assert.equal(isTimeUp(1800, 1799.4), false);
  assert.equal(remaining(1800, 1799.4), 1);
  assert.equal(isTimeUp(1800, 1800.0), true);
  assert.equal(isTimeUp(1800, 100000), true);
  assert.equal(noticeDue(3600, remaining(3600, 100000), false), false);
});

// ---- v1.1 texts of the study pages (M-1, M-10): the owner's wording, nothing of the old rules left ----
import common from '../i18n/catalog/common.ts';
import home from '../i18n/catalog/home.ts';
import quiz from '../i18n/catalog/quiz.ts';
import result from '../i18n/catalog/result.ts';
import records from '../i18n/catalog/records.ts';

const study = [common, home, quiz, result, records];

test('M-1: no old "unfinished counts as wrong" wording is left in the study catalogs; the label is 没有完成的题', () => {
  const banned = ['没做完', '没答完', '按答错计分', 'count as wrong. Submit', 'Unfinished questions'];
  for (const c of study) {
    for (const lang of ['en', 'zh', 'ja'] as const) {
      for (const [k, v] of Object.entries(c[lang] as Record<string, string>)) {
        for (const b of banned) assert.ok(!v.includes(b) || k.startsWith('act.submit.partial') || k.startsWith('home.resume.') || k.startsWith('quiz.overwrite.'), `${lang} ${k} has "${b}"`);
      }
    }
  }
  const en = result.en as Record<string, string>;
  const zh = result.zh as Record<string, string>;
  const ja = result.ja as Record<string, string>;
  assert.equal(zh['result.review.onlyOpen'], '没有完成的题（{n}）');
  assert.equal(en['result.review.onlyOpen'], 'Not completed ({n})');
  assert.equal(ja['result.review.onlyOpen'], '未回答（{n}）');
});

test('M-10: a partly answered hotspot counts as wrong - the dialog says so (zh / en / ja), no "as usual" wording is left', () => {
  const c = common as unknown as { en: Record<string, string>; zh: Record<string, string>; ja: Record<string, string> };
  assert.equal(c.zh['act.submit.partialTitle'], '有 {n} 道热点题只答了一部分');
  assert.equal(c.zh['act.submit.partialBody'], '只答了一部分的热点题按答错计算。现在交卷吗？');
  assert.equal(c.en['act.submit.partialTitle'], '{n} hotspot questions are only partly answered');
  assert.equal(c.en['act.submit.partialTitle_one'], '{n} hotspot question is only partly answered');
  assert.equal(c.en['act.submit.partialBody'], 'Partly answered hotspot questions count as wrong. Submit now?');
  assert.equal(c.ja['act.submit.partialTitle'], '一部だけ回答したホットスポット問題が {n} 問あります');
  assert.equal(c.ja['act.submit.partialBody'], '一部だけ回答したホットスポット問題は不正解になります。提出しますか？');
  for (const lang of ['en', 'zh', 'ja'] as const) {
    for (const [k, v] of Object.entries(c[lang])) {
      assert.ok(!/照常判分|graded as usual|通常どおり/.test(v), `${lang} ${k}`);
    }
  }
  // the three v1.0 keys of the old confirm are gone
  for (const lang of ['en', 'zh', 'ja'] as const) {
    for (const k of ['act.submit.openTitle', 'act.submit.openTitle_one', 'act.submit.openBody']) assert.ok(!(k in c[lang]), `${lang} ${k}`);
  }
});

test('M-10: a partly answered hotspot is answered (wrong, not "not completed"); an all-null one is not completed', () => {
  const rows = [
    { ans: [0, null], correct: false },
    { ans: [null, null], correct: false },
    { ans: null, correct: false },
  ];
  assert.deepEqual(markUnanswered(rows, 'exam').map((r) => r.unanswered === true), [false, true, true]);
  assert.equal(examAnswer([0, null], false) instanceof Array, true);
  assert.equal(examAnswer([null, null], false), null);
});
