# -*- coding: utf-8 -*-
"""Generate synthetic edge-case .xlsx fixtures (zipfile + hand-written XML, all content fictional) for the parser parity audit.

usage:  python -B -I make_fixtures.py [out_dir]      (default out_dir = <this folder>/fixtures)
Nothing outside out_dir is touched.  Python 3 stdlib only.
"""
import os
import sys
import zipfile
from xml.sax.saxutils import escape

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(HERE, 'fixtures')
os.makedirs(OUT, exist_ok=True)

MAIN = 'http://schemas.openxmlformats.org/spreadsheetml/2006/main'
RNS = 'http://schemas.openxmlformats.org/officeDocument/2006/relationships'
PKG = 'http://schemas.openxmlformats.org/package/2006/relationships'
XMLDECL = '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'


# ------------------------------------------------------------ cell / row / sheet helpers
def inl(ref, text, raw=False):
    t = text if raw else escape(text)
    return '<c r="%s" t="inlineStr"><is><t xml:space="preserve">%s</t></is></c>' % (ref, t)


def row_xml(n, cells, attrs=''):
    return '<row r="%s"%s>%s</row>' % (n, attrs, ''.join(cells))


def qrow(n, a=None, b=None, c=None, cols='ABC'):
    cells = []
    for col, v in zip(cols, (a, b, c)):
        if v is not None:
            cells.append(inl('%s%d' % (col, n), v))
    return row_xml(n, cells)


def sheet_xml(rows, extra_before='', extra_after='', root_attrs=''):
    return (XMLDECL + '<worksheet xmlns="%s" xmlns:r="%s"%s>%s<sheetData>%s</sheetData>%s</worksheet>'
            % (MAIN, RNS, root_attrs, extra_before, ''.join(rows), extra_after))


def build(name, sheets, shared=None, wb_sheets=None, rels=None, extra_files=None, drop=(), wb_raw=None):
    """sheets: list of dict(name, part, xml, rid=None, state=None). Written in the given order into the zip.
    wb_sheets: override list of (name, rid, state) in workbook.xml order. rels: override dict rid -> target."""
    files = {}
    files['[Content_Types].xml'] = (XMLDECL + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
                                    '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
                                    '<Default Extension="xml" ContentType="application/xml"/></Types>')
    files['_rels/.rels'] = (XMLDECL + '<Relationships xmlns="%s"><Relationship Id="rId1" Type="%s/officeDocument" Target="xl/workbook.xml"/></Relationships>' % (PKG, RNS))
    rel_map, wbs = {}, []
    for i, s in enumerate(sheets, 1):
        rid = s.get('rid') or 'rId%d' % i
        rel_map[rid] = s.get('target') or s['part'][len('xl/'):] if s['part'].startswith('xl/') else s['part']
        files[s['part']] = s['xml']
        wbs.append((s['name'], rid, s.get('state')))
    if rels is not None:
        rel_map = rels
    if wb_sheets is not None:
        wbs = wb_sheets
    sh = ''
    for i, (n, rid, state) in enumerate(wbs, 1):
        sh += '<sheet name="%s" sheetId="%d"%s r:id="%s"/>' % (escape(n, {'"': '&quot;'}), i, (' state="%s"' % state) if state else '', rid)
    files['xl/workbook.xml'] = wb_raw or (XMLDECL + '<workbook xmlns="%s" xmlns:r="%s"><sheets>%s</sheets></workbook>' % (MAIN, RNS, sh))
    files['xl/_rels/workbook.xml.rels'] = (XMLDECL + '<Relationships xmlns="%s">%s</Relationships>' % (
        PKG, ''.join('<Relationship Id="%s" Type="%s/worksheet" Target="%s"/>' % (k, RNS, v) for k, v in rel_map.items())))
    if shared is not None:
        files['xl/sharedStrings.xml'] = shared
    for k, v in (extra_files or {}).items():
        files[k] = v
    for k in drop:
        files.pop(k, None)
    path = os.path.join(OUT, name)
    with zipfile.ZipFile(path, 'w', zipfile.ZIP_DEFLATED) as z:
        for n, d in files.items():
            z.writestr(n, d if isinstance(d, bytes) else d.encode('utf-8'))
    return path


def one(name, rows, sheet='Sheet1', **kw):
    return build(name, [dict(name=sheet, part='xl/worksheets/sheet1.xml', xml=sheet_xml(rows))], **kw)


# ------------------------------------------------------------ fictional content
SC = ("次のうち、首都はどれですか。\n\nA. 大阪\nB. 東京\nC. 京都\nD. 名古屋",
      "【中文翻译】\n下面哪个是首都？\nA. 大阪\nB. 东京\nC. 京都\nD. 名古屋",
      "【答案】B\n\n【解析·怎么选】东京是首都。\n\n【记忆口诀】首都看题干。")
MC = ("果物を2つ選んでください。\n\nA. りんご\nB. にんじん\nC. バナナ\nD. たまねぎ",
      "【中文翻译】\n选出两个水果。\nA. 苹果\nB. 胡萝卜\nC. 香蕉\nD. 洋葱",
      "【答案】A、C\n\n【解析·怎么选】水果。")
HS = ("ホットスポット\n次の文は正しいですか。\n\n1. 太陽は東から昇る。　はい / いいえ\n\n2. 月は恒星である。　はい / いいえ",
      "【中文翻译】\n热点题：下面的句子对吗？\n1. 太阳从东边升起。\n2. 月亮是恒星。",
      "【答案】1 是；2 否\n\n【解析·怎么选】月亮是卫星。")

