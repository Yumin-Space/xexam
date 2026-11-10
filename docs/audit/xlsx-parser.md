# Phase 0 审计草稿：XLSX 解析器行为与 Rust 对等性（维度 `parser`）

> 范围：`app/xlsx_parser.py`（410 行）以及决定"什么文件会走到解析器"的 `server.py:check_xlsx`。
> 只读审计，未改动仓库。证据文件全部在 `（审计时的临时目录）`。
> 记号：**事实** = 读代码或实际运行得到；**规格要求** = 来自 owner 的现代化规格；**建议** = 我的推荐。
> 规格里把数据库文件写成 `<old-name>.db`，真实文件是 `xexam.db`（`server.py` 的 `open_store()` 里 `os.path.join(data_dir, 'xexam.db')`）：这是规格与实际的一处差异，已按实际名称处理。

## 0. 审计依据（md5 与环境）

| 文件 | md5 | 备注 |
|---|---|---|
| `app/xlsx_parser.py` | `239b594ef8cbba5206e9a6f116178afe` | 全程未变；git 历史只有首个提交 1cb7512 触及它，工作区相对 HEAD 无差异（`git diff --stat HEAD -- app/` 为空） |
| `server.py` | 开始时 `48306e1f...`，生成最终 golden 时 `fb9e06bf74b8db40c586108f77aa8950` | 审计过程中被别人改过；`check_xlsx` 仍是 `server.py:63-85`，上传处理在 `server.py:336-352`，`MAX_UPLOAD` 在 `:49`。所有 golden 的 `meta.server_md5` 都是 `fb9e06bf...` |
| `app/db.py` | 开始 `02846cbc...`，后来 `3ea04694...` | 只用来确认 `make_id` 的用途（`db.py:11`）和 `plan_import` 对解析结果的使用 |
| `web/app.js` | 开始 `0dd0e113...`，后来 `da9c02ac...` | 文件在审计中被改过，下面引用的 `app.js` 行号以 `0dd0e113` 版本为准 |

运行环境（golden 的产生环境）：Python 3.10.11、expat 2.5.0、Windows（`meta.platform = win32`），Unicode 数据库版本 13.0.0（`unicodedata.unidata_version`）。
**生产镜像是 `python:3.12-slim`（`Dockerfile:2`）**，所以 Unicode 版本（15.0）、expat 版本和 zipfile 的平台行为都可能与 golden 不同，见 §9。

## 1. 结论摘要（回答"Rust 移植要复制多少行为"）

1. **解析器是一个纯函数**：输入 = 一个 `.xlsx` 字节流，输出 = `(sheets, questions)` 或一条中文错误信息。没有 I/O、没有状态，所以可以用"同一文件 → 两份 JSON → diff"做完整对等验证（`run_parser.py` 就是为此写的）。
2. **必须逐字节复制的部分**（P0，属于业务规则 / 前端契约）：题目字典的 14 个字段及其生成规则；`make_id`（SHA-1 前 12 位十六进制，写入数据库，决定学习记录是否跟着题走）；选项 / 热点 / 答案 / 翻译 / 解析 / 口诀的全部识别规则；行号语义；隐藏工作表过滤；工作表顺序；`skipped` 里的 7 条原因文案（前端原样展示，`web/app.js:525` 一带）。
3. **必须复制、但文案来自 Python 库的部分**（P1）：`ET.ParseError`、`KeyError`、`zipfile` 异常的文字。Rust 侧无法逐字复制，只能约定"失败类别 + HTTP 400 + 中文前缀一致"。
4. **有理由故意不同的部分**（P2，需 owner 批准并写进 migration 文档）：`HINT_RE` 的多项式回溯（Rust `regex` 是线性时间，行为更好）；`<!DOCTYPE`/`<!ENTITY` 检查可被绕过（UTF-16、非 `.xml` 后缀、大写后缀，已验证实体会被展开）；没有行 / 列 / 文本长度上限。
5. **Rust `regex` 不支持的构造只有一处**：`parse_letters` 里的 lookbehind + lookahead（`xlsx_parser.py:216`）。其余 17 个正则全部可以直译。需要手写的还有：Python `int()` 对 Unicode 数字 / 空白 / 下划线 / 正负号 / 任意大整数的宽松解析（4 处），以及"按码点"的切片和 `len()`（`make_id` 的 `[:300]`/`[:200]`、长度阈值 15 / 8 / 40）。
6. **真实数据不碰到的边角很多**：owner 的两个私有工作簿都用 sharedStrings + 富文本 run，行 / 列规整；它们能覆盖主路径，但覆盖不到 inlineStr 以外的类型、隐藏表、rPh、DOCTYPE 等，所以我生成了 47 个合成夹具。

## 2. 工作簿怎么打开（`read_workbook`，`xlsx_parser.py:85-108`）

