# -*- coding: utf-8 -*-
"""Regenerate vectors.json (question-id test vectors) with the REAL app/xlsx_parser.py.

usage: python -I -B gen_vectors.py [repo_dir] [out_json]
Part A: direct make_id(sheet, stem, first) vectors.
Part B: pipeline vectors: rows (A,B,C cells) in one or more sheets -> parse_workbook() -> ids
        (covers duplicate suffix -2/-3, reordered rows, same stem in two sheets).
Each vector also stores the exact bytes that are hashed (hex) so a Rust port can be debugged byte by byte.
"""
import hashlib
import io
import json
import re
import sys
import zipfile
from xml.sax.saxutils import escape

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import paths  # noqa: E402
REPO = sys.argv[1] if len(sys.argv) > 1 else paths.LEGACY_ROOT
OUT = sys.argv[2] if len(sys.argv) > 2 else os.path.join(paths.GOLDEN, 'id_vectors.json')
sys.dont_write_bytecode = True
sys.path.insert(0, REPO)
from app import xlsx_parser as P  # noqa: E402

A_STEM = '東京は日本の首都です。日本の首都はどこですか？'
A_OPT = '大阪'

direct_cases = [
    # name, sheet, stem, first, note
    ('basic_ja', 'demo', A_STEM, A_OPT, 'plain Japanese'),
    ('sheet_changes_id', 'demo2', A_STEM, A_OPT, 'same stem, other sheet name -> other id'),
    ('sheet_case_matters', 'Demo', A_STEM, A_OPT, 'sheet name is case-sensitive'),
    ('sheet_trailing_space_kept', 'demo ', A_STEM, A_OPT, 'sheet name is NOT stripped'),
    ('sheet_empty', '', A_STEM, A_OPT, 'empty sheet name'),
    ('sheet_nfd_vs_nfc_a', 'caf\u00e9', 'q', 'a', 'sheet NFC e-acute'),
    ('sheet_nfd_vs_nfc_b', 'cafe\u0301', 'q', 'a', 'sheet NFD e + U+0301: different id from NFC (no normalisation)'),
    ('sheet_jp', 'スコープ１', A_STEM, A_OPT, 'Japanese sheet name with full-width digit'),
    ('sheet_newline_inside', 'a\nb', 'q', 'a', 'newline inside sheet name is kept'),
    ('stem_trailing_space', 'demo', A_STEM + '   ', A_OPT, 'trailing ASCII spaces ignored (== basic_ja)'),
    ('stem_trailing_newline', 'demo', A_STEM + '\n\n', A_OPT, 'trailing newlines ignored (== basic_ja)'),
    ('stem_crlf_inside', 'demo', '東京は日本の首都です。\r\n日本の首都はどこですか？', A_OPT, 'CRLF removed as whitespace'),
    ('stem_lf_inside', 'demo', '東京は日本の首都です。\n日本の首都はどこですか？', A_OPT, 'LF removed (== crlf case)'),
    ('stem_tab_inside', 'demo', '東京は\t日本の首都です。日本の首都はどこですか？', A_OPT, 'tab removed'),
    ('stem_ideographic_space', 'demo', '東京は\u3000日本の首都です。日本の首都はどこですか？', A_OPT, 'U+3000 removed (== tab case)'),
    ('stem_nbsp', 'demo', '東京は\u00a0日本の首都です。日本の首都はどこですか？', A_OPT, 'U+00A0 NBSP removed by \\s'),
    ('stem_u2003_emspace', 'demo', '東京は\u2003日本の首都です。日本の首都はどこですか？', A_OPT, 'U+2003 removed'),
    ('stem_u0085_nel', 'demo', '東京は\u0085日本の首都です。日本の首都はどこですか？', A_OPT, 'U+0085 NEL removed by Python \\s (XML parser accepts it raw)'),
    ('stem_u001c', 'demo', '東京は\x1c日本の首都です。日本の首都はどこですか？', A_OPT, 'U+001C removed by Python \\s but NOT by Rust regex \\s / char::is_whitespace; unreachable through XLSX (XML forbids it)'),
    ('stem_u2028', 'demo', '東京は\u2028日本の首都です。日本の首都はどこですか？', A_OPT, 'U+2028 LINE SEPARATOR removed'),
    ('stem_zwsp_kept', 'demo', '東京は\u200b日本の首都です。日本の首都はどこですか？', A_OPT, 'U+200B ZERO WIDTH SPACE is NOT whitespace for Python: kept -> different id'),
    ('stem_bom_kept', 'demo', '\ufeff' + A_STEM, A_OPT, 'U+FEFF kept -> different id'),
    ('stem_fullwidth_qmark_vs_ascii', 'demo', '東京は日本の首都です。日本の首都はどこですか?', A_OPT, 'ASCII ? instead of U+FF1F: different id (no NFKC)'),
    ('stem_fullwidth_digits', 'demo', '１ + １ は いくつですか？', 'Ａ', 'full-width digits and Latin kept as is'),
    ('stem_halfwidth_digits', 'demo', '1 + 1 は いくつですか?', 'A', 'half-width counterpart: different id'),
    ('stem_halfwidth_kana', 'demo', 'ﾄｳｷｮｳは日本の首都です', '大阪', 'half-width katakana kept (no NFKC)'),
    ('stem_fullwidth_kana', 'demo', 'トウキョウは日本の首都です', '大阪', 'full-width counterpart: different id'),
    ('stem_dakuten_precomposed', 'demo', 'がぎぐげご', 'x', 'precomposed hiragana (NFC)'),
    ('stem_dakuten_combining', 'demo', 'か\u3099き\u3099く\u3099け\u3099こ\u3099', 'x', 'base + U+3099 combining (NFD): different id from precomposed'),
    ('stem_latin_case', 'demo', 'What is AWS?', 'Amazon', 'case is preserved (no lower/casefold)'),
    ('stem_latin_case_lower', 'demo', 'what is aws?', 'amazon', 'lowercase: different id'),
    ('stem_sharp_s', 'demo', 'Stra\u00dfe', 'x', 'U+00DF: str.lower()/casefold() would matter if used; here untouched'),
    ('stem_turkish_dotted_i', 'demo', '\u0130stanbul', 'x', 'U+0130 untouched'),
    ('stem_nonbmp_cjk', 'demo', '\U00020BB7野家で食べる', '牛丼', 'non-BMP CJK (surrogate pair in UTF-16, 4 bytes in UTF-8)'),
    ('stem_emoji', 'demo', 'Which is a fruit? \U0001F34E', 'x', 'emoji'),
    ('first_empty', 'demo', A_STEM, '', 'empty first option text'),
    ('first_whitespace_only', 'demo', A_STEM, ' \t\u3000\n', 'whitespace-only first -> same as empty'),
    ('first_inner_whitespace', 'demo', A_STEM, 'り ん\tご', 'inner whitespace removed'),
    ('stem_empty', 'demo', '', A_OPT, 'empty stem'),
    ('both_empty', 'demo', '', '', 'empty stem and first'),
    ('stem_only_ws', 'demo', ' \n\t', '', 'whitespace-only stem'),
    ('stem_pipe_char', 'demo', 'a|b', 'c', 'literal | in stem: "a|b|c" collides with stem "a" first "b|c"'),
    ('stem_pipe_collision', 'demo', 'a', 'b|c', 'collides with stem_pipe_char (same raw string a|b|c) -> same id'),
    ('stem_newline_sheet_collision_a', 'x', 'y|z', 'w', 'sheet "x" stem "y|z"'),
    ('stem_newline_sheet_collision_b', 'x\ny', 'z', 'w', 'NOT a collision here: sheet "x\\ny" stem "z" (separator is \\n then stem; whitespace removed only inside stem)'),
]