# ------------------------------------------------------------ fixtures
# f01 inline strings only, basic types
one('f01_inline_basic.xlsx', [qrow(1, '题目', '翻译', '答案'), qrow(2, *SC), qrow(3, *MC), qrow(4, *HS)], sheet='basic')

# f02 shared strings: plain, rich text runs, phonetic runs (rPh + phoneticPr), plus a <t> directly followed by rPh
sst = (XMLDECL + '<sst xmlns="%s" count="9" uniqueCount="9">' % MAIN +
       '<si><t>次のうち、東京の読みはどれですか。\n\nA. とうきょう\nB. おおさか\nC. きょうと</t></si>' +                                       # 0 plain (newline literal)
       '<si><r><rPr><b/></rPr><t xml:space="preserve">【答案】</t></r><r><t>A</t></r></si>' +                                              # 1 rich text runs
       '<si><t>漢字</t><rPh sb="0" eb="2"><t>かんじ</t></rPh><phoneticPr fontId="1"/></si>' +                                          # 2 phonetic run after <t>
       '<si><r><t>次の</t></r><rPh sb="0" eb="1"><t>ふりがな</t></rPh><r><t>文を選べ。\n\nA. 甲\nB. 乙</t></r></si>' +                    # 3 phonetic run BETWEEN rich runs
       '<si><t>【答案】B</t></si>' +                                                                                                   # 4
       '<si></si>' +                                                                                                                    # 5 empty si
       '<si><r><t/></r></si>' +                                                                                                         # 6 empty run
       '<si><t>2</t></si>' +                                                                                                            # 7
       '<si><t>中文翻译不会被使用</t></si>' +                                                                                          # 8
       '</sst>')
rows = [
    row_xml(1, ['<c r="A1" t="s"><v>0</v></c>', '<c r="B1" t="s"><v>8</v></c>', '<c r="C1" t="s"><v>1</v></c>']),
    row_xml(2, ['<c r="A2" t="s"><v>3</v></c>', '<c r="C2" t="s"><v>4</v></c>']),
    row_xml(3, ['<c r="A3" t="s"><v>2</v></c>', '<c r="C3" t="s"><v>5</v></c>']),          # phonetic after <t>, empty si in C
    row_xml(4, ['<c r="A4" t="s"><v>99</v></c>', '<c r="B4" t="s"><v>x</v></c>', '<c r="C4" t="s"><v>6</v></c>']),   # bad shared index / non-int / empty run
]
one('f02_sharedstrings_rich_phonetic.xlsx', rows, sheet='sst', shared=sst)

# f03 worksheet part names are NOT sheetN.xml; relative/absolute/../ targets; target to a missing member; unknown rId
build('f03_sheet_part_names.xlsx', [
    dict(name='byname', part='xl/worksheets/data_a.xml', xml=sheet_xml([qrow(1, *SC)])),
    dict(name='nested', part='xl/worksheets/foo/bar.xml', xml=sheet_xml([qrow(1, *MC)])),
    dict(name='absolute', part='xl/worksheets/abs.xml', xml=sheet_xml([qrow(1, *HS)]), target='/xl/worksheets/abs.xml'),
    dict(name='dotdot', part='xl/worksheets/rel.xml', xml=sheet_xml([qrow(1, *SC)]), target='../xl/worksheets/rel.xml'),
    dict(name='missing-part', part='xl/worksheets/ghost.xml', xml=sheet_xml([qrow(1, *SC)])),
], wb_sheets=[('byname', 'rId1', None), ('nested', 'rId2', None), ('absolute', 'rId3', None), ('dotdot', 'rId4', None),
              ('missing-part', 'rId5', None), ('no-such-rid', 'rId77', None)], drop=('xl/worksheets/ghost.xml',))

# f04 workbook.xml order differs from file names (sheet1.xml holds "Beta", sheet2.xml holds "Alpha"; workbook lists Alpha first; rIds crossed)
build('f04_reversed_sheet_order.xlsx', [
    dict(name='Beta', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, *SC)]), rid='rId3'),
    dict(name='Alpha', part='xl/worksheets/sheet2.xml', xml=sheet_xml([qrow(1, *MC)]), rid='rId9'),
], wb_sheets=[('Alpha', 'rId9', None), ('Beta', 'rId3', None)])