| 步骤 | 事实 | 行号 |
|---|---|---|
| 打开 | `zipfile.ZipFile(path)`，传入的是 `io.BytesIO(body)`，整个请求体（≤20 MB）在内存里 | 87；`server.py:347` |
| 成员名集合 | `names = set(z.namelist())`；之后用原样字符串比较（大小写敏感，不做百分号解码，不把 `\` 换成 `/`） | 88 |
| sharedStrings | **固定路径** `xl/sharedStrings.xml`；存在才读；`sst.findall(NS+'si')` 只取根元素的直接子 `si`；索引 = 在这些 `si` 里的位置。不看 workbook 的 relationship 去找它 | 90-92 |
| workbook | **固定路径** `xl/workbook.xml` 和 `xl/_rels/workbook.xml.rels`，缺失抛 `KeyError`（最终变成"缺少 .xlsx 必需的内容"） | 93-94；379 |
| 不读的部分 | `[Content_Types].xml`、`_rels/.rels`（officeDocument 关系）、styles、theme、docProps、`xl/worksheets/_rels`。因此日期格式、数字格式、合并单元格、列宽全部忽略 | 全文 |
| 关系表 | `{Id: Target}`，只看 `Relationship` 的 `Id`/`Target`；**不检查 `Type`、不检查 `TargetMode`**；重复 Id 后者覆盖前者 | 95 |
| 工作表顺序 | **按 `workbook.xml` 里 `<sheet>` 的顺序**，用 `r:id`（`RNS+'id'`）经关系表解析成 part 路径，**不假定 `sheetN.xml`** | 97-107 |
| 隐藏表 | `state in ('hidden','veryHidden')` 跳过；**大小写敏感**，`Hidden` / `HIDDEN` 不算隐藏（夹具 f08 验证：`odd-state` 被读入） | 99-100 |
| 找不到的表 | `r:id` 不在关系表里，或目标 part 不在 zip 里：**静默丢弃**，无任何提示（夹具 f03：`missing-part`、`no-such-rid` 被丢弃） | 101-106 |
| 表名 | `s.get('name') or ''`，不限长度、不去重、允许空串（f19：空名、60 字符名、两个同名表都通过） | 107 |
| part 路径 | `_part_path`：以 `/` 开头 → `lstrip('/')`（去掉**所有**前导斜杠，不做 normpath）；否则 `posixpath.normpath(posixpath.join('xl', target))`，所以 `../xl/worksheets/x.xml` 可解析（f03 `dotdot` 通过），`../../x` 变成 `../x` 不存在而被丢弃 | 42-45 |
| 重复 zip 条目 | `z.read(name)` 取**最后一个**同名条目（f20j：读到 "SECOND COPY"） | 49 |
| 反斜杠成员名 | Windows 上 zipfile 会把 `os.sep` 换成 `/`（`zipfile.ZipInfo.__init__` 里 `if os.sep != "/" and os.sep in filename`，已用 `inspect.getsource` 确认）；**Linux 容器里不换**。f20i 在 win32 通过，Linux 结果未验证，是平台相关行为 | 见 `evidence/probe1.py` 输出 |

### 2.1 命名空间

`NS`/`RNS`/`PKG` 是三个固定的 URI（第 13-15 行）。ElementTree 把名字展开成 `{uri}local`，所以**前缀无关、URI 必须精确匹配**：
- 默认命名空间或 `x:` 前缀都行（f20f 通过）。
- `worksheet` 用了别的 URI（f20g）或 Strict OOXML 的 `purl.oclc.org` URI（f20h）：**不报错**，该表读出 0 行，最终该表以 `total=0,usable=0` 出现在预览里（f20g 的 `sheets=[["wrongns",0,0,0]]`），用户看到"这张表 0 题"但没有原因。
- Strict OOXML 的 `r:id` 命名空间也不同，所以整本 Strict 工作簿会得到 0 个工作表、0 题且无报错（f20h 的 sheets 里有 `strict` 一项是因为它只改了 worksheet 的 URI，workbook 仍用旧 URI；完整 Strict 文件未构造，**未验证**）。

### 2.2 XML 解析器相关行为（Rust 要显式处理）

| 行为 | 事实 / 证据 | Rust 侧 |
|---|---|---|
| 编码自动识别 | expat 接受 UTF-8（含 BOM，f22a 通过）和带 BOM 的 UTF-16（f22b 通过，status 200）以及声明的 ISO-8859-1/US-ASCII | `quick-xml` 默认只处理 UTF-8；要么拒绝 UTF-16（行为差异，需决定），要么先转码 |
| 行尾规范化 | 元素文本里**字面**的 `\r\n`/`\r` 被 expat 规范化为 `\n`；字符引用 `&#13;` 保留为 CR（f09 第 1-3 行）。**owner 两个真实工作簿的 sharedStrings.xml 里每个换行都是字面 CRLF**（CR 计数 990 / 873，等于 LF 计数），所以真实数据就依赖这一步 | `quick-xml` 不做 XML 1.0 §2.11 规范化（我没有运行 Rust 验证，依据文档记忆，标为"待验证"）。后果：`len(a.strip()) < 15` 等长度阈值会多算一个字符，`_norm_nl` 之前的 `val.strip()` 也受影响。建议在文本提取层实现 §2.11（先规范化字面 CR，再展开字符引用） |
| 实体 | 内部实体会被展开（f26a/b/c 里 `&e;` 变成 `EXPANDED`）；未定义实体报 `ParseError: undefined entity`（f16e） | `quick-xml` 不展开自定义实体；应对任何 `DOCTYPE` 事件一律拒绝（建议） |
| 错误文案 | `文件里的 XML 读不了：undefined entity: line 1, column 293` / `no element found: line 15, column 25` 等，是 expat 的文字 | 无法逐字复制；对比时只比"类别 = 400 且前缀 `文件里的 XML 读不了：`" |
| 注释 / CDATA / 处理指令 | ElementTree 默认丢弃注释和 PI，CDATA 并入文本。**我没有构造夹具验证**，只依据 ET 的 TreeBuilder 默认行为，标为待验证 | 需要拼接 `Text`+`CData` 事件，跳过注释 |
| 属性值规范化 | 属性里的换行 / 制表符在 XML 解析阶段变空格（只影响 `name`、`r` 等属性，实际影响很小） | `quick-xml` 不自动规范化 |

## 3. 单元格层（`_read_sheet`，`xlsx_parser.py:48-82`）

| 主题 | 事实 | 行号 / 证据 |
|---|---|---|
| 遍历行 | `root.iter(NS+'row')`：**根下任意深度**的所有 `row` 元素，不要求在 `sheetData` 下 | 51 |
| 行号 | `int(row.get('r'))`，失败（缺失 / 非数字）则**整行连同所有单元格静默丢弃**（f05 第 5、6 行）。展示给用户的行号就是 `r` 属性本身（不是出现顺序）；允许 0、负数、重复、乱序 | 52-55 |
| 行顺序 | **保持文档顺序，不排序**（f05：行出现顺序 1,2,3,10,8,11，题目顺序同样是 10 在 8 前）。数据库 `seq` 就是这个枚举顺序（`db.py:211`） | golden `f05` |
| 单元格 | 只取 `row` 的**直接子** `c`；用 `c.get('r')` 取列，字母 `[A-Za-z]+` 转成 0 基列号；没有 `r` 或不以字母开头 → 该单元格静默丢弃（f05 第 4 行：A 丢了只剩 C） | 57-60；32-39 |
| 列号 | 小写引用可用（`c3`）；**列号无上限**（Python 大整数，`ZZZZZZZZZZ…` 不会溢出）；之后只使用 0/1/2 列（A/B/C），其余列被读入 `cells` 后忽略（f05 第 2、13 行） | 37-39；`parse_workbook` 的 `cells.get(0/1/2)` |
| 重复引用 | 同一行同一列出现两次：后者覆盖前者（f05 第 11 行"二番目の A11 が勝つ"）；但若后者是空白，前者保留（因为空白不写入 `cells`） | 78-79 |
| `t="inlineStr"` | 取 `<is>` 子元素，用 `_si_text`；没有 `<is>`（例如把文本写进 `<v>`）→ 空串 → 丢弃（f07 第 13 行） | 62-64 |
| `t="s"`（共享串） | `int(v.text)` 查 `shared`；**负索引会从末尾回绕**（`-1` 取最后一个，探针验证：`['-1',' 1 ','+1','1_0','１','1.0','1e0','','99']` 的结果依次是 `s10, s1, s1, s10, s1`，`1.0`、`1e0`、空、`99` 被丢弃）；`ValueError`/`IndexError` → 丢弃该单元格 | 69-73；`evidence/probe2.py` |
| `t="b"` | `'TRUE' if v.text == '1' else 'FALSE'`（只有精确的 `"1"` 才是 TRUE；`" 1"`、`"true"` 都是 FALSE） | 74-75 |
| 其他类型 | `n`（默认）、`str`、`e`、`d`、无 `t`：**直接取 `<v>` 的原始文本**，不格式化。数字就是 XML 里写的那串（如 `12345`、`1E+3`、`0.10000000000000001`）；日期是序列号（如 `45322`）；错误值是 `#N/A`；公式只取缓存值，没有 `<v>` 的公式被丢弃（f07 第 8 行）；`t="d"` 的 ISO 日期原样保留 | 76-77；f07 |
| `<v>` 为空 | `v is None or v.text is None` → 丢弃 | 66-68 |
| 空白文本 | `if val.strip():` 才写入；纯空白单元格等于没有 | 78-79 |
| 空行 | `cells` 为空的行不进入 `rows` | 80-81 |
| 合并单元格 | 完全不处理（只读合并区域左上角那个有值的单元格） | 全文 |
| rPh（注音假名） | `_si_text` 只收集 `<t>` 子元素和 `<r>` 里的 `<t>`，**忽略 `<rPh>`**、`<phoneticPr>`、`<rPr>`（f02：行 3 的 `漢字` 没有 `かんじ`；`<rPh>` 夹在两个 run 之间也被丢掉：`次の`+`文を選べ…`） | 19-29 |
| 空 `si` | 保留为空串占位，索引不错位（f02 里 `<si></si>` 在索引 5） | 92 |

