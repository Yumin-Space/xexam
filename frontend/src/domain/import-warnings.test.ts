// Preview warnings of a six-column import: grouping per row, the language of each warning, the totals, and that the
// backend still sends the two reasons this file knows.
import assert from 'node:assert/strict';
import test from 'node:test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { WARN_JA_DIFFERS, WARN_ZH_DIFFERS, diffLangOf, rowWarnings, sheetWarnings, warningTotal } from './import-warnings.ts';

test('the reasons match the backend constants', () => {
  const src = readFileSync(fileURLToPath(new URL('../../../backend/src/parser/question.rs', import.meta.url)), 'utf8');
  assert.ok(src.includes(`WARN6_ZH_DIFFERS: &str = "${WARN_ZH_DIFFERS}"`), 'WARN6_ZH_DIFFERS changed in the backend');
  assert.ok(src.includes(`WARN6_JA_DIFFERS: &str = "${WARN_JA_DIFFERS}"`), 'WARN6_JA_DIFFERS changed in the backend');
});

test('each reason maps to its language; anything else is not a language', () => {
  assert.equal(diffLangOf(WARN_ZH_DIFFERS), 'zh');
  assert.equal(diffLangOf(WARN_JA_DIFFERS), 'ja');
  assert.equal(diffLangOf('something else'), null);
});

test('warnings of the same row are joined, Chinese first, rows in order, duplicates dropped', () => {
  const rows = rowWarnings([
    { row: 9, reason: WARN_JA_DIFFERS },
    { row: 4, reason: WARN_ZH_DIFFERS },
    { row: 9, reason: WARN_ZH_DIFFERS },
    { row: 9, reason: WARN_ZH_DIFFERS },
    { row: 12, reason: 'a new reason' },
  ]);
  assert.deepEqual(rows, [
    { row: 4, langs: ['zh'], other: [] },
    { row: 9, langs: ['zh', 'ja'], other: [] },
    { row: 12, langs: [], other: ['a new reason'] },
  ]);
});

test('only sheets with warnings are listed, with the count over the cap', () => {
  const sheets = [
    { name: 'Demo 1', warnings: [{ row: 3, reason: WARN_JA_DIFFERS }], warningsMore: 0 },
    { name: 'Demo 2' },
    { name: 'Demo 3', warnings: [], warningsMore: 0 },
    { name: 'Demo 4', warnings: [{ row: 5, reason: WARN_ZH_DIFFERS }], warningsMore: 7 },
  ];
  assert.deepEqual(sheetWarnings(sheets), [
    { sheet: 'Demo 1', rows: [{ row: 3, langs: ['ja'], other: [] }], more: 0 },
    { sheet: 'Demo 4', rows: [{ row: 5, langs: ['zh'], other: [] }], more: 7 },
  ]);
});

test('the total: the top-level count, else the sheets (listed + not listed), else 0', () => {
  const sheets = [
    { warnings: [{ row: 3, reason: WARN_JA_DIFFERS }], warningsMore: 2 },
    { warnings: [{ row: 5, reason: WARN_ZH_DIFFERS }], warningsCount: 4 },
  ];
  assert.equal(warningTotal({ warnings: 9, sheets }), 9);
  assert.equal(warningTotal({ sheets }), 3 + 4);
  assert.equal(warningTotal({ sheets: [{}] }), 0);
  // a 3 / 4-column file: no warning keys at all
  assert.equal(warningTotal({ sheets: [{ warnings: undefined }] }), 0);
});