# f05 missing cells / unusual references / row quirks
rows = [
    row_xml(1, [inl('A1', SC[0]), inl('C1', SC[2])]),                                   # B missing in the middle
    row_xml(2, [inl('A2', MC[0]), inl('B2', MC[1]), inl('C2', MC[2])] + [inl('E2', 'ignored column E')]),   # extra column ignored
    row_xml(3, [inl('c3', MC[2].replace('A、C', 'B、D')), inl('a3', MC[0].replace('果物', '野菜'))]),    # lowercase refs, C before A in document order
    row_xml(4, ['<c t="inlineStr"><is><t>%s</t></is></c>' % escape(SC[0]), inl('C4', SC[2])]),            # cell without r attribute -> skipped
    '<row>%s</row>' % (inl('A5', SC[0]) + inl('C5', SC[2])),                              # row without r -> skipped
    '<row r="abc">%s</row>' % (inl('A6', SC[0]) + inl('C6', SC[2])),                      # non-numeric r -> skipped
    row_xml(10, [inl('A10', '果物を選べ（順序入替）。\n\nA. 甲\nB. 乙'), inl('C10', '【答案】A')]),    # rows out of order: 10 before 8
    row_xml(8, [inl('A8', '色を選べ。\n\nA. 赤\nB. 青'), inl('C8', '【答案】B')]),
    row_xml(11, [inl('A11', '重複セルのテスト。\n\nA. 甲\nB. 乙'), inl('A11', '二番目の A11 が勝つ。\n\nA. 丙\nB. 丁'), inl('C11', '【答案】B')]),   # duplicate ref
    row_xml(12, ['<c r="A12" t="inlineStr"><is><t>%s</t></is></c>' % escape(SC[0].replace('首都', '都市')), '<c r="B12"/>', '<c r="C12"><v/></c>']),   # empty cells
    row_xml(13, [inl('AA13', 'column 27'), inl('C13', 'x')]),
    row_xml(14, []),                                                                    # empty row
    row_xml(15, ['<c r="A15" t="inlineStr"><is><t>   </t></is></c>', inl('C15', '【答案】A')]),   # whitespace-only A
]
one('f05_missing_cells.xlsx', rows, sheet='cells')

# f06 leading empty column (data in B,C,D) -> column A empty for every row
one('f06_leading_empty_column.xlsx', [qrow(1, SC[0], SC[1], SC[2], cols='BCD'), qrow(2, MC[0], MC[1], MC[2], cols='BCD')], sheet='shifted')

# f07 numbers / booleans / formulas / dates / errors / t=str in column A and elsewhere
rows = [
    row_xml(1, ['<c r="A1"><v>12345</v></c>', inl('B1', 'x'), inl('C1', 'y')]),                                 # number with B,C -> counted, no options
    row_xml(2, ['<c r="A2"><v>12345</v></c>']),                                                                  # number only, <15 chars -> remark
    row_xml(3, ['<c r="A3"><v>1234567890123456</v></c>']),                                                       # 16 digits alone -> counted (>=15)
    row_xml(4, ['<c r="A4" t="b"><v>1</v></c>', inl('B4', 'x')]),                                                 # TRUE
    row_xml(5, ['<c r="A5" t="b"><v>0</v></c>', inl('B5', 'x')]),                                                 # FALSE
    row_xml(6, ['<c r="A6" t="str"><f>CONCAT("a","b")</f><v>%s</v></c>' % escape(SC[0]), inl('C6', SC[2])]),     # formula string result
    row_xml(7, ['<c r="A7"><f>1+1</f><v>2</v></c>', inl('B7', 'x')]),                                              # formula numeric
    row_xml(8, ['<c r="A8"><f>1+1</f></c>', inl('B8', 'x')]),                                                      # formula without cached value -> dropped
    row_xml(9, ['<c r="A9" t="e"><v>#N/A</v></c>', inl('B9', 'x')]),                                              # error cell
    row_xml(10, ['<c r="A10" t="d"><v>2024-01-31T00:00:00</v></c>', inl('B10', 'x')]),                             # ISO date
    row_xml(11, ['<c r="A11" s="3"><v>45322</v></c>', inl('B11', 'x')]),                                           # date serial as number
    row_xml(12, ['<c r="A12"><v>1E+3</v></c>', inl('B12', 'x')]),
    row_xml(13, ['<c r="A13" t="inlineStr"><v>text in v</v></c>', inl('B13', 'x')]),                               # inlineStr but <v> instead of <is>
    row_xml(14, [inl('A14', SC[0]), '<c r="B14"><v>%s</v></c>' % escape('【中文翻译】\n下面哪个是首都？\nA. 大阪\nB. 东京\nC. 京都\nD. 名古屋'), '<c r="C14" t="str"><v>【答案】B</v></c>']),   # t omitted but text in <v>
    row_xml(15, [inl('A15', SC[0].replace('首都', '都')), '<c r="C15"><v>2</v></c>']),                             # numeric answer 2 (no letter)
    row_xml(16, [inl('A16', SC[0].replace('首都', '街')), '<c r="C16" t="b"><v>1</v></c>']),                       # boolean answer "TRUE" -> fallback answer text
]
one('f07_numbers_booleans.xlsx', rows, sheet='types')

# f08 hidden sheets
build('f08_hidden_sheet.xlsx', [
    dict(name='visible-1', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, *SC)])),
    dict(name='hidden-1', part='xl/worksheets/sheet2.xml', xml=sheet_xml([qrow(1, *MC)]), state='hidden'),
    dict(name='veryhidden-1', part='xl/worksheets/sheet3.xml', xml=sheet_xml([qrow(1, *HS)]), state='veryHidden'),
    dict(name='explicit-visible', part='xl/worksheets/sheet4.xml', xml=sheet_xml([qrow(1, *SC)]), state='visible'),
    dict(name='odd-state', part='xl/worksheets/sheet5.xml', xml=sheet_xml([qrow(1, *MC)]), state='Hidden'),
])