**限制（事实）**：解析器自身**没有任何行数、列数、单元格数、文本长度、XML 大小、解析时间上限**。唯一的上限在 `check_xlsx`（见 §8）和 20 MB 请求体（`server.py:49`）。

## 4. 题目模型（`parse_question`，`xlsx_parser.py:286-358`）

### 4.1 入口过滤（`parse_workbook`，`xlsx_parser.py:383-410`）

对每一行（A=0、B=1、C=2 列）：
1. A 列空 → 静默跳过（不计入 `total`）。
2. **短备注**：`len(a.strip()) < 15` 且 B、C 都为空 → 静默跳过（f17c：两个 13、14 码点的短备注行都被静默跳过；恰好 15 码点的边界值我没有构造夹具，**未验证**，按代码 `< 15` 推断）。
3. **标题行**：`len(a.strip()) <= 8` 且（`classify_heading(a)` 命中 answer/explanation/mnemonic/translation，**子串匹配**，或 `a.strip()` 精确等于 `题目/題目/问题/問題/Question`，大小写敏感）→ 静默跳过。f17c 的 golden 里该表 `total=4`、skipped 的行是 8、12、16、18：即 `题目题目题目题目题目`（A 列 10 码点、不是标题词且 >8，计入 total 后因无选项进 skipped）、`これは短い`（有 B 列所以不算备注）、`答案答案答案答案答`（9 码点，> 8）、`question`（小写，不在 `HEADER_WORDS` 里）都进入了 skipped；`Question ` 带尾空格但 `strip()` 后等于 `Question`，被当标题跳过；8 码点的 `答案答案答案答案` 被当标题跳过。
4. 其余计入 `total`；`parse_question` 返回 `None` 则记入该表的 `skipped`，否则加入题目并重名去重。
5. **去重 id**：`seen` 是整个工作簿共享的集合；重复时加后缀 `-2`、`-3`…（f18：同表三份相同题得到 `…`、`…-2`、`…-3`；不同表的相同内容因为 `sheet` 参与哈希所以 id 不同）。
6. **表是否进入结果**：`if info['total'] or not rows`。含义：有题的表进入；**完全没有内容行的表进入**（显示 0 题）；**有内容行但一题都没统计到的表（例如数据写在 B 列开始、只有表头）被静默省略，用户得不到任何提示**（f06、f17a 的 `sheets=[]`）。

### 4.2 题型判定

- 热点标记：A 列第一行 `.lower()` 精确等于 `ホットスポット` 或 `hotspot`（`xlsx_parser.py:293`）。同一行里还有别的字则不算。`tag` 字段在标记存在时固定为 `ホットスポット`（即使原文是 `hotspot`），没有标记时为 `''`。
- 有标记：`body` = 第一行之后的文本；**不尝试选项**，直接 `parse_hotspot`（297-300）。
- 无标记：先 `parse_options`；没有选项再 `parse_hotspot`（所以没有 `ホットスポット` 标记的热点题也能识别，f13 第 6 行，`tag=''`）。
- 都没有 → skipped 原因 1。

### 4.3 选项解析（`parse_options`，157-180；`OPT_LINE`，152）

- 按 `\n` 分行（**不是 splitlines**）。
- `OPT_LINE = ^\s*([A-Ha-h])(?:\s*[.．、)）:：]\s*(.*)|\s*)$`：
  - 字母只认 **ASCII 的 A-H / a-h**（小写转大写，f10 第 15 行通过；全角 `Ａ` 不认，第 16 行被跳过）。
  - 分隔符只认 `. ． 、 ) ） : ：`（f10 第 1-7 行通过）；**不认** `(A)`、`（A）`、`[A]`、`A `（字母后直接空格）、`A　`（全角空格）、`A-`、`A。`（f10 第 8-14 行全部被跳过，原因 1）。
  - 单独一行只有字母（`A` 换行 内容）也认（f10 第 17 行，选项文本来自后面几行）。
- 起点 = 第一个字母为 A 的行（之前是题干）。之后按 `expected` 依次期待 B、C…；匹配到期待字母就开新选项，**其余所有行（包括重复字母、跳号字母、乱序字母）都并入当前选项文本**（f12：`B. 乙 / B. 乙二` 变成一个选项 `乙\nB. 乙二`；`A, A` 同理；`A,B,D`（缺 C）把 `D. 丁` 并入 B）。
- 选项文本 = 各行 `strip()` 后去掉空行，用 `\n` 连接。
- **字母只到 H**：第 9 个选项 `I.` 不会被识别，成为 H 的续行（f11 第 1 行 `keys=ABCDEFGH`）；10 个选项同理。答案写 `I` → 原因 3（f11 第 2 行）。
- 被判为"没有选项"的条件：识别到的选项少于 2 个，或任一选项文本为空（f12 第 5、6、7 行）。
- 起点必须是 A：只有 B、C 的题没有选项（f12 第 9 行）；`B/A/C` 乱序（第 4 行）也失败。
- 某一行只有字母 `A` 的普通文本也会被当成起点（读代码得出，未做夹具）。

### 4.4 热点题（`parse_hotspot`，183-209）

- `NUM_PREFIX = ^\s*(\d{1,2})\s*[.．、)）]\s*`，`HINT_RE = [\t　 ]+([^\t　/]+?(?: +/ +[^\t　/]+?)+)[ \t　]*$`。
- 每行去掉序号前缀（`had_num` = 去前缀后文本是否变化）；`HINT_RE.search` 命中则按 `re.split(r'\s+/\s+', ...)` 切成 `choices`，至少 2 个才算；**斜杠两侧必须有 ASCII 空格**（`はい/いいえ` 无空格不识别，f13 第 9 行）；支持 3 个及以上选项（第 8 行）。
- 第一个命中的 choices 成为 `common`；之后**没有提示但带序号的行**用 `common` 当选项（f13 第 4 行：第 2、3 项）。
- 空行：在出现第一个小项之前追加进题干，之后忽略。
- 没有提示、没有序号、已出现小项的行进入 `pending`，**并入下一个小项的开头**（f13 第 10 行：`続きの説明行\n項目二`）。**最后一个小项之后的残留行会被静默丢弃**（读代码：`pending` 不会被冲出；未做夹具）。
- 序号数字不参与匹配：答案按"第 k 个小项"对应，题目里写的数字只用于去前缀（f13 第 5 行无序号也能识别）。
- 小项答案（`answer`）是每个小项 `choices` 里的下标。

