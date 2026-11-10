// Drift check between the backend's user-facing messages and their translations in server-messages.ts.
//
// The backend sources (backend/src) are read with a small Rust lexer: comments, `#[cfg(test)]` items, assert! / log /
// console macros and regular expressions are left out; every remaining string literal that contains Chinese or Japanese
// characters is a user-facing message unless it is listed in NOT_MESSAGES (with the reason). Format strings get sample
// values for their `{}` / `{name}` parts. Each message must then be translated for English and Japanese, keep the sample
// values, and contain no Chinese left over. Run with `node --experimental-strip-types --test` (npm test).
import assert from 'node:assert/strict';
import test from 'node:test';
import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import { knownServerMessages, localizeServerMessage } from './server-messages.ts';

const BACKEND_SRC = fileURLToPath(new URL('../../../backend/src/', import.meta.url));

/** The files that hold the backend's messages today; they must exist, so a rename cannot silently disable the check. */
const REQUIRED_FILES = [
  'error.rs',
  'api/response.rs',
  'api/guard.rs',
  'api/attempt.rs',
  'services/mod.rs',
  'parser/workbook.rs',
  'parser/question.rs',
  'security/mod.rs',
];

/** Files left out as a whole, with the reason. Test files (`tests.rs`, `tests/`) are always left out. */
const FILE_SKIPS: Record<string, string> = {
  'main.rs': 'console output of the command-line program; never sent to the browser',
  'database/schema.rs': 'SQL schema text (its comments are Chinese); not a message',
};

/** Chinese / Japanese string literals that are not messages shown to the user. */
const NOT_MESSAGES: ReadonlyArray<{ reason: string; values: readonly string[] }> = [
  {
    reason: 'header words: a row whose column A is one of these is a table header (parser/workbook.rs HEADER_WORDS)',
    values: ['题目', '題目', '问题', '問題'],
  },
  {
    reason: 'answer markers searched for in the workbook cells (parser/workbook.rs detect_four_columns)',
    values: ['【答案】', '【正解】', '【答え】'],
  },
  {
    reason: 'section heading words recognized inside 【…】 (parser/question.rs classify_heading)',
    values: ['答案', '正解', '解析', '解説', '口诀', '口訣', '记忆', '記憶', '翻译', '翻訳', '译文'],
  },
  {
    reason: 'yes / no words accepted in hotspot answers (parser/question.rs is_yes / is_no)',
    values: ['是', 'はい', '〇', '对', '對', '正确', '正確', '否', 'いいえ', '错', '錯', '错误', '不正確'],
  },
  {
    reason: 'first-line marker of a hotspot question, also stored as its tag (parser/question.rs); workbook data',
    values: ['ホットスポット'],
  },
  {
    reason: 'default file name of an upload without one (security/mod.rs clean_filename); shown as a file name, not a message',
    values: ['未命名.xlsx'],
  },
  {
    reason: 'result text of the progress.json migration (database/store.rs migrate_legacy_progress); printed on the console by main.rs and the migrate-progress command, never sent to the browser',
    values: [
      '之前已经迁移过',
      '没有找到旧的 progress.json',
      '旧的 progress.json 读不了：{e}',
      '数据库里已经有学习记录，没有覆盖',
      '旧的 progress.json 里有格式不对的内容，没有迁移：{e}',
      '旧的 progress.json 里有格式不对的内容，没有迁移：数据不满足表的约束',
      '迁移了 {} 场成绩、{} 道题的答题记录',
      '数据库里没有管理员账户，没有迁移',
    ],
  },
];

/** Messages added in v0.5 (v05-spec.md §4.4); translated even before the backend source that sends them exists. */
const SPEC_MESSAGES = [
  '这个题库不存在（可能已被删除），请刷新页面后再试。',
  '这条考试记录不存在（可能已被删除），请刷新页面后再试。',
  '上传目标不对，请刷新页面后再试。',
  '请求的格式不对。',
  '设置的值不对。',
];

/** Excel markers that an English text may quote verbatim (the user types them into the workbook). */
const EXCEL_MARKERS = ['【答案】', 'はい / いいえ'];

const CJK = /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}]/u;
const KANA = /[\p{Script=Hiragana}\p{Script=Katakana}]/u;
/** Characters that exist only in Simplified Chinese (Japanese writes 这→這, 题→題, 请→請 ...): a Japanese text must not contain them. */
const SIMPLIFIED_ONLY = new Set([...
  '这个们题请页试对错项时为设删读选记录务该让现关开还过进动网库览预处闭显发级层损坏压实许问说输导间绝码证验应换长认识两带态经别确节构杂单无运种档储链吗么']);

