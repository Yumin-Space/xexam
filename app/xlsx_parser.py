# -*- coding: utf-8 -*-
"""读取 Excel 题库并解析题目（只用标准库）。

每个工作表里：A 列是题目，B 列是中文翻译，C 列是答案/解析/记忆口诀。
这里的逻辑和之前的 server.py 完全一致，只是从服务端里拆了出来，方便单独测试。
"""
import hashlib
import posixpath
import re
import zipfile
import xml.etree.ElementTree as ET

NS = '{http://schemas.openxmlformats.org/spreadsheetml/2006/main}'
RNS = '{http://schemas.openxmlformats.org/officeDocument/2006/relationships}'
PKG = '{http://schemas.openxmlformats.org/package/2006/relationships}'

# ---------------------------------------------------------------- 读取 xlsx

def _si_text(si):
    """共享字符串/内联字符串 -> 纯文本。忽略 <rPh>（注音假名），只取正文。"""
    parts = []
    for child in si:
        if child.tag == NS + 't':
            parts.append(child.text or '')
        elif child.tag == NS + 'r':
            for t in child:
                if t.tag == NS + 't':
                    parts.append(t.text or '')
    return ''.join(parts)


def _col_index(ref):
    letters = re.match(r'[A-Za-z]+', ref or '')
    if not letters:
        return None
    n = 0
    for ch in letters.group(0).upper():
        n = n * 26 + ord(ch) - 64
    return n - 1


def _part_path(target):
    if target.startswith('/'):
        return target.lstrip('/')
    return posixpath.normpath(posixpath.join('xl', target))


def _read_sheet(z, member, shared):
    root = ET.fromstring(z.read(member))
    rows = []
    for row in root.iter(NS + 'row'):
        try:
            rn = int(row.get('r'))
        except (TypeError, ValueError):
            continue
        cells = {}
        for c in row.findall(NS + 'c'):
            col = _col_index(c.get('r'))
            if col is None:
                continue
            t = c.get('t')
            if t == 'inlineStr':
                node = c.find(NS + 'is')
                val = _si_text(node) if node is not None else ''
            else:
                v = c.find(NS + 'v')
                if v is None or v.text is None:
                    continue
                if t == 's':
                    try:
                        val = shared[int(v.text)]
                    except (ValueError, IndexError):
                        continue
                elif t == 'b':
                    val = 'TRUE' if v.text == '1' else 'FALSE'
                else:
                    val = v.text
            if val.strip():
                cells[col] = val
        if cells:
            rows.append((rn, cells))
    return rows


def read_workbook(path):
    """-> [(工作表名, [(行号, {列号: 文本})])]，隐藏的工作表会跳过。"""
    with zipfile.ZipFile(path) as z:
        names = set(z.namelist())
        shared = []
        if 'xl/sharedStrings.xml' in names:
            sst = ET.fromstring(z.read('xl/sharedStrings.xml'))
            shared = [_si_text(si) for si in sst.findall(NS + 'si')]
        wb = ET.fromstring(z.read('xl/workbook.xml'))
        rels = ET.fromstring(z.read('xl/_rels/workbook.xml.rels'))
        target = {r.get('Id'): r.get('Target') for r in rels.findall(PKG + 'Relationship')}
        out = []
        sheets = wb.find(NS + 'sheets')
        for s in (sheets.findall(NS + 'sheet') if sheets is not None else []):
            if s.get('state') in ('hidden', 'veryHidden'):
                continue
            tgt = target.get(s.get(RNS + 'id'))
            if not tgt:
                continue
            member = _part_path(tgt)
            if member not in names:
                continue
            out.append((s.get('name') or '', _read_sheet(z, member, shared)))
        return out


# ---------------------------------------------------------------- 解析题目

_YES = {'是', 'yes', 'y', 'true', 'はい', '○', '〇', '对', '對', '正确', '正確'}
_NO = {'否', 'no', 'n', 'false', 'いいえ', '×', '✕', '错', '錯', '错误', '不正確'}