### 4.5 答案解析

**取答案文本（304-315）**，顺序：
1. `split_sections` 找到的 `answer` 段（标题含 `答案` 或 `正解`，标题 ≤24 字符；`【答案与解析】`也算答案，f14 第 23 行）。
2. 否则在 `combined = (B + "\n\n" + C).strip()` 里用正则 `(?:^|\n)[ \t]*(?:答案|正解|正确答案|Answer|Ans)[ \t]*[:：=][ \t]*(.+)`（`re.I`）取该行余下部分。注意 **B 列也在搜索范围内**。
3. 否则取 C、B 中第一个"单行、≤40 码点、不以 `【` 开头"的整格文本（f14 第 36、37 行；超过 40 字符或多行 → 原因 2，第 38-40 行）。
4. `ans` 为空串（例如只写了 `【答案】`）→ 原因 2（f14 第 18 行）。注意空串不会触发 2、3 的回退，因为判断的是 `is None`。

**单 / 多选（`parse_letters`，212-222）**：
- 先把括号内容整段替换成空格（半角 / 全角括号可混用，不处理嵌套）；只看**第一个非空行**。
- 用 lookbehind / lookahead 找"两侧不是 ASCII 字母数字的 A-H/a-h"；去重、转大写、排序。
- 没找到且整行恰好是 2-6 个 A-H/a-h → 拆成字符集合（`AC` → A,C；`BA` → A,B，f14 第 3、29 行）。
- **接受的形式（已验证）**：`B`、`A、C`、`AC`、`b`、`B（理由略）`（括号内容进 `note`）、`A, C`、`A/C`、`ABC`、`答えはB`、`答案：B`、`Answer: C`、`ANS = D`、`正解：A`、`正确答案: B`、`B。`、`a、c`、`BA`、`Option B`、` A B C`、`A（B）`（note=`B`）、`【正解】B`、`【答案与解析】C`、`【解析】选B\n【答案】A`、`【答案】\nB`、`A、A、B`（去重）。
- **不接受（已验证）**：全角 `Ａ`、`Z`、空、`ABCDEFGHI`（9 个）、`ABCDEFGH`（8 个，超过 2-6 的回退范围）、`(A)(B)`（括号被删掉）、`CAT`、`1`、`甲`、答案字母不在选项里（`E` 对 A-D → 原因 4）。
- 多个字母 → `kind='multi'`，否则 `single`；`answer` 永远是字母升序列表。**不校验答案个数与题干"2つ選択"是否一致。**

**热点题答案（`parse_item_answers`，225-234）**：
- 删括号 → 按 `[；;\n，,]+` 切段 → 每段用 `^\s*(\d+)\s*[.．、:：)）\-]*\s*(.+?)\s*$`；同号后者覆盖。
- 若解析出的条目 **<2 个**，用 `(\d+)\s*[.．、:：)）]?\s*([^\s\d;；,，]+)` 的 `findall` **整体替换**结果。这个回退只取到下一个空白为止，所以单个小项的多词答案 `1 Model Stage` 得到 `{1:'Model'}`，在选项 `Model Stage / Release Stage` 里找不到 → 原因 6（**已验证，这是现有缺陷**）；多项并用分号（`1 Model Stage；2 Release Stage`）则正常。
- 答案文本与选项用 `canon` 比较（见 §6.2）。
- 检查顺序：先逐小项（缺答案 → 原因 5；不在选项里 → 原因 6），**最后**才检查多余的小项号（原因 7）。所以小项 1 缺失和多余小项同时存在时报的是 5。

### 4.6 解析 / 口诀 / 备注 / 翻译字段

- `split_sections(combined)`：用 `【([^】\n]{1,24})】` 找所有标题，`classify_heading` 按**子串**归类，优先级 **答案/正解 > 解析/解説 > 口诀/口訣/记忆/記憶 > 翻译/翻訳/译文**。不属于四类的 `【…】`（如 `【注意】`）**不是分段点**，会留在前一段文本里。每类只取**第一次出现**（`setdefault`），段文本到下一个已识别标题或末尾，再 `strip()`。
- `explanation`、`mnemonic` 就是对应段文本（只做换行规范化和 strip）；没有则 `''`。
- `note` = 答案段里所有括号内容，`；` 连接。

### 4.7 翻译（`parse_translation`，257-283）与"何时隐藏中文"

`translation` 字段（原文）：优先取 `translation` 段；否则取 B 列整格，**条件是 B 里没有任何 `【` 且 `len(b) > 40`**（f15 第 4 行 `zh` 有；第 5 行短于 40 → 无）。

`zh` 为 `None`（前端 `web/app.js:589` 的 `const hasZh = !!q.zh;` 据此隐藏"中文"切换，**不暴露结构错位的内容**）的条件，全部来自代码：
1. 没有翻译文本，或去掉热点前缀（`热点题 / 熱點題 / 热点 / hotspot` + 半角或全角冒号，`re.I`，仅位于开头）后为空。
2. 选择题：对翻译再跑 `parse_options`，**选项字母列表必须与日语题完全相同**（数量、顺序）。少一个选项（f15 第 2 行）、没有选项（第 3 行）、字母用了全角（第 10 行）、少于 2 个选项、有空选项，都返回 `None`。
3. 热点题：按行扫描，**序号必须等于 `已解析个数 + 1`**（从 1 开始连续）才开新小项；序号乱序（f15 第 8 行 `2.` 开头）当作普通行；最后 **小项个数必须等于日语小项数**（第 7 行 1≠2 → `None`）。
4. 题目既无选项也无小项 → `None`（实际不会发生）。
- 通过时 `zh = {'stem', 'options'}`（选择题）或 `{'stem', 'items'}`（热点题）。热点题的 `choices`（はい/いいえ…）**不翻译**，前端直接显示日语（`web/app.js` 对应处的注释）。
- 不检查翻译文本是不是中文；不检查 `stem` 是否非空。
- `translation` 字段本身（原文）仍会存进数据库并在"中文翻译"折叠块里显示，即使 `zh` 为 `None`。

### 4.8 题目字典的字段（JSON 键顺序，serde 结构体要保持一致才能字节级 diff）

`id, sheet, row, kind, tag, stem, options, items, answer, translation, zh, explanation, mnemonic, note`（`xlsx_parser.py:349-358`）。
`options[]` = `{key, text}`；`items[]` = `{text, choices}`；`answer` = 字母列表（选择题）或下标列表（热点）；`zh` = `{stem, options[]}` | `{stem, items[]}` | `null`。
`sheets[]` = `{name, total, usable, skipped:[{row, reason}]}`。

### 4.9 `make_id`（246-247）——必须逐位一致

`sha1( sheet + "\n" + re.sub(r'\s+','',stem)[:300] + "|" + re.sub(r'\s+','',first)[:200] ).hexdigest()[:12]`，`first` = 第一个选项文本（选择题）或第一个小项文本（热点）。
- `[:300]`、`[:200]` 是**码点**切片；Rust 必须用 `chars().take(n)`，不能按字节切。
- `\s+` 的集合见 §6.1。
- 不做 Unicode 规范化（NFC / NFD 的两个题 id 不同；f23 保存了含 NFD 的样例）。
- 这个 id 是数据库主键，也是学习记录的外键语义（`db.py:11`），所以任何偏差都会让学习记录"断链"。**这是整个移植里最关键的一个对等点。**