// ------------------------------------------------------------------ a small Rust lexer

interface Token {
  kind: 'ident' | 'punct' | 'str' | 'char' | 'lifetime' | 'num';
  text: string;
  /** Decoded value of a string literal */
  value: string;
  /** b"…" / c"…" literals (never messages) */
  bytes: boolean;
  line: number;
}

export interface Literal {
  file: string;
  line: number;
  value: string;
  /** First argument of format! / #[error(…)]: `{}` / `{name}` are replaced by values, `{{` `}}` are braces */
  format: boolean;
}

const ID_START = /[A-Za-z_]/;
const ID_CHAR = /[A-Za-z0-9_]/;

function tokenize(src: string): Token[] {
  const toks: Token[] = [];
  const n = src.length;
  let i = 0;
  let line = 1;
  const at = (k: number): string => src[k] ?? '';
  const push = (kind: Token['kind'], text: string, value = '', bytes = false, startLine = line): void => {
    toks.push({ kind, text, value, bytes, line: startLine });
  };
  const countLines = (s: string): number => s.split('\n').length - 1;

  // Reads a quoted literal whose opening quote is at `q`; returns the decoded value and the index after the closing quote.
  const readQuoted = (q: number): { value: string; end: number } => {
    let j = q + 1;
    let v = '';
    while (j < n && at(j) !== '"') {
      if (at(j) !== '\\') {
        v += at(j);
        j += 1;
        continue;
      }
      const e = at(j + 1);
      if (e === 'n') v += '\n';
      else if (e === 'r') v += '\r';
      else if (e === 't') v += '\t';
      else if (e === '0') v += '\0';
      else if (e === 'x') {
        v += String.fromCharCode(parseInt(src.slice(j + 2, j + 4), 16));
        j += 4;
        continue;
      } else if (e === 'u') {
        const close = src.indexOf('}', j);
        v += String.fromCodePoint(parseInt(src.slice(j + 3, close).replace(/_/g, ''), 16));
        j = close + 1;
        continue;
      } else if (e === '\n' || e === '\r') {
        // line continuation: the newline and the leading whitespace of the next line are dropped
        j += 1;
        while (j < n && /\s/.test(at(j))) j += 1;
        continue;
      } else v += e;
      j += 2;
    }
    return { value: v, end: j + 1 };
  };

  while (i < n) {
    const c = at(i);
    if (c === '\n') {
      line += 1;
      i += 1;
      continue;
    }
    if (/\s/.test(c)) {
      i += 1;
      continue;
    }
    if (c === '/' && at(i + 1) === '/') {
      while (i < n && at(i) !== '\n') i += 1;
      continue;
    }
    if (c === '/' && at(i + 1) === '*') {
      let depth = 1;
      let j = i + 2;
      while (j < n && depth > 0) {
        if (at(j) === '/' && at(j + 1) === '*') {
          depth += 1;
          j += 2;
        } else if (at(j) === '*' && at(j + 1) === '/') {
          depth -= 1;
          j += 2;
        } else j += 1;
      }
      line += countLines(src.slice(i, j));
      i = j;
      continue;
    }
    if (c === 'r' || c === 'b' || c === 'c') {
      const bytes = c !== 'r';
      const j = bytes ? i + 1 : i;
      if (at(j) === 'r') {
        let k = j + 1;
        let hashes = 0;
        while (at(k) === '#') {
          hashes += 1;
          k += 1;
        }
        if (at(k) === '"') {
          const close = src.indexOf(`"${'#'.repeat(hashes)}`, k + 1);
          const value = src.slice(k + 1, close);
          const startLine = line;
          line += countLines(value);
          push('str', src.slice(i, close + 1 + hashes), value, bytes, startLine);
          i = close + 1 + hashes;
          continue;
        }
      } else if (bytes && at(j) === '"') {
        const startLine = line;
        const { value, end } = readQuoted(j);
        line += countLines(src.slice(i, end));
        push('str', src.slice(i, end), value, true, startLine);
        i = end;
        continue;
      }
    }
    if (c === '"') {
      const startLine = line;
      const { value, end } = readQuoted(i);
      line += countLines(src.slice(i, end));
      push('str', src.slice(i, end), value, false, startLine);
      i = end;
      continue;
    }
    if (c === "'") {
      if (at(i + 1) === '\\') {
        const e = at(i + 2);
        let j = e === 'u' ? src.indexOf('}', i) + 1 : e === 'x' ? i + 5 : i + 3;
        if (at(j) !== "'") j = src.indexOf("'", i + 2);
        push('char', src.slice(i, j + 1));
        i = j + 1;
        continue;
      }
      const cp = src.codePointAt(i + 1) ?? 0;
      const len = cp > 0xffff ? 2 : 1;
      if (at(i + 1 + len) === "'") {
        push('char', src.slice(i, i + 2 + len));
        i += 2 + len;
        continue;
      }
      let j = i + 1;
      while (j < n && ID_CHAR.test(at(j))) j += 1;
      push('lifetime', src.slice(i, j));
      i = j;
      continue;
    }
    if (ID_START.test(c)) {
      let j = i + 1;
      while (j < n && ID_CHAR.test(at(j))) j += 1;
      push('ident', src.slice(i, j));
      i = j;
      continue;
    }
    if (/[0-9]/.test(c)) {
      let j = i + 1;
      while (j < n && ID_CHAR.test(at(j))) j += 1;
      push('num', src.slice(i, j));
      i = j;
      continue;
    }
    const cp = src.codePointAt(i) ?? 0;
    const len = cp > 0xffff ? 2 : 1;
    push('punct', src.slice(i, i + len));
    i += len;
  }
  return toks;
}