def canon(s):
    t = re.sub(r'[\s_\-–—‐・.．、,，/／]+', '', str(s).strip().lower())
    if t in _YES:
        return '\x00yes'
    if t in _NO:
        return '\x00no'
    return t


def classify_heading(name):
    if '答案' in name or '正解' in name:
        return 'answer'
    if '解析' in name or '解説' in name:
        return 'explanation'
    if '口诀' in name or '口訣' in name or '记忆' in name or '記憶' in name:
        return 'mnemonic'
    if '翻译' in name or '翻訳' in name or '译文' in name:
        return 'translation'
    return None


def split_sections(text):
    """按【答案】【解析·怎么选】【记忆口诀】【中文翻译】切段。"""
    marks = []
    for m in re.finditer(r'【([^】\n]{1,24})】', text):
        kind = classify_heading(m.group(1))
        if kind:
            marks.append((m.start(), m.end(), kind))
    secs = {}
    for i, (_, e, kind) in enumerate(marks):
        end = marks[i + 1][0] if i + 1 < len(marks) else len(text)
        secs.setdefault(kind, text[e:end].strip())
    return secs


OPT_LINE = re.compile(r'^\s*([A-Ha-h])(?:\s*[.．、)）:：]\s*(.*)|\s*)$')
NUM_PREFIX = re.compile(r'^\s*(\d{1,2})\s*[.．、)）]\s*')
HINT_RE = re.compile(r'[\t\u3000 ]+([^\t\u3000/]+?(?: +/ +[^\t\u3000/]+?)+)[ \t\u3000]*$')


def parse_options(body):
    """选择题：支持「A. 内容」和「A 换行 内容」两种写法。-> (题干, [选项])"""
    lines = body.split('\n')
    start = None
    for i, ln in enumerate(lines):
        m = OPT_LINE.match(ln)
        if m and m.group(1).upper() == 'A':
            start = i
            break
    if start is None:
        return body.strip(), []
    opts, cur, expected = [], None, 'A'
    for ln in lines[start:]:
        m = OPT_LINE.match(ln)
        if m and m.group(1).upper() == expected:
            cur = {'key': expected, 'lines': [(m.group(2) or '').strip()]}
            opts.append(cur)
            expected = chr(ord(expected) + 1)
        elif cur is not None:
            cur['lines'].append(ln.strip())
    options = [{'key': o['key'], 'text': '\n'.join(x for x in o['lines'] if x).strip()} for o in opts]
    if len(options) < 2 or any(not o['text'] for o in options):
        return body.strip(), []
    return '\n'.join(lines[:start]).strip(), options


def parse_hotspot(body):
    """热点题：每个小项末尾带「はい / いいえ」之类的选项。-> (题干, [小项])"""
    stem_lines, items, pending, common = [], [], [], None
    for ln in body.split('\n'):
        if not ln.strip():
            if not items:
                stem_lines.append(ln)
            continue
        text = NUM_PREFIX.sub('', ln, count=1)
        had_num = text != ln
        m = HINT_RE.search(text)
        if m:
            choices = [t.strip() for t in re.split(r'\s+/\s+', m.group(1)) if t.strip()]
            if len(choices) >= 2:
                common = common or choices
                head = text[:m.start()].strip()
                items.append({'text': '\n'.join(pending + [head]).strip(), 'choices': choices})
                pending = []
                continue
        if items and had_num and common:
            items.append({'text': '\n'.join(pending + [text.strip()]).strip(), 'choices': list(common)})
            pending = []
        elif items:
            pending.append(ln.strip())
        else:
            stem_lines.append(ln)
    return '\n'.join(stem_lines).strip(), items


def parse_letters(ans):
    s = re.sub(r'[（(][^）)]*[）)]', ' ', ans)
    first = next((ln for ln in s.split('\n') if ln.strip()), '')
    letters = []
    for m in re.finditer(r'(?<![A-Za-z0-9])([A-Ha-h])(?![A-Za-z0-9])', first):
        k = m.group(1).upper()
        if k not in letters:
            letters.append(k)
    if not letters and re.fullmatch(r'[A-Ha-h]{2,6}', first.strip()):
        letters = sorted(set(first.strip().upper()))
    return sorted(letters)