# f09 whitespace: CRLF, lone CR, tabs, U+3000, NBSP, U+2028, U+0085, U+001C, trailing spaces. Char refs (&#13;) survive XML parsing; literal CRLF is normalised by the XML parser.
A = ("CRLF の問題。&#13;&#10;&#13;&#10;A. 甲&#13;&#10;B. 乙&#13;&#10;C. 丙")
C_ = "【答案】&#13;&#10;B&#13;&#10;&#13;&#10;【解析·怎么选】&#13;&#10;解析第一行&#13;&#10;解析第二行"
rows = [
    row_xml(1, [inl('A1', A, raw=True), inl('C1', C_, raw=True)]),
    row_xml(2, [inl('A2', 'literal CRLF in file\r\n\r\nA. 甲\r\nB. 乙', raw=True), inl('C2', '【答案】A', raw=True)]),                 # literal CRLF -> expat normalises to LF
    row_xml(3, [inl('A3', 'lone CR char ref&#13;&#13;A. 甲&#13;B. 乙', raw=True), inl('C3', '【答案】B', raw=True)]),
    row_xml(4, [inl('A4', 'タブ区切り。\n\n\tA.\t甲\n　B.　乙\n C. 丙(NBSP)', raw=True), inl('C4', '【答案】　C', raw=True)]),
    row_xml(5, [inl('A5', 'ホットスポット\n1. 項目一\t　はい / いいえ　\t\n2. 項目二　　はい / いいえ', raw=True), inl('C5', '【答案】1 はい；2 いいえ', raw=True)]),
    row_xml(6, [inl('A6', 'U+2028 と U+0085 の問題&#x2028;A. 甲&#x2028;B. 乙&#x85;C. 丙\n\nA. 本物A\nB. 本物B', raw=True), inl('C6', '【答案】A', raw=True)]),
    row_xml(7, [inl('A7', 'U+001C separator test\x1c\n\nA. 甲\nB. 乙'.replace('\x1c', '&#x1c;'), raw=True), inl('C7', '【答案】A', raw=True)]) if False else row_xml(7, [inl('A7', 'plain row')]),
]
one('f09_whitespace_crlf_tab_u3000.xlsx', rows, sheet='ws')

# f10 option label forms
def mk(label_fmt, n=4, stem='ラベルの形式テスト'):
    return stem + ' [%s]\n\n' % label_fmt.replace('{k}', 'X') + '\n'.join(label_fmt.replace('{k}', k) + ' 選択肢' + k for k in 'ABCDEFGHI'[:n])
forms = ['{k}.', '{k})', '{k}、', '{k}:', '{k}：', '{k}）', '{k}．', '({k})', '（{k}）', '[{k}]', '{k} ', '{k}　', '{k}-', '{k}。']
rows = []
for i, f in enumerate(forms, 1):
    rows.append(qrow(i, mk(f), None, '【答案】B'))
low = 'ラベル小文字\n\na. 甲\nb. 乙\nc. 丙'
rows.append(qrow(len(forms) + 1, low, None, '【答案】b'))
fw = 'ラベル全角\n\nＡ. 甲\nＢ. 乙'
rows.append(qrow(len(forms) + 2, fw, None, '【答案】A'))
sep = 'ラベルだけの行（A 換行 内容）\n\nA\n甲の内容\nB\n乙の内容\nC\n丙の内容'
rows.append(qrow(len(forms) + 3, sep, None, '【答案】C'))
mix = 'A の文字で始まる行\n\nA は最初の文字です\nA. 本物の甲\nB. 本物の乙'
rows.append(qrow(len(forms) + 4, mix, None, '【答案】B'))
one('f10_option_label_forms.xlsx', rows, sheet='labels')

# f11 9+ options (A..I): only A..H are letters the parser knows
nine = 'ラベル9個\n\n' + '\n'.join('%s. 選択肢%s' % (k, k) for k in 'ABCDEFGHI')
ten = 'ラベル10個\n\n' + '\n'.join('%s. 選択肢%s' % (k, k) for k in 'ABCDEFGHIJ')
eight = 'ラベル8個\n\n' + '\n'.join('%s. 選択肢%s' % (k, k) for k in 'ABCDEFGH')
one('f11_nine_options.xlsx', [qrow(1, nine, None, '【答案】H'), qrow(2, nine.replace('9', '9b'), None, '【答案】I'), qrow(3, ten, None, '【答案】A'),
                              qrow(4, eight, None, '【答案】H、G'), qrow(5, eight.replace('8', '8b'), None, '【答案】ABCDEFGH')], sheet='nine')

# f12 duplicated / skipped / out-of-order letters
rows = [
    qrow(1, '重複A\n\nA. 甲\nB. 乙\nB. 乙二\nC. 丙', None, '【答案】C'),
    qrow(2, '重複A2\n\nA. 甲\nA. 甲二\nB. 乙', None, '【答案】B'),
    qrow(3, '欠番\n\nA. 甲\nB. 乙\nD. 丁', None, '【答案】B'),
    qrow(4, '順序違い\n\nB. 乙\nA. 甲\nC. 丙', None, '【答案】A'),
    qrow(5, '選択肢が1つだけ\n\nA. 甲', None, '【答案】A'),
    qrow(6, '空の選択肢\n\nA.\nB. 乙', None, '【答案】B'),
    qrow(7, '空の選択肢2\n\nA. 甲\nB.\nC. 丙', None, '【答案】A'),
    qrow(8, '複数行の選択肢\n\nA. 甲\n続き\nB. 乙\n\nC. 丙\n最後', None, '【答案】A'),
    qrow(9, 'Aがない\n\nB. 乙\nC. 丙', None, '【答案】B'),
]
one('f12_duplicate_letters.xlsx', rows, sheet='dups')

