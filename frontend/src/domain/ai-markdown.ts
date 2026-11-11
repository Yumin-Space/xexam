// The small Markdown subset AI Chat shows in answers (v0.8), parsed into a tree of blocks the page builds with text nodes
// only (never innerHTML, so nothing in an answer is ever interpreted as HTML: "<script>" stays text). Pure.
//   blocks: paragraphs (single line breaks kept), "- " / "* " / "+ " lists, "1. " lists, fenced ``` code blocks (no
//           highlighting; an unclosed fence, as while an answer is still being written, runs to the end), headings
//           "#" to "######" (shown as a bold paragraph), and "---" rules;
//   inline: **bold**, *emphasis*, `code`; a link [text](address) becomes the plain text "text (address)".

export type Inline =
  | { t: 'text'; v: string }
  | { t: 'b'; c: Inline[] }
  | { t: 'em'; c: Inline[] }
  | { t: 'code'; v: string };

export type Block =
  | { t: 'p'; c: Inline[] }
  | { t: 'h'; c: Inline[] }
  | { t: 'ul'; items: Inline[][] }
  | { t: 'ol'; start: number; items: Inline[][] }
  | { t: 'pre'; v: string }
  | { t: 'hr' };

const FENCE = /^ {0,3}(`{3,}|~{3,})/;
const BULLET = /^\s{0,8}[-*+]\s+(.*)$/;
const NUMBERED = /^\s{0,8}(\d{1,9})[.)]\s+(.*)$/;
const HEADING = /^ {0,3}(#{1,6})\s+(.*?)\s*#*\s*$/;
const RULE = /^ {0,3}([-*_])(\s*\1){2,}\s*$/;

/** Inline markup of one piece of text */
export function parseInline(src: string): Inline[] {
  const out: Inline[] = [];
  let text = '';
  const flush = (): void => {
    if (text) out.push({ t: 'text', v: text });
    text = '';
  };
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (c === '`') {
      // A code span: the same number of backticks closes it; without a closing run the backticks are text
      let n = 1;
      while (src[i + n] === '`') n++;
      const fence = '`'.repeat(n);
      const end = src.indexOf(fence, i + n);
      if (end > i) {
        flush();
        let v = src.slice(i + n, end);
        if (v.length > 2 && v.startsWith(' ') && v.endsWith(' ')) v = v.slice(1, -1);
        out.push({ t: 'code', v });
        i = end + n;
        continue;
      }
      text += fence;
      i += n;
      continue;
    }
    if (c === '*' && src[i + 1] === '*') {
      const end = src.indexOf('**', i + 2);
      if (end > i + 2 && src[i + 2] !== ' ') {
        flush();
        out.push({ t: 'b', c: parseInline(src.slice(i + 2, end)) });
        i = end + 2;
        continue;
      }
    } else if (c === '*' || c === '_') {
      // *emphasis*: not "a * b" (a lone star between spaces), not snake_case words
      const prev = i > 0 ? src[i - 1] ?? '' : '';
      const next = src[i + 1] ?? '';
      const end = src.indexOf(c, i + 1);
      const closing = end > i + 1 ? src[end - 1] ?? '' : '';
      const after = end > 0 ? src[end + 1] ?? '' : '';
      const wordy = (ch: string): boolean => /[A-Za-z0-9]/.test(ch);
      if (end > i + 1 && next !== ' ' && closing !== ' ' && !(c === '_' && (wordy(prev) || wordy(after))) && src[end + 1] !== c) {
        flush();
        out.push({ t: 'em', c: parseInline(src.slice(i + 1, end)) });
        i = end + 1;
        continue;
      }
    } else if (c === '[') {
      const m = /^\[([^\]\n]+)\]\(([^)\s]+)\)/.exec(src.slice(i));
      if (m) {
        text += `${m[1] ?? ''} (${m[2] ?? ''})`;
        i += m[0].length;
        continue;
      }
    }
    text += c;
    i++;
  }
  flush();
  return out;
}

/** The blocks of an answer (complete or still being written) */
export function parseMarkdown(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, '\n').split('\n');
  const blocks: Block[] = [];
  let para: string[] = [];
  let list: { t: 'ul' | 'ol'; start: number; items: string[] } | null = null;

  const endPara = (): void => {
    if (para.length) blocks.push({ t: 'p', c: parseInline(para.join('\n')) });
    para = [];
  };
  const endList = (): void => {
    if (!list) return;
    const items = list.items.map((it) => parseInline(it));
    blocks.push(list.t === 'ul' ? { t: 'ul', items } : { t: 'ol', start: list.start, items });
    list = null;
  };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i] ?? '';
    const fence = FENCE.exec(line);
    if (fence) {
      endPara();
      endList();
      const mark = fence[1] ?? '```';
      const code: string[] = [];
      let j = i + 1;
      while (j < lines.length && !(lines[j] ?? '').trimStart().startsWith(mark.slice(0, 3))) {
        code.push(lines[j] ?? '');
        j++;
      }
      blocks.push({ t: 'pre', v: code.join('\n') });
      i = j; // the closing fence (or past the end)
      continue;
    }
    if (!line.trim()) {
      endPara();
      // A blank line inside a list ends it only when the next line is not another item of it
      const next = lines[i + 1] ?? '';
      const cur = list as { t: 'ul' | 'ol' } | null;
      if (cur && !((cur.t === 'ul' && BULLET.test(next)) || (cur.t === 'ol' && NUMBERED.test(next)))) endList();
      continue;
    }
    const heading = HEADING.exec(line);
    if (heading) {
      endPara();
      endList();
      blocks.push({ t: 'h', c: parseInline(heading[2] ?? '') });
      continue;
    }
    if (RULE.test(line) && !para.length) {
      endList();
      blocks.push({ t: 'hr' });
      continue;
    }
    const bullet = BULLET.exec(line);
    const numbered = bullet ? null : NUMBERED.exec(line);
    if (bullet || numbered) {
      endPara();
      const kind = bullet ? 'ul' : 'ol';
      const text = (bullet ? bullet[1] : numbered?.[2]) ?? '';
      if (!list || list.t !== kind) {
        endList();
        list = { t: kind, start: numbered ? Number(numbered[1]) || 1 : 1, items: [] };
      }
      list.items.push(text);
      continue;
    }
    if (list) {
      // A continued line of the last item (indented, or a lazy continuation)
      const items = list.items;
      if (/^\s/.test(line) && items.length) {
        items[items.length - 1] += '\n' + line.trim();
        continue;
      }
      endList();
    }
    para.push(line);
  }
  endPara();
  endList();
  return blocks;
}

/** The plain text of inline content (for an accessible name or a test) */
export function inlineText(c: readonly Inline[]): string {
  return c.map((x) => (x.t === 'text' || x.t === 'code' ? x.v : inlineText(x.c))).join('');
}
