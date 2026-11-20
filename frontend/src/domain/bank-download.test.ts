import assert from 'node:assert/strict';
import test from 'node:test';
import {
  canonicalLangs, DOWNLOAD_FORMATS, DOWNLOAD_LANGS, downloadPath, fallbackFileName, filenameFromDisposition, FORMAT_CHOICES,
} from './bank-download.ts';

test('downloadPath builds the query', () => {
  assert.equal(downloadPath(7, 'xlsx'), '/api/banks/download?id=7&format=xlsx');
  assert.equal(downloadPath(12, 'docx'), '/api/banks/download?id=12&format=docx');
});

test('filenameFromDisposition reads the RFC 5987 name first', () => {
  const h = `attachment; filename="xexam-bank-7.xlsx"; filename*=UTF-8''%E7%A4%BA%E4%BE%8B%E9%A2%98%E5%BA%93.xlsx`;
  assert.equal(filenameFromDisposition(h), '示例题库.xlsx');
});

test('filenameFromDisposition reads quoted, bare and missing names', () => {
  assert.equal(filenameFromDisposition('attachment; filename="sample bank.docx"'), 'sample bank.docx');
  assert.equal(filenameFromDisposition('attachment; filename=sample.xlsx'), 'sample.xlsx');
  assert.equal(filenameFromDisposition(null), null);
  assert.equal(filenameFromDisposition(''), null);
  assert.equal(filenameFromDisposition('attachment'), null);
});

test('filenameFromDisposition survives a malformed percent escape', () => {
  assert.equal(filenameFromDisposition(`attachment; filename*=UTF-8''%E7%A4%`), null);
  assert.equal(filenameFromDisposition(`attachment; filename="a.xlsx"; filename*=UTF-8''%E7%A4%`), 'a.xlsx');
});

test('fallbackFileName follows the backend rule', () => {
  assert.equal(fallbackFileName('Sample bank', 'xlsx'), 'Sample bank.xlsx');
  assert.equal(fallbackFileName('a/b:c*d?"e<f>g|h\\i', 'docx'), 'a_b_c_d__e_f_g_h_i.docx');
  assert.equal(fallbackFileName('...', 'xlsx'), 'bank.xlsx');
  assert.equal(fallbackFileName('  . x .  ', 'xlsx'), 'x.xlsx');
  assert.equal(Array.from(fallbackFileName('a'.repeat(200), 'xlsx')).length, 120 + '.xlsx'.length);
});

test('FORMAT_CHOICES: Excel first, then Word', () => {
  assert.deepEqual(DOWNLOAD_FORMATS, ['xlsx', 'docx']);
  assert.deepEqual(FORMAT_CHOICES.map((c) => c.format), ['xlsx', 'docx']);
  assert.deepEqual(FORMAT_CHOICES.map((c) => c.icon), ['excel', 'word']);
  assert.equal(FORMAT_CHOICES[0]?.label, 'banks.dl.excel');
  assert.equal(FORMAT_CHOICES[1]?.aria, 'banks.dl.wordAria');
});

test('downloadPath: no languages or all three keep the old path', () => {
  assert.equal(downloadPath(7, 'xlsx', undefined), '/api/banks/download?id=7&format=xlsx');
  assert.equal(downloadPath(7, 'xlsx', ['zh', 'en', 'ja']), '/api/banks/download?id=7&format=xlsx');
  assert.equal(downloadPath(7, 'docx', ['ja', 'ja', 'zh', 'en']), '/api/banks/download?id=7&format=docx');
});

test('downloadPath: a subset is written in English, Chinese, Japanese order with a literal comma', () => {
  assert.equal(downloadPath(7, 'docx', ['zh', 'en']), '/api/banks/download?id=7&format=docx&langs=en,zh');
  assert.equal(downloadPath(7, 'xlsx', ['ja']), '/api/banks/download?id=7&format=xlsx&langs=ja');
  assert.equal(downloadPath(7, 'xlsx', ['ja', 'zh']), '/api/banks/download?id=7&format=xlsx&langs=zh,ja');
});

test('canonicalLangs orders and de-duplicates; the dialog lists Chinese, English, Japanese', () => {
  assert.deepEqual(canonicalLangs(['ja', 'en', 'ja', 'zh']), ['en', 'zh', 'ja']);
  assert.deepEqual(canonicalLangs([]), []);
  assert.deepEqual(DOWNLOAD_LANGS, ['zh', 'en', 'ja']);
});

test('fallbackFileName: a subset adds _<codes>, all three do not', () => {
  assert.equal(fallbackFileName('Garden', 'docx', ['zh', 'en']), 'Garden_en-zh.docx');
  assert.equal(fallbackFileName('Garden', 'xlsx', ['ja']), 'Garden_ja.xlsx');
  assert.equal(fallbackFileName('Garden', 'xlsx', ['ja', 'zh', 'en']), 'Garden.xlsx');
  assert.equal(fallbackFileName('Garden', 'xlsx'), 'Garden.xlsx');
  assert.equal(fallbackFileName('', 'docx', ['en', 'ja']), 'bank_en-ja.docx');
});