# f13 hotspot variants
rows = [
    qrow(1, *HS),
    qrow(2, 'hotspot\n以下の文章\n1. 一つ目　yes / no\n2. 二つ目　yes / no', None, '【答案】1 yes；2 no'),
    qrow(3, 'HotSpot\n1. 項目A　はい / いいえ', None, '【答案】1 はい'),
    qrow(4, 'ホットスポット\n導入文\n1. 項目一　はい / いいえ\n2. 項目二\n3. 項目三', None, '【答案】1 はい；2 はい；3 いいえ'),                 # items 2,3 numbered without hint -> common choices
    qrow(5, 'ホットスポット\n導入文\n項目一　はい / いいえ\n項目二　はい / いいえ', None, '【答案】1 はい；2 いいえ'),                         # no numbers
    qrow(6, '番号なしマーカーなし\n1. 項目一　はい / いいえ\n2. 項目二　はい / いいえ', None, '【答案】1 はい；2 いいえ'),                   # no marker, via fallback parse_hotspot
    qrow(7, 'ホットスポット\n1. 項目一　Model Stage / Release Stage\n2. 項目二　Model Stage / Release Stage', None, '【答案】1 model stage；2 Release-Stage'),
    qrow(8, 'ホットスポット\n1. 三択　甲 / 乙 / 丙\n2. 二つ目　甲 / 乙 / 丙', None, '【答案】1 丙；2 甲'),
    qrow(9, 'ホットスポット\n1. スラッシュ詰め　はい/いいえ\n2. もう一つ　はい/いいえ', None, '【答案】1 はい；2 いいえ'),                        # '/' without spaces -> hint not recognised
    qrow(10, 'ホットスポット\n1. 項目一　はい / いいえ\n続きの説明行\n2. 項目二　はい / いいえ', None, '【答案】1 はい；2 いいえ'),             # continuation line goes to NEXT item
    qrow(11, 'ホットスポット\n1. 項目一　はい / いいえ\n2. 項目二　はい / いいえ', None, '【答案】1 はい'),                                      # missing item 2
    qrow(12, 'ホットスポット\n1. 項目一　はい / いいえ', None, '【答案】1 はい；2 いいえ'),                                                    # extra item 2
    qrow(13, 'ホットスポット\n1. 項目一　はい / いいえ', None, '【答案】1 たぶん'),                                                              # not in choices
    qrow(14, 'ホットスポット\n1. 項目一　はい / いいえ\n2. 項目二　はい / いいえ', None, '【答案】1 ○；2 ×'),                                     # canon symbols
    qrow(15, 'ホットスポット\n1. 項目一　はい / いいえ\n2. 項目二　はい / いいえ', None, '【答案】1 对；2 错误'),                                  # Chinese yes/no vs Japanese choices
    qrow(16, 'ホットスポット', None, '【答案】1 はい'),                                                                                          # marker only
    qrow(17, 'ホットスポット\nA. 甲\nB. 乙', None, '【答案】A'),                                                                                  # marker + options-looking lines -> options NOT tried
    qrow(18, '10. 十番\n11. 十一番　はい / いいえ\n12. 十二番　はい / いいえ', None, '【答案】12 はい；11 いいえ'),                              # 2-digit numbers, answers in other order
    qrow(19, 'ホットスポット\n1. 項目一　はい / いいえ\n2. 項目二　はい / いいえ', None, '【答案】1はい，2いいえ'),                               # fullwidth comma, no spaces
    qrow(20, 'ホットスポット\n1. 項目一　はい / いいえ\n2. 項目二　はい / いいえ', None, '【答案】1: 是\n2: 否'),                                  # newline separated
]
one('f13_hotspot_variants.xlsx', rows, sheet='hot')

# f14 answers: every accepted form and some unaccepted
base = '答案形式の問題 {n}\n\nA. 甲\nB. 乙\nC. 丙\nD. 丁'
ans_forms = ['【答案】B', '【答案】A、C', '【答案】AC', '【答案】b', '【答案】Ａ', '【答案】B（理由略）', '【答案】A, C', '【答案】A/C', '【答案】ABC', '【答案】Z',
             '【答案】答えはB', '答案：B', 'Answer: C', 'ANS = D', '正解：A', '正确答案: B', 'B', '【答案】', '【答案】ABCDEFGHI', '【答案】A、A、B', '【答案】\nB',
             '【正解】B', '【答案与解析】C', '【解析】选B\n【答案】A', '【答案】(A)(B)', '【答案】A（B）', '【答案】 A B C', '【答案】Option B', '【答案】BA', '【答案】CAT',
             '【答案】a、c', '【答案】E', '【答案】B。', '【答案】1', '【答案】甲']
rows = [qrow(i, base.format(n=i), None, f) for i, f in enumerate(ans_forms, 1)]
rows.append(qrow(len(rows) + 1, base.format(n='B列に答え'), 'B', None))                                    # answer only in column B
rows.append(qrow(len(rows) + 1, base.format(n='C空B翻訳'), '短い翻訳です', None))                         # B short non-answer used as answer
rows.append(qrow(len(rows) + 1, base.format(n='長すぎる'), None, 'B' + 'x' * 45))                            # C > 40 chars -> not used as answer
rows.append(qrow(len(rows) + 1, base.format(n='C改行'), None, 'B\nそのほか'))                                 # C multi-line -> not used
rows.append(qrow(len(rows) + 1, base.format(n='C空2'), None, None))
one('f14_answer_forms.xlsx', rows, sheet='answers')

