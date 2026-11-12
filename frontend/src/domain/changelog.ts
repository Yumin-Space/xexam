// The changelog shown on the About page comes from the "### vX.Y — date" entries of the three changelogs,
// docs/en/changelog.md, docs/ja/changelog.md and docs/zh-CN/changelog.md (one source of truth: the files CI checks against
// CODE_VERSION in VERSION). The Vite plugin in vite.config.ts parses them at build time with parseChangelog(); the page
// renders the lines with a tiny Markdown subset (lists, bold, inline code, links), text only.

export interface ChangelogEntry {
  /** "0.5" */
  version: string;
  /** The rest of the heading, usually the date: "2026-11-10" */
  date: string;
  /** The Markdown lines of the entry (without the heading) */
  lines: string[];
}

const ENTRY = /^###\s+v(\d+(?:\.\d+)*)\s*(?:[—–-]\s*)?(.*)$/;

/** All entries from the first "### vX" heading to the next "## " heading (or the end); whole-line HTML comments (such as the footer "write new versions above this line") are skipped. */
export function parseChangelog(markdown: string): ChangelogEntry[] {
  const out: ChangelogEntry[] = [];
  let cur: ChangelogEntry | null = null;
  for (const raw of markdown.replace(/\r\n?/g, '\n').split('\n')) {
    const m = ENTRY.exec(raw);
    if (m) {
      cur = { version: m[1] ?? '', date: (m[2] ?? '').trim(), lines: [] };
      out.push(cur);
      continue;
    }
    if (!cur) continue;
    if (/^##\s/.test(raw)) break; // the next level-2 section ends the changelog
    if (/^\s*<!--.*-->\s*$/.test(raw)) continue; // a comment for maintainers, not part of the entry
    cur.lines.push(raw);
  }
  for (const e of out) {
    while (e.lines.length && !e.lines[0]?.trim()) e.lines.shift();
    while (e.lines.length && !e.lines[e.lines.length - 1]?.trim()) e.lines.pop();
  }
  return out;
}

export type Inline =
  | { kind: 'text'; text: string }
  | { kind: 'bold'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'link'; text: string; href: string };

/** Splits one line into text, **bold**, `code` and [link](href) pieces (no nesting). */
export function inlineTokens(line: string): Inline[] {
  const out: Inline[] = [];
  const re = /\*\*([^*]+)\*\*|`([^`]+)`|\[([^\]]+)\]\(([^)\s]+)\)/g;
  let last = 0;
  for (let m = re.exec(line); m; m = re.exec(line)) {
    if (m.index > last) out.push({ kind: 'text', text: line.slice(last, m.index) });
    if (m[1] != null) out.push({ kind: 'bold', text: m[1] });
    else if (m[2] != null) out.push({ kind: 'code', text: m[2] });
    else out.push({ kind: 'link', text: m[3] ?? '', href: m[4] ?? '' });
    last = m.index + m[0].length;
  }
  if (last < line.length) out.push({ kind: 'text', text: line.slice(last) });
  return out;
}

export type Block =
  | { kind: 'para'; text: string }
  | { kind: 'list'; items: Array<{ text: string; children: string[] }> };

/** Groups the lines of an entry into paragraphs and (two-level) bullet lists; continuation lines join the item above. */
export function blocks(lines: readonly string[]): Block[] {
  const out: Block[] = [];
  let list: Extract<Block, { kind: 'list' }> | null = null;
  let para: Extract<Block, { kind: 'para' }> | null = null;
  for (const raw of lines) {
    const line = raw.replace(/\s+$/, '');
    if (!line.trim()) {
      list = null;
      para = null;
      continue;
    }
    const bullet = /^(\s*)[-*]\s+(.*)$/.exec(line);
    if (bullet) {
      para = null;
      const nested = (bullet[1] ?? '').length >= 2;
      if (!list) {
        list = { kind: 'list', items: [] };
        out.push(list);
      }
      const last = list.items[list.items.length - 1];
      if (nested && last) last.children.push(bullet[2] ?? '');
      else list.items.push({ text: bullet[2] ?? '', children: [] });
      continue;
    }
    if (list && /^\s+/.test(line)) {
      // continuation of the previous bullet
      const last = list.items[list.items.length - 1];
      if (last) {
        if (last.children.length) last.children[last.children.length - 1] += ' ' + line.trim();
        else last.text += ' ' + line.trim();
      }
      continue;
    }
    list = null;
    if (para) para.text += ' ' + line.trim();
    else {
      para = { kind: 'para', text: line.trim() };
      out.push(para);
    }
  }
  return out;
}