const OPEN: Record<string, string> = { '(': ')', '[': ']', '{': '}' };

/** `toks[k]` is an opening bracket; returns the index after its matching closing bracket. */
function skipBalanced(toks: Token[], k: number): number {
  let depth = 0;
  for (let j = k; j < toks.length; j += 1) {
    const t = toks[j];
    if (!t || t.kind !== 'punct') continue;
    if (t.text in OPEN) depth += 1;
    else if (t.text === ')' || t.text === ']' || t.text === '}') {
      depth -= 1;
      if (depth === 0) return j + 1;
    }
  }
  return toks.length;
}

/** Skips the item after a `#[cfg(test)]` attribute (`mod tests { … }`, `fn …(…) { … }`, `use …;`) and returns the index after it. */
function skipItem(toks: Token[], k: number): number {
  let j = k;
  while (toks[j]?.text === '#' && toks[j + 1]?.text === '[') j = skipBalanced(toks, j + 1);
  let depth = 0;
  for (; j < toks.length; j += 1) {
    const t = toks[j];
    if (!t || t.kind !== 'punct') continue;
    if (t.text === '(' || t.text === '[') depth += 1;
    else if (t.text === ')' || t.text === ']') depth -= 1;
    else if (t.text === ';' && depth === 0) return j + 1;
    else if (t.text === '{' && depth === 0) return skipBalanced(toks, j);
  }
  return toks.length;
}

/** Whether the tokens just before index k are exactly `seq` */
function preceded(toks: Token[], k: number, seq: readonly string[]): boolean {
  return seq.every((s, idx) => toks[k - seq.length + idx]?.text === s);
}

const FORMAT_MACROS = ['format', 'format_args', 'panic', 'unreachable', 'todo', 'unimplemented'];
/** Macros whose text never reaches the browser: assertions, log lines and console output */
const SKIPPED_MACROS = /^(?:debug_)?assert(?:_eq|_ne)?$|^(?:trace|debug|info|warn|error)$|^e?print(?:ln)?$/;

function isRegex(toks: Token[], k: number): boolean {
  return (
    preceded(toks, k, ['re', '(']) ||
    preceded(toks, k, ['re', '(', '&', 'format', '!', '(']) ||
    preceded(toks, k, ['Regex', ':', ':', 'new', '(']) ||
    preceded(toks, k, ['Regex', ':', ':', 'new', '(', '&', 'format', '!', '('])
  );
}

function isFormat(toks: Token[], k: number): boolean {
  if (preceded(toks, k, ['#', '[', 'error', '('])) return true;
  return FORMAT_MACROS.some((m) => preceded(toks, k, [m, '!', '(']));
}