# truncation: stem[:300] and first[:200] in CODE POINTS after whitespace removal
long_ja = '日本語' * 120                       # 360 code points
direct_cases += [
    ('stem_len_300_exact', 'demo', 'あ' * 300, 'x', 'exactly 300 chars'),
    ('stem_len_301_same_as_300', 'demo', 'あ' * 301, 'x', '301st char ignored: same id as exactly-300'),
    ('stem_len_360_ja', 'demo', long_ja, 'x', 'truncation by code points (not bytes)'),
    ('stem_len_360_ja_tail_changed', 'demo', long_ja[:300] + '違う', 'x', 'change after char 300 does not change the id'),
    ('stem_trunc_with_ws_removed', 'demo', ('あ ' * 400), 'x', 'whitespace removed BEFORE truncating (800 chars in, 300 hashed)'),
    ('stem_trunc_nonbmp', 'demo', '\U00020BB7' * 305, 'x', '305 non-BMP chars: truncation counts code points, UTF-16 slicing would differ'),
    ('first_len_200_exact', 'demo', 'q', 'い' * 200, 'exactly 200 chars'),
    ('first_len_201_same_as_200', 'demo', 'q', 'い' * 201, '201st char ignored'),
    ('first_trunc_nonbmp', 'demo', 'q', '\U0001F34E' * 205, '205 emoji: 200 code points hashed'),
]


