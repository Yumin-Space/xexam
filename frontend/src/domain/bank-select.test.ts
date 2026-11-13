// What deleting the checked sheets of Bank management means (BL-3).
import assert from 'node:assert/strict';
import test from 'node:test';
import { deletionPlan, partlySelected, selectionCount, sheetSelKey, type SelBank } from './bank-select.ts';

const banks: SelBank[] = [
  { id: 1, ai: false, sheets: ['Ovens', 'Knives', 'Sinks'] },
  { id: 2, ai: false, sheets: ['Only'] },
  { id: 3, ai: true, sheets: ['AI'] },
];
const k = (b: number, s: string): string => sheetSelKey(b, s);

test('nothing checked: nothing to do', () => {
  assert.deepEqual(deletionPlan([], banks), { banks: [], parts: [] });
  assert.equal(selectionCount([], banks), 0);
});

test('every sheet of a bank goes whole; a partly checked bank loses only its sheets', () => {
  assert.deepEqual(deletionPlan([k(1, 'Ovens'), k(1, 'Knives'), k(1, 'Sinks')], banks), { banks: [1], parts: [] });
  assert.deepEqual(deletionPlan([k(1, 'Knives'), k(1, 'Ovens')], banks), { banks: [], parts: [{ id: 1, sheets: ['Ovens', 'Knives'] }] });
  // a one-sheet bank checked is the whole bank
  assert.deepEqual(deletionPlan([k(2, 'Only')], banks), { banks: [2], parts: [] });
});

test('an AI bank only goes whole; mixed selections keep the list order', () => {
  assert.deepEqual(deletionPlan([k(3, 'AI')], banks), { banks: [3], parts: [] });
  assert.deepEqual(deletionPlan([k(3, 'AI'), k(1, 'Sinks'), k(2, 'Only')], banks), { banks: [2, 3], parts: [{ id: 1, sheets: ['Sinks'] }] });
});

test('keys of banks or sheets that are not listed are ignored; counts and the tri-state', () => {
  const keys = [k(1, 'Ovens'), k(9, 'Ghost'), k(1, 'Nope'), k(2, 'Only')];
  assert.equal(selectionCount(keys, banks), 2);
  assert.deepEqual(deletionPlan(keys, banks), { banks: [2], parts: [{ id: 1, sheets: ['Ovens'] }] });
  assert.equal(partlySelected(keys, banks[0]!), true);
  assert.equal(partlySelected(keys, banks[1]!), false);
  assert.equal(partlySelected([], banks[0]!), false);
  assert.equal(sheetSelKey(1, 'a'), '1\u0000a');
});