export function extractLiterals(src: string, file: string): Literal[] {
  const toks = tokenize(src);
  const out: Literal[] = [];
  let k = 0;
  while (k < toks.length) {
    const t = toks[k];
    if (!t) break;
    if (t.text === '#' && ['[', 'cfg', '(', 'test', ')', ']'].every((s, idx) => toks[k + 1 + idx]?.text === s)) {
      k = skipItem(toks, k + 7);
      continue;
    }
    if (t.kind === 'ident' && SKIPPED_MACROS.test(t.text) && toks[k + 1]?.text === '!' && toks[k + 2]?.text !== '=') {
      k = skipBalanced(toks, k + 2);
      continue;
    }
    if (t.kind === 'str' && !t.bytes && !isRegex(toks, k)) {
      out.push({ file, line: t.line, value: t.value, format: isFormat(toks, k) });
    }
    k += 1;
  }
  return out;
}

/** Replaces the `{}` / `{name}` parts of a format string with sample values (样本1, 样本2, …) */
function fillFormat(template: string): { text: string; samples: string[] } {
  const samples: string[] = [];
  const text = template.replace(/\{\{|\}\}|\{[^{}]*\}/g, (m) => {
    if (m === '{{') return '{';
    if (m === '}}') return '}';
    const sample = `样本${samples.length + 1}`;
    samples.push(sample);
    return sample;
  });
  return { text, samples };
}

/** Format strings with every placeholder written as `{}`, to compare templates regardless of argument names */
function normalizeTemplate(template: string): string {
  return template.replace(/\{[^{}]*\}/g, '{}');
}

function rustFiles(dir: string, rel = ''): string[] {
  const out: string[] = [];
  for (const name of readdirSync(dir).sort()) {
    const full = join(dir, name);
    const relName = rel ? `${rel}/${name}` : name;
    if (statSync(full).isDirectory()) {
      if (name !== 'tests') out.push(...rustFiles(full, relName));
    } else if (name.endsWith('.rs') && name !== 'tests.rs' && !(relName in FILE_SKIPS)) {
      out.push(relName);
    }
  }
  return out;
}

const SKIPPED = new Set(NOT_MESSAGES.flatMap((g) => g.values));

interface BackendMessage {
  literal: Literal;
  text: string;
  samples: string[];
}

function backendMessages(): BackendMessage[] {
  const seen = new Set<string>();
  const out: BackendMessage[] = [];
  for (const file of rustFiles(BACKEND_SRC)) {
    const src = readFileSync(join(BACKEND_SRC, file), 'utf8');
    for (const lit of extractLiterals(src, file)) {
      if (!CJK.test(lit.value) || SKIPPED.has(lit.value) || seen.has(lit.value)) continue;
      seen.add(lit.value);
      const { text, samples } = lit.format ? fillFormat(lit.value) : { text: lit.value, samples: [] };
      out.push({ literal: lit, text, samples });
    }
  }
  return out;
}

/** The translation with the sample values and the quoted Excel markers taken out */
function residue(out: string, samples: readonly string[]): string {
  let r = out;
  for (const s of [...samples, ...EXCEL_MARKERS]) r = r.split(s).join(' ');
  return r;
}

/** Problems of one translation; an empty list means it is fine */
function problems(source: string, samples: readonly string[], lang: 'en' | 'ja'): string[] {
  const out = localizeServerMessage(source, lang);
  const found: string[] = [];
  if (out === source) return ['not translated'];
  for (const s of samples) if (!out.includes(s)) found.push(`value ${s} is missing in "${out}"`);
  const rest = residue(out, samples);
  if (lang === 'en' && CJK.test(rest)) found.push(`Chinese / Japanese text left in "${out}"`);
  if (lang === 'ja') {
    if (!KANA.test(rest)) found.push(`no kana, so probably not Japanese: "${out}"`);
    const simplified = [...rest].filter((ch) => SIMPLIFIED_ONLY.has(ch));
    if (simplified.length) found.push(`Simplified Chinese characters ${simplified.join('')} in "${out}"`);
  }
  return found;
}

// ------------------------------------------------------------------ tests