def hash_input(sheet, stem, first):
    return sheet + '\n' + re.sub(r'\s+', '', stem)[:300] + '|' + re.sub(r'\s+', '', first)[:200]


vectors = []
for name, sheet, stem, first, note in direct_cases:
    got = P.make_id(sheet, stem, first)
    raw = hash_input(sheet, stem, first)
    assert got == hashlib.sha1(raw.encode('utf-8')).hexdigest()[:12], name   # independent re-implementation agrees
    vectors.append({'kind': 'make_id', 'name': name, 'sheet': sheet, 'stem': stem, 'first': first,
                    'hash_input_utf8_hex': raw.encode('utf-8').hex(), 'id': got, 'note': note})


# ---------------------------------------------------------------- Part B: pipeline vectors
def cell(ref, t):
    return '<c r="%s" t="inlineStr"><is><t xml:space="preserve">%s</t></is></c>' % (ref, escape(t))


def make_xlsx(sheets):
    """sheets: [(sheet_name, [(A,B,C), ...])] -> bytes of a minimal xlsx (inline strings only)."""
    files = {
        '[Content_Types].xml': '<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/></Types>',
        '_rels/.rels': '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>',
    }
    wb_sheets, rels = [], []
    for i, (name, rows) in enumerate(sheets, 1):
        wb_sheets.append('<sheet name="%s" sheetId="%d" r:id="rId%d"/>' % (escape(name, {'"': '&quot;'}), i, i))
        rels.append('<Relationship Id="rId%d" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet%d.xml"/>' % (i, i))
        body = ''.join('<row r="%d">%s</row>' % (rn, ''.join(cell('%s%d' % (c, rn), t) for c, t in zip('ABC', r) if t != ''))
                       for rn, r in enumerate(rows, 1))
        files['xl/worksheets/sheet%d.xml' % i] = '<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>%s</sheetData></worksheet>' % body
    files['xl/workbook.xml'] = '<?xml version="1.0" encoding="UTF-8"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>%s</sheets></workbook>' % ''.join(wb_sheets)
    files['xl/_rels/workbook.xml.rels'] = '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">%s</Relationships>' % ''.join(rels)
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, 'w', zipfile.ZIP_DEFLATED) as z:
        for n, d in files.items():
            z.writestr(n, d)
    return buf.getvalue()


def single(stem, opts, ans='B', note=''):
    a = stem + '\n\n' + '\n'.join('%s. %s' % (chr(65 + i), o) for i, o in enumerate(opts))
    return (a, '', '【答案】' + ans + '\n\n【解析·怎么选】' + (note or 'x'))


def hotspot(stem, items, ans):
    a = 'ホットスポット\n' + stem + '\n\n' + '\n\n'.join('%d. %s　はい / いいえ' % (i, t) for i, t in enumerate(items, 1))
    return (a, '', '【答案】' + ans)


Q1 = single('次のうち、首都はどれですか？', ['大阪', '東京', '京都'])
Q2 = single('信号が赤のとき、どうしますか？', ['進む', '止まる', '走る'])
Q3 = single('１ + １ は いくつですか？', ['１', '２', '３'], 'B')
Q4 = single('Which is a fruit?', ['りんご', 'にんじん'], 'A')
Q5 = single('日本の四季は？', ['春夏秋冬', '雨季乾季'], 'A')
H1 = hotspot('次の文について選択してください。', ['太陽は東から昇る。', '月は恒星である。'], '1 はい；2 いいえ')
# same stem/first as Q1 but different answer and explanation (duplicate fingerprint, different content)
Q1b = single('次のうち、首都はどれですか？', ['大阪', '東京', '京都'], 'C', 'second copy')
Q1c = single('次のうち、首都はどれですか？', ['大阪', '京都'], 'A', 'third copy')