## 5. 会出现在"读不出来"列表里的全部文案（精确文本）

前端在预览里逐条显示 `第 ${k.row} 行：${k.reason}`（`web/app.js:525` 一带，文件已被改动，行号以旧版为准），每个工作表最多显示 `SKIP_SHOWN = 30` 条（`server.py:51`），其余用 `skippedMore`/`skippedCount` 汇总。

| # | 触发条件 | 精确文本 | 行号 |
|---|---|---|---|
| 1 | 既无选项也无带"はい / いいえ"的小项 | `没有找到选项（A、B、C…）或带「はい / いいえ」的小项` | `xlsx_parser.py:302` |
| 2 | 找不到答案 | `没有找到答案（C 列要写「【答案】…」）` | 315 |
| 3 | 答案里没有字母（`ans[:20]`，按码点截断） | `答案「%s」里没有找到选项字母` | 321 |
| 4 | 答案字母不在选项里 | `答案 %s 不在选项 %s 里`（两个 `%s` 都用 `、` 连接字母） | 324 |
| 5 | 某个小项没有答案 | `第 %d 小项没有答案` | 332 |
| 6 | 小项答案不在选项里（**答案文本不截断**） | `第 %d 小项的答案「%s」不在选项「%s」里`（选项用 ` / ` 连接） | 335 |
| 7 | 答案里有超出的小项号 | `答案里有第 %s 项，但题目只有 %d 个小项`（号码用 `、` 连接） | 339 |

整个文件被拒绝时的 `ValueError`（`xlsx_parser.py:377,379,381`；`server.py:63-85`）：
- `这个文件不是有效的 .xlsx（可能是 .xls、.csv，或者文件已损坏）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。`（`BadZipFile`，377）
- `这个文件缺少 .xlsx 必需的内容（%s），可能已损坏。`（`KeyError`，379；`%s` 是 `KeyError` 的 repr，f20a 实测为 `"There is no item named 'xl/_rels/workbook.xml.rels' in the archive"`）
- `文件里的 XML 读不了：%s`（`ET.ParseError`，381）
- `check_xlsx`（`server.py:63-85`）：`文件是空的，或者没有上传完整。`（<100 字节）、`这是旧版 .xls 格式，或者是设置了密码的 Excel 文件。…`（OLE 魔数）、`这个文件不是 .xlsx（可能是 .csv、.pdf 或别的格式）。…`、`这个 .xlsx 文件已损坏，打不开。…`、`这个文件里没有 Excel 工作簿的内容，不是有效的 .xlsx。`、`文件解压后太大，已拒绝。`、`文件里含有不被允许的内容（XML 实体声明），已拒绝。正常的 Excel 文件不会有这个。`
- 其他任何异常：`读取这个 Excel 时出错：<异常类名>: <异常文字>`（`server.py` 上传处理的 `except Exception`，约 `:348-351`），HTTP 400。

**可能逃出解析器、落到最后这条的异常（读代码得出；除标注外未逐一实测）**：
- `ValueError: Exceeds the limit (4300) for integer string conversion`：`parse_item_answers` 里 `int(m.group(1))` 遇到超过 4300 位数字（**函数级已验证**，`evidence/probe2.py`）；传播路径是 `parse_question`(328) → `parse_workbook`(397，没有 try) → 上传处理里的 `except ValueError`，所以**整份上传被拒绝**，而且消息是 Python 的英文原文。
- 加密 / 不支持压缩方法的成员：`RuntimeError`、`NotImplementedError`（读 zipfile 源码，未能构造样例：我的构造脚本被 Python 的写入端拒绝）。
- CRC 错误 / zlib 错误：`z.read(info)` 在 `check_xlsx` 内（`server.py:82-84`）不在 try 里，会以 `BadZipFile` 逃出并走最后一条 `读取这个 Excel 时出错：BadZipFile: …`（读代码得出，未运行）。
- `MemoryError`、`RecursionError`（极深嵌套的 XML）：未测。

## 6. Python → Rust 对等审计

### 6.1 正则逐条（全文件共 18 处；无 `splitlines`、无 `unicodedata`、无 `\w`/`\b`/`\W`，已 grep 确认）