def parse_item_answers(ans):
    s = re.sub(r'[（(][^）)]*[）)]', '', ans)
    out = {}
    for seg in re.split(r'[；;\n，,]+', s):
        m = re.match(r'^\s*(\d+)\s*[.．、:：)）\-]*\s*(.+?)\s*$', seg)
        if m:
            out[int(m.group(1))] = m.group(2)
    if len(out) < 2:
        out = {int(n): v for n, v in re.findall(r'(\d+)\s*[.．、:：)）]?\s*([^\s\d;；,，]+)', s)}
    return out


def find_choice(choices, val):
    cv = canon(val)
    for j, ch in enumerate(choices):
        if canon(ch) == cv:
            return j
    return None


def make_id(sheet, stem, first):
    raw = sheet + '\n' + re.sub(r'\s+', '', stem)[:300] + '|' + re.sub(r'\s+', '', first)[:200]
    return hashlib.sha1(raw.encode('utf-8')).hexdigest()[:12]


def _norm_nl(s):
    return (s or '').replace('\r\n', '\n').replace('\r', '\n').strip()


_HOT_LEAD = re.compile(r'^\s*(?:热点题|熱點題|热点|hotspot)\s*[:：]\s*', re.I)


def parse_translation(tr, options, items):
    """把 B 列的中文翻译拆成和日语题目一样的结构（题干 / 选项 / 小项），用来在答题时切换语言。
    拆出来的选项字母或小项数量和日语题目对不上时返回 None（这道题就不提供中文切换，不会显示错位的内容）。"""
    tr = _HOT_LEAD.sub('', _norm_nl(tr), count=1).strip()
    if not tr:
        return None
    if options:
        stem, opts = parse_options(tr)
        if [o['key'] for o in opts] != [o['key'] for o in options]:
            return None
        return {'stem': stem, 'options': [o['text'] for o in opts]}
    if items:
        stem_lines, parts, cur = [], [], None
        for ln in tr.split('\n'):
            m = NUM_PREFIX.match(ln)
            if m and int(m.group(1)) == len(parts) + 1:
                cur = [ln[m.end():].strip()]
                parts.append(cur)
            elif cur is not None:
                if ln.strip():
                    cur.append(ln.strip())
            else:
                stem_lines.append(ln)
        if len(parts) != len(items):
            return None
        return {'stem': '\n'.join(stem_lines).strip(), 'items': ['\n'.join(p).strip() for p in parts]}
    return None