# f15 translation match / mismatch
rows = [
    qrow(1, *SC),                                                                                                            # match
    qrow(2, SC[0].replace('首都', '都2'), '【中文翻译】\n下面哪个是首都？\nA. 大阪\nB. 东京\nC. 京都', '【答案】B'),                       # 3 options vs 4 -> zh None
    qrow(3, SC[0].replace('首都', '都3'), '【中文翻译】\n没有选项字母的翻译。', '【答案】B'),                                         # no options in zh
    qrow(4, SC[0].replace('首都', '都4'), '下面哪个是首都？\nA. 大阪\nB. 东京\nC. 京都\nD. 名古屋 这是没有括号标记的长翻译，长度超过四十个字符才会被当作翻译使用', '【答案】B'),   # no heading, len>40
    qrow(5, SC[0].replace('首都', '都5'), '短翻译\nA. 大阪\nB. 东京', '【答案】B'),                                                   # no heading, short -> not translation (but maybe answer?)
    qrow(6, *HS),                                                                                                            # hotspot match
    qrow(7, HS[0].replace('太陽', '太陽2'), '【中文翻译】\n热点题：下面的句子对吗？\n1. 太阳从东边升起。', '【答案】1 是；2 否'),         # 1 item vs 2 -> None
    qrow(8, HS[0].replace('太陽', '太陽3'), '【中文翻译】\n热点题：下面的句子对吗？\n2. 太阳从东边升起。\n1. 月亮是恒星。', '【答案】1 是；2 否'),   # numbering out of sequence
    qrow(9, HS[0].replace('太陽', '太陽4'), '【中文翻译】\nHotspot: 下面的句子对吗？\n1) 太阳从东边升起。\n续行\n2、月亮是恒星。', '【答案】1 是；2 否'),   # hotspot lead English + ) and 、 + continuation line
    qrow(10, SC[0].replace('首都', '都6'), '【中文翻译】\nＡ. 全角字母\nＢ. 乙', '【答案】B'),                                           # fullwidth letters in zh
    qrow(11, SC[0].replace('首都', '都7'), '【中文翻译】\n题干\n\nA\n大阪\nB\n东京\nC\n京都\nD\n名古屋', '【答案】B'),                       # label-on-own-line form in zh
    qrow(12, SC[0].replace('首都', '都8'), '【中文翻译】\n题干\nA. 大阪\nB. 东京\nC. 京都\nD. 名古屋', '【答案】B\n【翻译】另一个翻译标题在C列'),     # translation heading in C
    qrow(13, SC[0].replace('首都', '都9'), '【答案】B\n【中文翻译】\n题干\nA. 大阪\nB. 东京\nC. 京都\nD. 名古屋', None),                   # all in B
    qrow(14, SC[0].replace('首都', '都10'), '【中文翻译】\n题干\nA. 大阪\nB. 东京\nC. 京都\nD. 名古屋', '【答案】B\n【中文翻译】第二个翻译标题被忽略'),   # duplicate heading: first wins (setdefault)
    qrow(15, SC[0].replace('首都', '都11'), '【中文翻译】\n【解析·怎么选】在B列的解析\nA. 大阪\nB. 东京\nC. 京都\nD. 名古屋', '【答案】B'),   # empty translation section
    qrow(16, SC[0].replace('首都', '都12'), '  \n\n', '【答案】B'),
]
one('f15_translation_match_mismatch.xlsx', rows, sheet='tr')

# f16 DOCTYPE / entities
dt_sheet = (XMLDECL + '<!DOCTYPE worksheet [<!ENTITY e "HELLO">]><worksheet xmlns="%s"><sheetData>%s</sheetData></worksheet>' % (MAIN, qrow(1, '&e;\n\nA. 甲\nB. 乙', None, '【答案】A')))
build('f16a_doctype_in_sheet.xlsx', [dict(name='dt', part='xl/worksheets/sheet1.xml', xml=dt_sheet)])
dt_plain = (XMLDECL + '<!DOCTYPE worksheet><worksheet xmlns="%s"><sheetData>%s</sheetData></worksheet>' % (MAIN, qrow(1, '外部DTDなし\n\nA. 甲\nB. 乙', None, '【答案】A')))
build('f16b_doctype_no_entity.xlsx', [dict(name='dt', part='xl/worksheets/sheet1.xml', xml=dt_plain)])
bomb = (XMLDECL + '<!DOCTYPE lolz [<!ENTITY a "aaaaaaaaaa"><!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;"><!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">]>'
        '<worksheet xmlns="%s"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>&c;</t></is></c></row></sheetData></worksheet>' % MAIN)
build('f16c_entity_expansion_small.xlsx', [dict(name='bomb', part='xl/worksheets/sheet1.xml', xml=bomb)])
# DOCTYPE only in sharedStrings / only in workbook.xml
build('f16d_doctype_in_workbook.xlsx', [dict(name='ok', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, *SC)]))],
      wb_raw=XMLDECL + '<!DOCTYPE workbook [<!ENTITY x "y">]><workbook xmlns="%s" xmlns:r="%s"><sheets><sheet name="ok" sheetId="1" r:id="rId1"/></sheets></workbook>' % (MAIN, RNS))
# undefined entity reference without DOCTYPE
build('f16e_undefined_entity.xlsx', [dict(name='ue', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, '&amp;未定義', None, '【答案】A')]).replace('未定義', '&undefined;'))])