| # | 行 | 模式（标志） | Rust `regex` 支持？ | 差异 / 风险 / 对应写法 |
|---|---|---|---|---|
| R1 | 33 | `[A-Za-z]+`（`re.match`，锚在开头） | 支持 | 无差异。列号用 Python 大整数，Rust 需 `u64` 饱和或限制长度（只有 0/1/2 列有意义） |
| R2 | 118 | `[\s_\-–—‐・.．、,，/／]+`（`re.sub`） | 支持（类内 `\s`、`\-` 合法） | **`\s` 差异**：Python 的 `\s` = `str.isspace()`，29 个字符，含 U+001C-U+001F；Rust Unicode `\s` = White_Space，不含这 4 个（`evidence/probe1.py`：Python 独有 `0x1c-0x1f`，Rust 独有 无；Rust 集合是我按 Unicode White_Space 定义手写的，**没有在 Rust 里运行**）。XML 1.0 不允许这 4 个控制字符出现在文本里（按规范推断，未运行验证），所以实际影响接近 0 |
| R3 | 141 | `【([^】\n]{1,24})】`（`finditer`） | 支持 | 重复上限按码点计数，一致。`m.start()/end()` 在 Rust 是**字节偏移**，后面的切片必须同样用字节偏移，不能混用码点 |
| R4 | 152 | `^\s*([A-Ha-h])(?:\s*[.．、)）:：]\s*(.*)|\s*)$`（`match`） | 支持 | 只在单行上用（先 `split('\n')`），`$` 在 Python 里允许匹配在末尾 `\n` 之前，但行内没有 `\n`，所以 Rust 的 `$`（无 `(?m)` = 文本末尾）等价。`.` 在两边都只排除 `\n`（U+2028、U+0085、`\r` 都能被 `.` 匹配）。第二分支让 `group(2)` 为 `None`，要用 `captures.get(2).map_or("")` |
| R5 | 153 | `^\s*(\d{1,2})\s*[.．、)）]\s*`（`match`/`sub(count=1)`） | 支持 | **`\d` 都是 Unicode Nd**（Python：650 个字符，等于 `unicodedata.category == 'Nd'`；Rust `\d` = `\p{Nd}`），但随后 `int(m.group(1))`（272 行）能解析全角、阿拉伯-印度数字（实测 `int('１２')=12`、`int('٣')=3`），Rust `str::parse::<u32>` **不能**。需要自己实现 Nd 数值（连续 10 个码点一组）或接受并记录差异 |
| R6 | 154 | `[\t\u3000 ]+([^\t\u3000/]+?(?: +/ +[^\t\u3000/]+?)+)[ \t\u3000]*$`（`search`） | 支持（`\u3000` 合法，惰性量词同 Perl 语义） | **Python 侧是灾难性回溯**（实测，Python 3.10.11：`'x' + n 个空格 + 'y'`，n=100 → 0.012 s，200 → 0.14 s，300 → 0.56 s，500 → 4.2 s，**1000 → 54 s**，约 n^3.5~n^4；证据 `evidence/probe3_redos.py`，更大的 n 我没再跑）。一个 20 MB 内允许的恶意单行就能卡住 Python 进程（GIL 持有）。Rust `regex` 线性时间，且保证与回溯引擎一致的 leftmost-first 结果，所以**结果等价、性能更好**（P2：故意不同） |
| R7 | 195 | `\s+/\s+`（`re.split`） | 支持 | 同 R2 的 `\s` 差异；无捕获组，Rust `split` 的首尾空串行为与 Python 一致 |
| R8 | 213 | `[（(][^）)]*[）)]` → `' '`（`sub`） | 支持 | 不处理嵌套；替换串是字面空格，Rust 里避免 `$` 展开（用 `NoExpand`） |
| R9 | **216** | `(?<![A-Za-z0-9])([A-Ha-h])(?![A-Za-z0-9])`（`finditer`） | **不支持**（lookbehind / lookahead） | 必须手写：遍历 `char`，对每个 `A-Ha-h` 检查前一个、后一个字符不是 ASCII 字母数字。边界字符是**只看 ASCII**，所以全角字母、汉字、`。` 都算边界（实测 `答えはB。`→B，`A1`→空，`Ａ`→空）。**不要用 `fancy-regex`**（回溯引擎，会把 R6 的问题带回来） |
| R10 | 220 | `[A-Ha-h]{2,6}`（`fullmatch`） | 支持 | 用 `^…$` 并确保是整串；Python 的 `fullmatch` 不允许结尾 `\n`，Rust 无 `(?m)` 的 `$` 同样不允许 |
| R11 | 226 | `[（(][^）)]*[）)]` → `''` | 支持 | 同 R8 |
| R12 | 228 | `[；;\n，,]+`（`split`） | 支持 | 一致 |
| R13 | 229 | `^\s*(\d+)\s*[.．、:：)）\-]*\s*(.+?)\s*$`（`match`） | 支持（惰性 `.+?` 配 `\s*$`） | `\d+` 之后 `int()` 可能是 5000 位的数（Python 抛 4300 位限制的 `ValueError`），Rust 需自定上限并决定是"拒绝整个文件"（复制）还是"跳过该小项"（修正） |
| R14 | 233 | `(\d+)\s*[.．、:：)）]?\s*([^\s\d;；,，]+)`（`findall`） | 支持 | 同上；该回退会**替换**前一步的全部结果（见 §4.5） |
| R15 | 246×2 | `\s+` → `''`（`sub`） | 支持 | 同 R2；之后的 `[:300]`/`[:200]` 是码点切片 |
| R16 | 254 | `^\s*(?:热点题|熱點題|热点|hotspot)\s*[:：]\s*`，`re.I`（`sub(count=1)`） | 支持 `(?i)` | Python `re.I` 对 str 是 Unicode 大小写折叠（实测 `re.search('Ans','ANſ',re.I)` 为真，`ſ` 与 `s`、开尔文符号与 `k` 都能匹配）；Rust `(?i)` 用 Unicode simple case folding，预期相同，**未在 Rust 里验证** |
| R17 | 306 | `(?:^|\n)[ \t]*(?:答案|正解|正确答案|Answer|Ans)[ \t]*[:：=][ \t]*(.+)`，`re.I`（`search`） | 支持 | 同 R16；`^` 无 `re.M`，只匹配整个字符串开头，所以靠 `\n` 实现"行首"；Rust 里也不要加 `(?m)`；`(.+)` 到行末 |
| R18 | 346 | `[（(]([^）)]*)[）)]`（`findall`） | 支持 | 一致 |

**`regex` 之外的限制**：Rust `regex` 不支持反向引用、lookaround、原子分组、占有量词；本文件**只用到了 lookaround（R9）**，没有用反向引用 / 原子分组 / 占有量词（逐条读过全部 18 个模式）。

### 6.2 `canon()`（`xlsx_parser.py:116-123`）

`re.sub(R2, '', str(s).strip().lower())`，再与 `_YES`/`_NO` 集合比较，命中则返回哨兵 `'\x00yes'`/`'\x00no'`。
- `strip()` / `lower()`：Python 的完整 Unicode 大小写映射（`'İ'.lower()` = `i`+U+0307；词尾 sigma `ΑΣ`→`ας`，已实测）；Rust `str::to_lowercase` 也实现了这两个特殊情形，但两边 Unicode 版本可能不同（Python 3.10 = 13.0，3.12 = 15.0，Rust 取决于工具链）。
- **不做 NFKC**：全角 `Ｙｅｓ` 小写后是 `ｙｅｓ`，不等于 `yes`（已实测）。
- 在比较时会删掉 `_ - – — ‐ ・ . ． 、 , ， / ／` 和所有空白，所以 `Model-Stage` 与 `Model Stage` 等价，`Y.` 等于 `yes`。

### 6.3 其他 Python 惯用法

| 项 | Python 行为 | Rust 对应 / 风险 |
|---|---|---|
| `splitlines()` | **未使用**（grep 确认）。全部用 `split('\n')`；`_norm_nl` 只把 `\r\n`/`\r` 换成 `\n`。U+2028、U+0085 保留在文本里，不会被当成换行（f09 第 6 行验证） | 必须用 `split('\n')`，**不要用 `str::lines()`**（`lines()` 会吃掉结尾 `\r` 并丢掉最后的空行） |
| `str.split('\n', 1)` | 最多切一次 | `splitn(2, '\n')` |
| `str.strip()` / `.isspace()` | Unicode 空白，含 U+001C-1F | `trim()` 用 White_Space；差异同 R2，实际可忽略 |
| `len(s)` | 码点数（`len('😀')=1`，`len('e\u0301')=2`，已实测） | `s.chars().count()`，**不是** `s.len()`（字节）。用在阈值 `<15`、`<=8`、`<=40`、`>40` |
| 切片 `s[:n]` | 按码点 | `chars().take(n).collect()`；用在 `make_id`、`ans[:20]`（原因 3 的文案） |
| `int()` | 接受首尾空白、`+`/`-`、下划线（`int('1_0')=10`）、Unicode 数字、任意大整数；>4300 位抛 `ValueError`（实测） | `parse::<i64>` 都不接受；用在 `row r`(53)、共享串索引(71)、序号(272)、答案键(229/233)。需要为每一处写清"接受什么"（见 §3 的 `-1` 回绕问题；建议负数一律视为非法，并把它记为已解释的差异） |
| `sorted()` | 字符串按码点；整数 | 字母都是 ASCII，字节序等价；整数用数值序 |
| 字典顺序 | 保持插入顺序；`cells`、`target`、`out` 均按键访问，顺序不影响结果；**结果 JSON 的键顺序**取决于 `dict` 字面量，见 §4.8 | `serde` 结构体字段顺序决定输出顺序；比较时建议用"规范化 JSON"，但前端不依赖顺序 |
| `hashlib.sha1(...).hexdigest()[:12]` | UTF-8 编码；孤立代理项在 XML 中不可能出现 | `sha1` crate，取前 6 个字节的十六进制 |
| `json.dumps(ensure_ascii=False)` | 不转义非 ASCII | `serde_json` 默认也不转义，一致 |
| 异常作为控制流 | `ValueError`/`IndexError` 在共享串里被吞掉；`KeyError`/`BadZipFile`/`ParseError` 被映射成中文消息 | 用显式的 `Result` 枚举；不能出现 `panic!`（Python 里 `IndexError` 被吞掉的位置，Rust 越界会 panic） |
| 集合 `set` | `seen`、`names`、`_YES`、`_NO` | `HashSet` |