def parse_question(sheet, row, a, b, c):
    """-> (题目dict, None) 或 (None, 读不出来的原因)"""
    text, b, c = _norm_nl(a), _norm_nl(b), _norm_nl(c)
    combined = (b + '\n\n' + c).strip()
    secs = split_sections(combined)

    first_line = next((ln.strip() for ln in text.split('\n') if ln.strip()), '')
    is_hot = first_line.lower() in ('ホットスポット', 'hotspot')
    body = text.split('\n', 1)[1] if is_hot and '\n' in text else ('' if is_hot else text)

    stem, options, items = '', [], []
    if not is_hot:
        stem, options = parse_options(body)
    if not options:
        stem, items = parse_hotspot(body)
    if not options and not items:
        return None, '没有找到选项（A、B、C…）或带「はい / いいえ」的小项'

    ans = secs.get('answer')
    if ans is None:
        m = re.search(r'(?:^|\n)[ \t]*(?:答案|正解|正确答案|Answer|Ans)[ \t]*[:：=][ \t]*(.+)', combined, re.I)
        if m:
            ans = m.group(1).strip()
    if ans is None:
        for cand in (c, b):
            if cand and '\n' not in cand and len(cand) <= 40 and not cand.startswith('【'):
                ans = cand
                break
    if not ans:
        return None, '没有找到答案（C 列要写「【答案】…」）'

    if options:
        letters = parse_letters(ans)
        keys = [o['key'] for o in options]
        if not letters:
            return None, '答案「%s」里没有找到选项字母' % ans[:20]
        bad = [x for x in letters if x not in keys]
        if bad:
            return None, '答案 %s 不在选项 %s 里' % ('、'.join(bad), '、'.join(keys))
        kind, answer = ('multi' if len(letters) > 1 else 'single'), letters
        first_text = options[0]['text']
    else:
        vals = parse_item_answers(ans)
        answer = []
        for k, it in enumerate(items, 1):
            if k not in vals:
                return None, '第 %d 小项没有答案' % k
            j = find_choice(it['choices'], vals[k])
            if j is None:
                return None, '第 %d 小项的答案「%s」不在选项「%s」里' % (k, vals[k], ' / '.join(it['choices']))
            answer.append(j)
        extra = sorted(set(vals) - set(range(1, len(items) + 1)))
        if extra:
            return None, '答案里有第 %s 项，但题目只有 %d 个小项' % ('、'.join(map(str, extra)), len(items))
        kind = 'hotspot'
        first_text = items[0]['text']

    translation = secs.get('translation')
    if translation is None and b and '【' not in b and len(b) > 40:
        translation = b
    note = '；'.join(x.strip() for x in re.findall(r'[（(]([^）)]*)[）)]', ans) if x.strip())
    zh = parse_translation(translation, options, items) if translation else None

    return {
        'id': make_id(sheet, stem, first_text),
        'sheet': sheet, 'row': row, 'kind': kind,
        'tag': 'ホットスポット' if is_hot else '',
        'stem': stem, 'options': options, 'items': items, 'answer': answer,
        'translation': translation or '', 'zh': zh,
        'explanation': secs.get('explanation', ''),
        'mnemonic': secs.get('mnemonic', ''),
        'note': note,
    }, None



# ---------------------------------------------------------------- 整份工作簿 -> 题目列表

HEADER_WORDS = ('题目', '題目', '问题', '問題', 'Question')


def parse_workbook(source):
    """source 是 .xlsx 的路径或文件对象。返回 (sheets, questions)。

    sheets: [{name, total, usable, skipped: [{row, reason}]}]
    questions: 解析好的题目 dict 列表（每题带 id / sheet / row / kind / stem / options / items / answer ...）
    读不了文件时抛出 ValueError（带中文说明）。
    """
    try:
        raw = read_workbook(source)
    except zipfile.BadZipFile:
        raise ValueError('这个文件不是有效的 .xlsx（可能是 .xls、.csv，或者文件已损坏）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。')
    except KeyError as e:
        raise ValueError('这个文件缺少 .xlsx 必需的内容（%s），可能已损坏。' % e)
    except ET.ParseError as e:
        raise ValueError('文件里的 XML 读不了：%s' % e)

    sheets, questions, seen = [], [], set()
    for name, rows in raw:
        info = {'name': name, 'total': 0, 'usable': 0, 'skipped': []}
        for rn, cells in rows:
            a = cells.get(0)
            if not a or not a.strip():
                continue
            b, c = cells.get(1, ''), cells.get(2, '')
            if len(a.strip()) < 15 and not b.strip() and not c.strip():
                continue                      # 备注之类的短文字
            if len(a.strip()) <= 8 and (classify_heading(a) in ('answer', 'explanation', 'mnemonic', 'translation')
                                        or a.strip() in HEADER_WORDS):
                continue                      # 标题行（如「题目 / 翻译 / 答案」）
            info['total'] += 1
            q, why = parse_question(name, rn, a, b, c)
            if q is None:
                info['skipped'].append({'row': rn, 'reason': why})
                continue
            base, n = q['id'], 2
            while q['id'] in seen:
                q['id'] = '%s-%d' % (base, n)
                n += 1
            seen.add(q['id'])
            questions.append(q)
            info['usable'] += 1
        if info['total'] or not rows:
            sheets.append(info)
    return sheets, questions