test('the lexer skips comments, test code, assertions, logs and regexes, and decodes escapes', () => {
  const src = String.raw`
// "注释里的字符串"
/// doc "文档"
/* block "块注释" /* nested */ still "注释" */
pub const A: &str = "普通\"引号\"\n第二行";
const B: &str = r#"原始 "字符串""#;
const L: &'static str = "生命周期之后";
const C: char = '【';
const D: char = '\'';
fn f(x: &str) -> String {
    assert!(x.len() > 0, "断言里的");
    tracing::warn!("日志里的 {x}");
    println!("控制台");
    let r = re(r"【([^】]+)】");
    format!("第 {x} 项，{{括号}}")
}
#[error("属性 {0}")]
struct E;
#[cfg(test)]
#[allow(dead_code)]
fn helper(a: [u8; 2]) -> &'static str { "测试函数" }
#[cfg(test)]
mod tests {
    #[test]
    fn t() { let s = "测试模块"; }
}
const Z: &str = "续行\
                 合并";
const Y: &[u8] = b"bytes";
`;
  const lits = extractLiterals(src, 'x.rs');
  assert.deepEqual(
    lits.map((l) => [l.value, l.format]),
    [
      ['普通"引号"\n第二行', false],
      ['原始 "字符串"', false],
      ['生命周期之后', false],
      ['第 {x} 项，{{括号}}', true],
      ['属性 {0}', true],
      ['续行合并', false],
    ],
  );
  assert.equal(lits[0]?.line, 5);
  assert.deepEqual(fillFormat('第 {x} 项，{{括号}} {} {0:>3}'), { text: '第 样本1 项，{括号} 样本2 样本3', samples: ['样本1', '样本2', '样本3'] });
});

test('the backend message files exist and the lexer finds their messages', () => {
  for (const f of REQUIRED_FILES) assert.ok(existsSync(join(BACKEND_SRC, f)), `backend/src/${f} not found`);
  const msgs = backendMessages();
  assert.ok(msgs.length >= 30, `only ${msgs.length} messages found; is the lexer broken?`);
  const files = new Set(msgs.map((m) => m.literal.file));
  for (const f of ['error.rs', 'api/response.rs', 'api/guard.rs', 'api/attempt.rs', 'services/mod.rs', 'parser/workbook.rs', 'parser/question.rs']) {
    assert.ok(files.has(f), `no message found in backend/src/${f}`);
  }
});

test('every Chinese backend message is translated to English and Japanese', () => {
  const failures: string[] = [];
  for (const m of backendMessages()) {
    for (const lang of ['en', 'ja'] as const) {
      for (const p of problems(m.text, m.samples, lang)) {
        failures.push(`${m.literal.file}:${m.literal.line} [${lang}] ${JSON.stringify(m.literal.value)}: ${p}`);
      }
    }
    assert.equal(localizeServerMessage(m.text, 'zh'), m.text, 'the Chinese UI shows the backend text unchanged');
  }
  assert.deepEqual(failures, [], `Add the translation to src/i18n/server-messages.ts (or, if the text is not shown to users, to NOT_MESSAGES in this test with the reason):\n${failures.join('\n')}`);
});

test('the v0.5 messages of the spec are translated', () => {
  for (const msg of SPEC_MESSAGES) {
    for (const lang of ['en', 'ja'] as const) assert.deepEqual(problems(msg, [], lang), [], msg);
    assert.equal(localizeServerMessage(msg, 'zh'), msg);
  }
});

test('no stale translations: every known text still exists in the backend (or in the v0.5 spec)', () => {
  const msgs = backendMessages();
  const plain = new Set(msgs.map((m) => m.literal.value));
  const templates = new Set(msgs.filter((m) => m.literal.format).map((m) => normalizeTemplate(m.literal.value)));
  const known = knownServerMessages();
  const stale = [
    ...known.exact.filter((s) => !plain.has(s) && !SPEC_MESSAGES.includes(s)),
    ...known.templates.filter((s) => !templates.has(normalizeTemplate(s))),
  ];
  assert.deepEqual(stale, [], 'these texts are no longer sent by the backend: update or remove them in server-messages.ts');
});