pipeline_cases = [
    ('p_single_sheet', [('S', [Q1, Q2, H1])], 'three question kinds in order'),
    ('p_reordered_rows', [('S', [H1, Q2, Q1])], 'same questions, rows reordered: ids must be identical to p_single_sheet'),
    ('p_same_stem_two_sheets', [('S1', [Q1]), ('S2', [Q1])], 'same question in two sheets: different ids (sheet is hashed)'),
    ('p_dup_in_sheet', [('S', [Q1, Q1b, Q1c, Q2])], 'identical fingerprint 3x: ids base, base-2, base-3'),
    ('p_dup_reordered', [('S', [Q1c, Q1, Q1b, Q2])], 'same three copies in other order: suffix follows row order, so the id of each CONTENT changes'),
    ('p_dup_across_sheets_no_suffix', [('S1', [Q1, Q1b]), ('S2', [Q1, Q1b])], 'duplicates are counted globally over the workbook ("seen" is shared) but sheet differs so only in-sheet dups get suffix'),
    ('p_dup_removed_first', [('S', [Q1b, Q2])], 'if the first copy is deleted from Excel the second copy inherits the unsuffixed id'),
    ('p_fullwidth_digits', [('S', [Q3, Q4])], 'full-width digits/latin in stem'),
    ('p_unicode_ws_in_cells', [('S', [single('東京は\u3000日本の\t首都\r\nです。', ['大阪', '東京'], 'B')])], 'U+3000 / tab / CRLF inside the stem cell'),
    ('p_trailing_ws_in_option', [('S', [single('末尾の空白', ['大阪   ', '東京'], 'B')])], 'trailing spaces after option A text'),
    ('p_nfd_stem', [('S', [single('か\u3099き\u3099く\u3099は何ですか', ['あ', 'い'], 'A')])], 'NFD stem stays NFD in the id input'),
    ('p_two_sheets_mixed', [('スコープ１', [Q1, Q2]), ('スコープ２', [Q5, H1])], 'Japanese sheet names'),
]
for name, sheets, note in pipeline_cases:
    data = make_xlsx(sheets)
    s, qs = P.parse_workbook(io.BytesIO(data))
    vectors.append({'kind': 'pipeline', 'name': name, 'note': note,
                    'sheets_in': [{'name': n, 'rows': [list(r) for r in rows]} for n, rows in sheets],
                    'sheets_out': s,
                    'ids': [{'sheet': q['sheet'], 'row': q['row'], 'kind': q['kind'], 'stem': q['stem'],
                             'first': (q['options'][0]['text'] if q['options'] else q['items'][0]['text']),
                             'id': q['id']} for q in qs]})

n_direct = sum(1 for v in vectors if v['kind'] == 'make_id')
n_pipe_ids = sum(len(v['ids']) for v in vectors if v['kind'] == 'pipeline')
doc = {
    'generator': 'gen_vectors.py', 'algorithm': 'sha1( sheet + "\\n" + re.sub(r"\\s+","",stem)[:300] + "|" + re.sub(r"\\s+","",first)[:200] ).hexdigest()[:12]; '
                 'then in parse_workbook: while id in seen: id = base-N (N=2,3,...)',
    'python_whitespace_regex_class_\\s': ['%04X' % cp for cp in range(0x110000) if re.match(r'\s', chr(cp))],
    'counts': {'make_id_vectors': n_direct, 'pipeline_cases': len(pipeline_cases), 'pipeline_ids': n_pipe_ids, 'total_id_assertions': n_direct + n_pipe_ids},
    'vectors': vectors,
}
with open(OUT, 'w', encoding='utf-8', newline='\n') as f:
    json.dump(doc, f, ensure_ascii=False, indent=1)
print('wrote', OUT, doc['counts'])