# f17 header only / empty sheets / sheet without sheetData rows
one('f17a_header_only.xlsx', [qrow(1, '题目', '翻译', '答案')], sheet='header-only')
build('f17b_empty_sheets.xlsx', [
    dict(name='no-rows', part='xl/worksheets/sheet1.xml', xml=sheet_xml([])),
    dict(name='no-sheetdata', part='xl/worksheets/sheet2.xml', xml=XMLDECL + '<worksheet xmlns="%s"/>' % MAIN),
    dict(name='remarks-only', part='xl/worksheets/sheet3.xml', xml=sheet_xml([qrow(1, '短いメモ'), qrow(2, 'ほかのメモ', '', '')])),
    dict(name='blank-cells-only', part='xl/worksheets/sheet4.xml', xml=sheet_xml([row_xml(1, ['<c r="A1"/>'])])),
    dict(name='real', part='xl/worksheets/sheet5.xml', xml=sheet_xml([qrow(1, *SC)])),
])
# header words and short remarks
one('f17c_headings_and_remarks.xlsx', [qrow(1, '题目'), qrow(2, '題目', 'x'), qrow(3, '問題', '', ''), qrow(4, 'Question', 'a', 'b'), qrow(5, '答案', 'a', 'b'), qrow(6, '解析', 'a', 'b'),
                                       qrow(7, '【记忆口诀】', 'a', 'b'), qrow(8, '题目题目题目题目题目', 'a', 'b'), qrow(9, '正解', None, None), qrow(10, 'メモ 14 chars!!', None, None),
                                       qrow(11, 'メモ 15 chars!!!', None, None), qrow(12, 'これは短い', 'B', None), qrow(13, '翻译', 'a', 'b'), qrow(14, '口诀', 'a', 'b'),
                                       qrow(15, '答案答案答案答案', 'a', 'b'), qrow(16, '答案答案答案答案答', 'a', 'b'), qrow(17, 'Question ', 'a', 'b'), qrow(18, 'question', 'a', 'b')],
    sheet='heads')

# f18 duplicates (same sheet + stem + first option -> id suffix -2, -3), same stem different sheet
build('f18_duplicate_ids.xlsx', [
    dict(name='dupA', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, *SC), qrow(2, *SC), qrow(3, SC[0].replace('\n\n', '\n \n'), SC[1], SC[2])])),
    dict(name='dupB', part='xl/worksheets/sheet2.xml', xml=sheet_xml([qrow(1, *SC)])),
])

# f19 sheet names: empty, unicode, special characters, very long, duplicate names
build('f19_sheet_names.xlsx', [
    dict(name='', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, *SC)])),
    dict(name='日本語シート & <特殊> "q"', part='xl/worksheets/sheet2.xml', xml=sheet_xml([qrow(1, *SC)])),
    dict(name='x' * 60, part='xl/worksheets/sheet3.xml', xml=sheet_xml([qrow(1, *SC)])),
    dict(name='same', part='xl/worksheets/sheet4.xml', xml=sheet_xml([qrow(1, *SC)])),
    dict(name='same', part='xl/worksheets/sheet5.xml', xml=sheet_xml([qrow(1, *MC)])),
])

# f20 package structure problems
build('f20a_missing_rels.xlsx', [dict(name='s', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, *SC)]))], drop=('xl/_rels/workbook.xml.rels',))
build('f20b_missing_workbook.xlsx', [dict(name='s', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, *SC)]))], drop=('xl/workbook.xml',))
build('f20c_broken_sheet_xml.xlsx', [dict(name='s', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, *SC)])[:-30])])
build('f20d_bad_sharedstrings_xml.xlsx', [dict(name='s', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, *SC)]))], shared='<sst><si><t>unterminated')
build('f20e_no_sheets_element.xlsx', [dict(name='s', part='xl/worksheets/sheet1.xml', xml=sheet_xml([qrow(1, *SC)]))],
      wb_raw=XMLDECL + '<workbook xmlns="%s" xmlns:r="%s"/>' % (MAIN, RNS))