test('concrete translations', () => {
  const tooLarge = '文件太大了（最大 20 MB）。题库通常只有 1–2 MB，请确认选对了文件。';
  assert.equal(localizeServerMessage(tooLarge, 'en'), 'The file is too large (20 MB at most). A question bank is usually only 1–2 MB, so check that you chose the right file.');
  assert.equal(localizeServerMessage(tooLarge, 'ja'), 'ファイルが大きすぎます（最大 20 MB）。問題集は通常 1〜2 MB ほどです。正しいファイルを選んだか確認してください。');
  assert.equal(localizeServerMessage(tooLarge, 'zh'), tooLarge);
  // surrounding whitespace does not matter
  assert.equal(localizeServerMessage(`  ${tooLarge}\n`, 'en'), localizeServerMessage(tooLarge, 'en'));

  const origin = '被拒绝：这个请求来自「exam.example.com」，和服务看到的 Host「127.0.0.1」不是同一个主机。如果你在 Cloudflare Tunnel 等反向代理后面且代理改写了 Host，请把「exam.example.com」加进 XEXAM_ALLOWED_ORIGINS。';
  assert.equal(
    localizeServerMessage(origin, 'en'),
    'Denied: this request comes from “exam.example.com”, which is not the same host as the Host the service sees (“127.0.0.1”). If Xexam runs behind a reverse proxy such as Cloudflare Tunnel that rewrites the Host, add “exam.example.com” to XEXAM_ALLOWED_ORIGINS.',
  );
  const emptyHost = '被拒绝：这个请求的 Host 是「」，不在允许的列表里。请把它加进 XEXAM_ALLOWED_HOSTS（或设 XEXAM_ALLOW_ANY_HOST=1，只在前面有身份验证时）。';
  assert.match(localizeServerMessage(emptyHost, 'ja'), /^拒否されました：このリクエストの Host「」は許可リスト/);

  const forbidden = 'Forbidden host: lan.example。通过别的地址访问（局域网 IP、你的域名）时，请把它加进环境变量 XEXAM_ALLOWED_HOSTS，或设 XEXAM_ALLOW_ANY_HOST=1（只在前面有身份验证时）。';
  assert.match(localizeServerMessage(forbidden, 'en'), /^Forbidden host: lan\.example\. To open Xexam through another address/);
  assert.match(localizeServerMessage(forbidden, 'ja'), /^Forbidden host: lan\.example。別のアドレス/);

  // skip reasons: values from the workbook are kept; lists use the language's separator; English singular / plural
  assert.equal(localizeServerMessage('答案 E、F 不在选项 A、B、C、D 里', 'en'), 'The answer letters E, F are not among the options A, B, C, D');
  assert.equal(localizeServerMessage('答案 E 不在选项 A、B、C、D 里', 'en'), 'The answer letter E is not among the options A, B, C, D');
  assert.equal(localizeServerMessage('答案 E、F 不在选项 A、B、C、D 里', 'ja'), '解答の E、F は選択肢 A、B、C、D にありません');
  assert.equal(localizeServerMessage('答案「见解析」里没有找到选项字母', 'en'), 'No option letter was found in the answer “见解析”');
  assert.equal(localizeServerMessage('第 2 小项没有答案', 'ja'), '小問 2 の解答がありません');
  assert.equal(
    localizeServerMessage('第 3 小项的答案「是」不在选项「はい / いいえ」里', 'en'),
    'The answer “是” to sub-item 3 is not one of its choices “はい / いいえ”',
  );
  assert.equal(localizeServerMessage('答案里有第 4、5 项，但题目只有 3 个小项', 'en'), 'The answer has entries for sub-items 4, 5, but the question has only 3 sub-items');
  assert.equal(localizeServerMessage('答案里有第 2 项，但题目只有 1 个小项', 'en'), 'The answer has an entry for sub-item 2, but the question has only 1 sub-item');
  assert.equal(localizeServerMessage('答案里有第 4、5 项，但题目只有 3 个小项', 'ja'), '解答に小問 4、5 の答えがありますが、この問題の小問は 3 個だけです');
  assert.equal(
    localizeServerMessage('这个文件缺少 .xlsx 必需的内容（"There is no item named \'xl/workbook.xml\' in the archive"），可能已损坏。', 'en'),
    'This file is missing content that every .xlsx file needs ("There is no item named \'xl/workbook.xml\' in the archive"). It may be damaged.',
  );
});

test('protocol errors get a sentence in every language; unknown text is returned unchanged', () => {
  assert.equal(localizeServerMessage('bad json', 'en'), 'The request is not valid JSON.');
  assert.equal(localizeServerMessage('bad json', 'zh'), '请求不是有效的 JSON。');
  assert.equal(localizeServerMessage('not found', 'ja').includes('サービス'), true);
  for (const s of knownServerMessages().protocol) {
    for (const lang of ['en', 'zh', 'ja'] as const) assert.notEqual(localizeServerMessage(s, lang), s, `${s} [${lang}]`);
  }
  for (const unknown of ['某个以后才会出现的新消息', 'Something the backend has never said', '']) {
    for (const lang of ['en', 'zh', 'ja'] as const) assert.equal(localizeServerMessage(unknown, lang), unknown);
  }
  // a known message inside a longer text is not a match (patterns are anchored)
  const longer = '前缀：请求的格式不对。';
  assert.equal(localizeServerMessage(longer, 'en'), longer);
});