## 7. 夹具与 golden（证据）

目录：`（审计时的临时目录）`

| 文件 | 说明 |
|---|---|
| `run_parser.py` | `python -B -I run_parser.py <xlsx> [--meta] [--no-reader] > out.json`：输出 `input / upload_check / parse / server_outcome / reader / meta`。`server_outcome` 复刻了 `POST /api/upload` 的结果（先 `check_xlsx` 后 `parse_workbook`；`ValueError` → 400 + 原文；其他异常 → 400 + `读取这个 Excel 时出错：类名: 文字`）。用 `-B` 与 `dont_write_bytecode`，不会在仓库里写 `.pyc`（仓库里已有的 `__pycache__` 是我开始前就有的，时间戳 05:22，早于我的第一次运行） |
| `make_fixtures.py` | 用 `zipfile` + 手写 XML 生成 47 个夹具（全部虚构内容）到 `fixtures/` |
| `run_all.py` | 对 demo + 全部夹具跑一遍，写 `golden/<名>.json` 和 `golden/_index.json` |
| `summarize.py` | 把一份 golden 的 skipped / 题目概要打印出来 |
| `private_stats.py` | 只输出私有工作簿的**结构统计**（见 §10），不输出任何题目文字 |
| `evidence/probe1.py` `probe2.py` `probe3_redos.py` | 本文中引用的探针（Unicode 空白 / 数字、`int()`、共享串索引、4300 位限制、ReDoS 计时） |
| `golden/*.json` | 48 份解析输出（demo + 47 个夹具） + `_index.json` + `private_structure_stats.json`（后者是**两个**顺序拼接的 JSON 对象，不是一个合法 JSON 文档） |

夹具覆盖（`fixtures/`）：

| 夹具 | 覆盖点 |
|---|---|
| f01 | 仅 inlineStr：单选 / 多选 / 热点 |
| f02 | 共享串：纯文本、富文本 run、`rPh`（在 `<t>` 之后 / 夹在两个 run 之间）、空 `si`、空 run、索引越界与非整数 |
| f03 | part 名不是 `sheetN.xml`：`data_a.xml`、嵌套目录、绝对路径 Target、`../` Target、目标缺失、未知 rId |
| f04 | `workbook.xml` 顺序与文件名顺序相反、rId 交叉 |
| f05 | 行内缺单元格、额外列、小写引用、无 `r` 的单元格 / 行、非数字行号、乱序行号、重复引用、空单元格、27 列 |
| f06 | 前导空列（数据在 B/C/D） |
| f07 | A 列为数字 / 布尔 / 公式 / 错误 / 日期 / 科学计数 / `inlineStr` 里写 `<v>` / 答案为数字或布尔 |
| f08 | 隐藏 / veryHidden / `visible` / `Hidden`（大小写） |
| f09 | CRLF（字面与字符引用）、孤立 CR、制表符、U+3000、NBSP、U+2028 / U+0085 |
| f10 | 14 种选项标签写法、小写、全角、"字母单独成行"、起始行是普通文本 `A …` |
| f11 | 8 / 9 / 10 个选项 |
| f12 | 重复字母、缺号、乱序、单选项、空选项、续行 |
| f13 | 热点变体 20 种（标记大小写、无序号、无标记、3 选项、无空格斜杠、续行、缺 / 多答案、符号答案、中日混用、两位序号、换行分隔） |
| f14 | 40 种答案写法（见 §4.5） |
| f15 | 16 种翻译情形（匹配 / 少选项 / 无选项 / 无标记长翻译 / 序号乱序 / 全角字母 / 标题在 C 列 / 重复标题 / 空翻译段） |
| f16a-e | DOCTYPE（工作表、无实体、小型实体展开、workbook、未定义实体） |
| f17 | 仅表头、全空表、短备注、标题词边界 |
| f18-f19 | 重复 id、表名（空 / 特殊字符 / 60 字符 / 同名） |
| f20a-j | 缺 rels / 缺 workbook / 截断的 XML / 坏 sharedStrings / 无 `<sheets>` / 前缀命名空间 / 错误命名空间 / Strict URI / 反斜杠成员名 / 重复 zip 条目 |
| f21 | 非 zip、OLE 头、空文件、`PK` 开头但截断 |
| f22-f25 | UTF-8 BOM、UTF-16、含 emoji / NFD、3000 行、宽行 |
| f26a-c | `check_xlsx` 的绕过：UTF-16 编码的 DOCTYPE、非 `.xml` 后缀、大写 `.XML` 后缀 |

重要结果（`golden/_index.json`）：demo.xlsx 解析出 5 题（单选 B、多选 A,C、热点 [0,1,0]、单选 B、多选 B,D，`zh` 全部非空）。47 个夹具全部能跑出 golden，没有任何一个让运行器崩溃。

## 8. `check_xlsx` 与解析器之间的缝隙（决定 Rust 侧"拒绝什么"）

事实（`server.py:63-85`）：
- 请求体 <100 字节 → 拒绝；前 2 字节不是 `PK` → 区分 OLE 头 (`D0CF11E0A1B11AE1`) 与其他；`ZipFile()` 构造失败 → 损坏；`xl/workbook.xml` 不在 → 拒绝。
- 解压大小用 **zip 目录里声明的 `file_size`**：单个 >100 MB 或累计 >300 MB 拒绝。声明值是攻击者可控的；Python 的 `ZipExtFile` 读取到声明大小就停（读源码，未做压缩炸弹实验），所以内存上限是"声明值"，仍可达数百 MB，再加上 ElementTree 的整棵树。**没有压缩比检查、没有行 / 列 / 单元格上限、没有时间上限。**
- 只对文件名以 `.xml`/`.rels`（**大小写敏感**）的成员读取全文，并检查是否包含 ASCII 字节 `<!ENTITY` 或 `<!DOCTYPE`。

**已验证的绕过（夹具 f26a/b/c，`upload_check.ok = true`、`server_outcome.status = 200`、实体被展开，题干变成 `EXPANDED`）**：
1. 工作表 XML 用 **UTF-16** 编码（字节里没有 ASCII 的 `<!ENTITY`）。
2. 工作表成员名**不以 `.xml` 结尾**（如 `sheet1.dat`），只要 relationship 指向它。
3. 成员名用**大写后缀** `.XML`。