# prefixed (non-default) namespace everywhere: x:worksheet / x:row / x:c ; and a wrong-namespace worksheet
px = (XMLDECL + '<x:worksheet xmlns:x="%s"><x:sheetData><x:row r="1"><x:c r="A1" t="inlineStr"><x:is><x:t>%s</x:t></x:is></x:c><x:c r="C1" t="inlineStr"><x:is><x:t>【答案】B</x:t></x:is></x:c></x:row></x:sheetData></x:worksheet>' % (MAIN, escape(SC[0])))
build('f20f_prefixed_namespace.xlsx', [dict(name='pfx', part='xl/worksheets/sheet1.xml', xml=px)])
wrongns = sheet_xml([qrow(1, *SC)]).replace(MAIN, 'http://example.invalid/not-main')
build('f20g_wrong_namespace_sheet.xlsx', [dict(name='wrongns', part='xl/worksheets/sheet1.xml', xml=wrongns)])
# rels in a different namespace prefix / rels with Target Mode External / transitional vs strict namespaces
strict = sheet_xml([qrow(1, *SC)]).replace(MAIN, 'http://purl.oclc.org/ooxml/spreadsheetml/main')
build('f20h_strict_ooxml_namespace.xlsx', [dict(name='strict', part='xl/worksheets/sheet1.xml', xml=strict)])
# Windows-style backslash member name (platform dependent: zipfile converts os.sep to '/' on Windows only)
bs = os.path.join(OUT, 'f20i_backslash_member_name.xlsx')
with zipfile.ZipFile(bs, 'w', zipfile.ZIP_DEFLATED) as z:
    z.writestr('[Content_Types].xml', XMLDECL + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>')
    z.writestr('xl/workbook.xml', XMLDECL + '<workbook xmlns="%s" xmlns:r="%s"><sheets><sheet name="bs" sheetId="1" r:id="rId1"/></sheets></workbook>' % (MAIN, RNS))
    z.writestr('xl/_rels/workbook.xml.rels', XMLDECL + '<Relationships xmlns="%s"><Relationship Id="rId1" Type="%s/worksheet" Target="worksheets/sheetwin.xml"/></Relationships>' % (PKG, RNS))
    z.writestr('xl\\worksheets\\sheetwin.xml', sheet_xml([qrow(1, *SC)]))
# duplicate zip entry names (same member twice): which copy is read?
dup = os.path.join(OUT, 'f20j_duplicate_zip_entries.xlsx')
import warnings
with warnings.catch_warnings():
    warnings.simplefilter('ignore')
    with zipfile.ZipFile(dup, 'w', zipfile.ZIP_DEFLATED) as z:
        z.writestr('[Content_Types].xml', XMLDECL + '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"/>')
        z.writestr('xl/workbook.xml', XMLDECL + '<workbook xmlns="%s" xmlns:r="%s"><sheets><sheet name="dup" sheetId="1" r:id="rId1"/></sheets></workbook>' % (MAIN, RNS))
        z.writestr('xl/_rels/workbook.xml.rels', XMLDECL + '<Relationships xmlns="%s"><Relationship Id="rId1" Type="%s/worksheet" Target="worksheets/sheet1.xml"/></Relationships>' % (PKG, RNS))
        z.writestr('xl/worksheets/sheet1.xml', sheet_xml([qrow(1, 'FIRST COPY の問題\n\nA. 甲\nB. 乙', None, '【答案】A')]))
        z.writestr('xl/worksheets/sheet1.xml', sheet_xml([qrow(1, 'SECOND COPY の問題\n\nA. 甲\nB. 乙', None, '【答案】B')]))
# non-xlsx inputs
with open(os.path.join(OUT, 'f21a_not_a_zip.xlsx'), 'wb') as f:
    f.write(b'name,value\n' + b'x,1\n' * 30)
with open(os.path.join(OUT, 'f21b_legacy_xls_header.xlsx'), 'wb') as f:
    f.write(b'\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1' + b'\x00' * 200)
with open(os.path.join(OUT, 'f21c_empty.xlsx'), 'wb') as f:
    f.write(b'')
with open(os.path.join(OUT, 'f21d_pk_but_truncated.xlsx'), 'wb') as f:
    f.write(b'PK\x03\x04' + b'\x00' * 200)
# UTF-8 BOM / UTF-16 sheet XML / encoding declarations
bom = b'\xef\xbb\xbf' + sheet_xml([qrow(1, *SC)]).encode('utf-8')
build('f22a_utf8_bom_sheet.xlsx', [dict(name='bom', part='xl/worksheets/sheet1.xml', xml=bom)])
u16 = sheet_xml([qrow(1, *SC)]).replace('encoding="UTF-8"', 'encoding="UTF-16"').encode('utf-16')
build('f22b_utf16_sheet.xlsx', [dict(name='u16', part='xl/worksheets/sheet1.xml', xml=u16)])
# control characters / surrogate-ish / emoji / combining / NFC vs NFD
rows = [qrow(1, 'Unicode: 😀 絵文字を含む問題\n\nA. 😀 甲\nB. é (NFD)\nC. é (NFC)', None, '【答案】B')]
one('f23_unicode_astral_nfd.xlsx', rows, sheet='uni😀')
# big sheet: many rows (performance / memory shape only)
big = [qrow(i, SC[0].replace('首都', '都%d' % i), SC[1], SC[2]) for i in range(1, 3001)]
one('f24_3000_rows.xlsx', big, sheet='big')
# row with 40 000 identical empty-ish cells in a wide row & huge cell text
wide = [row_xml(1, [inl('A1', SC[0]), inl('C1', SC[2])] + [inl('%s1' % c, 'x') for c in ('D', 'E', 'F', 'G')])]
one('f25_wide_row.xlsx', wide, sheet='wide')
print('fixtures written to', OUT)

# f26 check_xlsx bypasses (server.py scans only members whose NAME ends in .xml/.rels, for the literal ASCII bytes '<!ENTITY' / '<!DOCTYPE')
ent_sheet = (XMLDECL + '<!DOCTYPE worksheet [<!ENTITY e "EXPANDED">]><worksheet xmlns="%s"><sheetData>%s</sheetData></worksheet>' % (MAIN, row_xml(1, [inl('A1', '&e;\n\nA. 甲\nB. 乙', raw=True), inl('C1', '【答案】A')])))
build('f26a_doctype_in_utf16_sheet.xlsx', [dict(name='u16ent', part='xl/worksheets/sheet1.xml', xml=ent_sheet.replace('encoding="UTF-8"', 'encoding="UTF-16"').encode('utf-16'))])
build('f26b_doctype_in_nonxml_extension.xlsx', [dict(name='dat', part='xl/worksheets/sheet1.dat', xml=ent_sheet)])
build('f26c_doctype_in_uppercase_ext.xlsx', [dict(name='UP', part='xl/worksheets/sheet1.XML', xml=ent_sheet)])
print('f26 written')
