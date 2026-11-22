import assert from 'node:assert/strict';
import { test } from 'node:test';
import { greetName, popularRows, rankNote } from './dash.ts';

test('greetName takes the local part of an e-mail username', () => {
  assert.equal(greetName('alice'), 'alice');
  assert.equal(greetName('bob@example.invalid'), 'bob');
  assert.equal(greetName('  carol  '), 'carol');
  assert.equal(greetName('@example.invalid'), '@example.invalid');
  assert.equal(greetName('小林@example.invalid'), '小林');
  assert.equal(greetName(''), '');
});

const bank = {
  questions: [
    { id: 'q1', stem: 'Which   port\nis HTTP?', sheet: 'Net', bank: 1 },
    { id: 'q2', stem: 'Second sample', sheet: 'Misc', bank: 2 },
  ],
};

test('popularRows resolves stems, bank and sheet, and drops unknown ids', () => {
  const rows = popularRows(
    [
      { questionId: 'q2', bankId: 2, times: 9, correct: 5 },
      { questionId: 'gone', bankId: 1, times: 7, correct: 1 },
      { questionId: 'q1', bankId: 1, times: 3, correct: 3 },
    ],
    bank,
  );
  assert.deepEqual(rows, [
    { id: 'q2', stem: 'Second sample', bankId: 2, sheet: 'Misc', times: 9 },
    { id: 'q1', stem: 'Which port is HTTP?', bankId: 1, sheet: 'Net', times: 3 },
  ]);
});

test('popularRows without a loaded bank is empty', () => {
  assert.deepEqual(popularRows([{ questionId: 'q1', bankId: 1, times: 1, correct: 0 }], null), []);
});

test('rankNote: an empty board shows one line only, the invitation', () => {
  // Empty install: nobody ranked, the viewer neither -> only the invitation, not "nobody has answered" as well
  assert.equal(rankNote(0, null), 'noneYou');
  // Others ranked, the viewer not -> the invitation under the rows
  assert.equal(rankNote(3, null), 'noneYou');
  // The viewer ranked -> nothing more
  assert.equal(rankNote(3, 2), null);
  assert.equal(rankNote(0, 1), 'empty');
});