没有测试真实的"十亿笑声"炸弹（expat 2.4.0+ 自带放大保护，具体行为随 expat 版本而变）。**建议**：Rust 侧对每一个被读取的 XML 部件，在解析层直接拒绝任何 DOCTYPE，并拒绝非 UTF-8（或先转码）的编码，而不是靠扫描字节。

## 9. 现有缺陷与技术债（本维度）

| # | 描述 | 证据 |
|---|---|---|
| D1 | `HINT_RE` 多项式回溯，单行 1000 个空格 54 s | `evidence/probe3_redos.py`，§6.1 R6 |
| D2 | `<!DOCTYPE`/`<!ENTITY` 检查可被绕过，实体会被展开 | f26a/b/c |
| D3 | 没有行 / 列 / 单元格 / 文本 / 时间上限；内存按声明大小计 | §3、§8 |
| D4 | 超长数字串 (>4300 位) 让整个上传以英文消息失败 | `evidence/probe2.py`（函数级） |
| D5 | 共享串索引可以是负数并回绕到末尾 | `evidence/probe2.py` |
| D6 | `workbook.xml` / `sharedStrings.xml` 路径写死，`_rels/.rels` 不读；sharedStrings 在别的路径时所有共享串单元格被静默丢弃，整本工作簿读出 0 题且无错误（读代码得出，未做夹具） | `xlsx_parser.py:90,93-94` |
| D7 | 有数据但不在 A 列 / 只有表头的工作表被静默省略，用户得不到反馈 | f06、f17a |
| D8 | 没有有效 `r` 的行 / 单元格被静默丢弃 | f05 |
| D9 | 单个小项的多词答案因为回退正则被截断，报"不在选项里" | `parse_item_answers('1 Model Stage')` → `{1:'Model'}` |
| D10 | 热点最后一个小项后的续行被静默丢弃；续行归属于**下一个**小项 | f13 第 10 行；`pending` 读代码 |
| D11 | 选项字母到 H 为止；第 9 个选项被并入 H | f11 |
| D12 | 答案可从 **B 列**（翻译列）里取，行首写 `Answer:` 的翻译会被当成答案 | `xlsx_parser.py:306` |
| D13 | `canon` 不做 NFKC，全角 `Ｙｅｓ` 不等于 yes | 实测 |
| D14 | 日期 / 数字原样输出（日期是序列号），不看样式 | `xlsx_parser.py:76-77` |
| D15 | 错误消息把 Python 异常类名和文字直接返回给用户（内部信息泄露的轻微形式） | `server.py` 上传处理 `except Exception` |
| D16 | 同名工作表会让下游 `plan_import` 的 `by_sheet` 字典合并（Excel 本身禁止重名，仅防畸形文件） | f19；`db.py` 的 `by_sheet[s['name']]` |
| D17 | 两张表共用一个 `seen` 集合，id 后缀与表的顺序有关 | f18 |

## 10. 私有工作簿的结构统计（仅结构，无题目文字）

业主有两份真实的题库工作簿。它们的**结构统计**（工作表数、题型数量、是否用共享字符串、是否有富文本等，不含任何题目文字）只保留在业主本机，不放进仓库；结论是：真实数据只走到解析器的主路径，所以边界情况要靠 `tests/baseline/fixtures/` 里的合成夹具覆盖。

## 11. Rust 移植的建议（我的推荐，非事实）

1. **架构**：`parser/` 模块只暴露 `parse_upload(bytes) -> Result<Parsed, ParseError>`，内部分三层：zip 层（带上限）、XML/单元格层（流式、命名空间感知）、题目层（纯字符串函数）。题目层不依赖 XML，可以直接拿 §7 的 golden `reader` 输出作输入单测。
2. **P0 全部复制**：§4 的所有规则、§5 的 7 条文案、`make_id`、字段与键顺序、行号语义、工作表过滤与顺序。
3. **P1**：库异常文案无法复制，约定类别 + 前缀；对比脚本只比 `server_outcome.status` 与 `error` 前缀。
4. **P2（需 owner 批准）**：DOCTYPE 一律拒绝并拒绝 UTF-16；加入压缩比 / 单部件 / 总量 / 行数 / 单元格数 / 单元格文本长度 / 解析时间上限；`HINT_RE` 线性。
5. **整数**：明确每一处 `int()` 的接受范围；对 Unicode 数字的处理是**业务可见**的（热点答案 `１ 是`），建议复制（实现 Nd 数值表）而不是静默改变。
6. **XML**：先在 spike 里验证 `quick-xml` 的行为（行尾规范化、CDATA、注释、编码、命名空间解析、实体），再选库；"没有 sheet1.xml 假设"和"按 workbook 顺序"必须保持。
7. **对比流程**：`run_parser.py` 与未来的 Rust 二进制对同一批文件输出规范化 JSON，逐字段 diff；任何未解释的差异按规格视为缺陷。

## 12. 待确认问题（openQuestions）

1. 对 UTF-16 编码的 XML 部件，Rust 版是接受（先转码以保持 Python 行为）还是拒绝？owner 的文件从不出现，但 Python 当前接受（f22b）。
2. 对共享串负索引、数字串 >4300 位、行号 `r` 带空白或下划线这类畸形输入，是"保持 Python 行为"还是"一律视为畸形并跳过 / 拒绝"？（建议后者，并写入 migration 文档为已解释的差异。）
3. 热点答案的 Unicode 数字（全角 `１`）和单项多词答案回退（D9）：保持现状（兼容）还是顺手修复？修复会改变 API 行为，规格不允许悄悄修。
4. 解析器是否要在 Rust 版里加上行 / 列 / 文本上限？上限取多少？（owner 的真实文件：每表 25 行、3 列、总计 50 题，远低于任何合理上限。）
5. `check_xlsx` 的 20 MB 之外，Rust 版"实际 body 限制"与 Nginx 的 `client_max_body_size 20m` 如何分工，不在本维度，但与 §8 的声明大小问题相关。

## 13. 未能验证的内容（notVerified）

- 本机没有 Rust 工具链（`which rustc cargo` 无输出），所以所有关于 `regex` crate / `quick-xml` / `zip` crate 行为的陈述都来自文档与记忆，**没有在 Rust 里运行**（标注处已说明）。
- golden 在 Windows + Python 3.10.11 + expat 2.5.0 上生成；生产镜像是 Python 3.12-slim（Linux），Unicode 数据库版本、expat 版本、zipfile 的反斜杠处理可能不同；f20i 是平台相关的。
- 没有测试真实的压缩炸弹、十亿笑声实体、超大行数工作簿的内存和时间；只有 3000 行的夹具（f24）跑通。
- 没有构造加密成员 / 不支持的压缩方法 / CRC 错误的 zip，对应异常类型来自读代码。
- 没有构造完整的 Strict OOXML 工作簿；没有验证 XML 注释 / CDATA / 处理指令在 ElementTree 里的合并行为。
- "XML 1.0 不允许 U+001C-U+001F 出现在文本里"来自规范，没有用 expat 实测。
- 没有对 `web/app.js` 的完整消费路径做逐行确认，只确认了 `q.zh` 驱动中文切换和 `skipped` 的展示。
- 单一用户的 `Pending` 流程、数据库事务、HTTP 层限制属于其他维度，未在此审计。
