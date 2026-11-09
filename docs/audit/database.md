# Xexam Phase 0 审计草稿：SQLite 架构、查询、题目身份哈希与数据兼容（维度 db）

> 范围：只读审计。除 `（审计时的临时目录）` 外没有写任何文件；没有读取 `data/` 与 `progress.json`；没有启动服务器或浏览器；没有 git 写操作。
> 命名：文中旧名一律写作「the old name」。规格里的 `<old-name>.db` 与实际文件 `xexam.db` 不一致，这是一处**规格与现实的差异**，见 §13。
> 记号：**[事实]** = 读了代码或运行了命令；**[规格]** = 规格要求；**[建议]** = 我的建议。

## 0. 被依赖文件的 md5（工作树，CRLF 原样）

| 文件 | md5 | 备注 |
|---|---|---|
| `server.py` (477 行) | 48306e1f716d5dadeeedb63e805cebae | 工作树已修改（v0.2），与 HEAD 不同 |
| `app/db.py` (308 行) | 02846cbc21b21da42bb6f680f8638792 | LF 规范化后 fba28f9a69695b5fd817a2bed0c5e5f2 = 初始提交 1cb7512 = HEAD 8cec408 的 blob：**自初始提交以来未改动** |
| `app/xlsx_parser.py` (410 行) | 239b594ef8cbba5206e9a6f116178afe | LF 规范化后 6175cd02b44c6d55c8217234ee36525d = HEAD blob：未改动 |
| `VERSION` | 15085825464d9079e36d615be010435c | 内容 `0.2` |
| `docker-compose.yml` | 160023431bbe53035860a022124df9ae | TZ、卷 |
| `Dockerfile` | b031d35fad7d2bae6c2777be520e0399 | uid 10001 |
| `README.md` | 567104f5…（开始时）→ b1cc377271ee6ba1c97d36631b3ca14b（结束时） | **审计期间被别人改动**；我只引用其中的数据库说明 |
| `web/app.js` | 0dd0e1139626fe020f483ff03a02de07 | 只用于确认前端如何消费字段 |

运行环境：Python 3.10.11（本机），SQLite 3.40.1，Python `sqlite3` 模块 2.6.0。Docker 镜像基于 `python:3.12-slim`（Dockerfile:2），**镜像里的 Python/SQLite 版本我没有验证**（Docker 守护进程未运行，`docker images` 连接失败）。

---

## 1. 数据库文件与初始化

| 项 | 事实 | 出处 |
|---|---|---|
| 文件名 | `<data目录>/xexam.db`；Docker 内 `/data/xexam.db`（`XEXAM_DATA=/data`） | server.py:380, 393；Dockerfile:6 |
| 目录创建 | `os.makedirs(dirname(abspath(path)), exist_ok=True)` | db.py:86-87 |
| 连接 | `sqlite3.connect(path, check_same_thread=False, isolation_level=None)`（自动提交，事务手动控制）；`row_factory = sqlite3.Row` | db.py:89-90 |
| 连接数 | **每个进程一个连接**，所有线程共用，靠 `threading.RLock` 串行化 | db.py:88 |
| PRAGMA 执行顺序 | `journal_mode=DELETE` → `foreign_keys=ON` → `synchronous=FULL` → `executescript(SCHEMA)` → `meta('schema','1')`（`INSERT OR IGNORE`） | db.py:93-97 |
| 理由（注释） | 不用 WAL：Docker Desktop 挂载 Windows 文件夹时共享内存文件不可靠 | db.py:91-92 |

### 1.1 实测 PRAGMA（用真实 `Store` 新建数据库后查询；脚本 `dump_schema.py`，输出 `schema_dump.txt`）

| PRAGMA | Store 连接 | 另开一个普通连接看到的 | 是否持久在文件里 |
|---|---|---|---|
| `journal_mode` | `delete` | `delete` | 是（DELETE 是默认；WAL 才是持久的特殊模式） |
| `synchronous` | `2`（FULL） | `2` | **否**：每连接设置。碰巧默认也是 FULL，但 Rust 必须自己设 |
| `foreign_keys` | `1` | `0` | **否**：每连接设置 |
| `busy_timeout` | `5000` | `5000` | 否。Python `sqlite3.connect` 默认 `timeout=5.0`，代码没改 |
| `user_version` | `0` | `0` | 是，**从未使用** |
| `application_id` | 0 | | |
| `page_size` / `encoding` / `auto_vacuum` | 4096 / UTF-8 / 0 | | |
| `PRAGMA integrity_check` / `foreign_key_check` | `ok` / 空 | | |

- **[事实]** `foreign_keys=ON` 但**整个 schema 没有任何 FOREIGN KEY**（每张表 `PRAGMA foreign_key_list` 为空）。所以 `stats.question_id` 不引用 `questions.id`：下架题、未知 id 都能有 stats 行（实测：对不存在的 id 提交成绩会新建 stats 行）。这是有意的（「下架但保留学习记录」）；Rust 版**不要**加外键。
- **[事实]** 自动创建了 `sqlite_sequence` 表（因为 `attempts`、`imports` 用了 `AUTOINCREMENT`）。

### 1.2 schema 版本机制

- **[事实]** 版本只存在 `meta` 表：`('schema','1')`（`SCHEMA_VERSION = 1`，db.py:22；写入只在缺失时，db.py:97）。**没有任何地方读取或比较它**，`user_version` 恒为 0。实测把 `meta.schema` 改成 `99` 后 `Store` 照常打开，值保持 99。
- **[事实]** 没有迁移框架：升级 = `CREATE TABLE/INDEX IF NOT EXISTS`。给已有表加列/改列在现有代码里**不可能**自动发生。
- **[建议]** Rust 版若要改 schema，必须自己引入「读 `meta.schema` → 按版本执行迁移脚本 → 备份在先」的机制；不改 schema 则保持 `meta.schema='1'` 且不写 `user_version`。

---

## 2. 精确 schema（`sqlite_master` 原文，db.py:26-71）

```sql
CREATE TABLE meta (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL);
CREATE TABLE questions (
  id        TEXT PRIMARY KEY NOT NULL,
  sheet     TEXT NOT NULL,
  row_no    INTEGER NOT NULL,
  seq       INTEGER NOT NULL,
  data      TEXT NOT NULL,
  active    INTEGER NOT NULL DEFAULT 1,
  added_at  TEXT NOT NULL,
  updated_at TEXT NOT NULL);
CREATE INDEX idx_questions_sheet ON questions(sheet, active);
CREATE TABLE attempts (
  id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, sheets TEXT NOT NULL, mode TEXT NOT NULL,
  total INTEGER NOT NULL, correct INTEGER NOT NULL, percent INTEGER NOT NULL, seconds INTEGER NOT NULL);
CREATE TABLE stats (
  question_id TEXT PRIMARY KEY NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0, correct INTEGER NOT NULL DEFAULT 0,
  wrong INTEGER NOT NULL DEFAULT 0, streak INTEGER NOT NULL DEFAULT 0,
  last INTEGER, at TEXT);
CREATE TABLE imports (
  id INTEGER PRIMARY KEY AUTOINCREMENT, at TEXT NOT NULL, filename TEXT NOT NULL,
  added INTEGER NOT NULL, updated INTEGER NOT NULL, removed INTEGER NOT NULL,
  restored INTEGER NOT NULL, skipped INTEGER NOT NULL, total INTEGER NOT NULL);
```
（文件里的 CREATE 语句带中文注释；`sqlite_master.sql` 保存原文含注释。Rust 若用 `CREATE TABLE IF NOT EXISTS` 对已有库是空操作，不会改动。）

`sqlite_master` 顺序（rowid）：meta, sqlite_autoindex_meta_1, questions, sqlite_autoindex_questions_1, idx_questions_sheet, attempts, sqlite_sequence, stats, sqlite_autoindex_stats_1, imports。

| 表 | 列（cid, 名, 类型, notnull, 默认, pk） | 索引 |
|---|---|---|
| meta | 0 key TEXT 1 · pk1；1 value TEXT 1 | `sqlite_autoindex_meta_1`(key) UNIQUE（pk） |
| questions | 0 id TEXT 1 pk1；1 sheet TEXT 1；2 row_no INTEGER 1；3 seq INTEGER 1；4 data TEXT 1；5 active INTEGER 1 默认 `1`；6 added_at TEXT 1；7 updated_at TEXT 1 | `idx_questions_sheet`(sheet, active) 非唯一；`sqlite_autoindex_questions_1`(id) UNIQUE（pk） |
| attempts | 0 id INTEGER **notnull=0** pk1（rowid 别名 + AUTOINCREMENT）；1 at TEXT 1；2 sheets TEXT 1；3 mode TEXT 1；4 total/5 correct/6 percent/7 seconds INTEGER 1 | 无 |
| stats | 0 question_id TEXT 1 pk1；1 attempts/2 correct/3 wrong/4 streak INTEGER 1 默认 `0`；5 last INTEGER **可空**；6 at TEXT **可空** | `sqlite_autoindex_stats_1`(question_id) UNIQUE |
| imports | 0 id INTEGER pk1 AUTOINCREMENT；1 at TEXT；2 filename TEXT；3-8 added/updated/removed/restored/skipped/total INTEGER，全部 NOT NULL | 无 |

- **没有** CHECK 约束、触发器、视图、WITHOUT ROWID、外键。
- `idx_questions_sheet` 在代码里**没有任何查询用到**（所有 questions 查询要么全表扫描要么按 `seq` 排序）；保留它即可，丢掉也不影响行为。

---

## 3. 每一条 SQL（db.py 行号）

| # | 行 | 语句 | 目的 | 参数 |
|---|---|---|---|---|
| 1 | 93-95 | `PRAGMA journal_mode=DELETE` / `foreign_keys=ON` / `synchronous=FULL` | 连接设置 | 无 |
| 2 | 96 | `executescript(SCHEMA)` | 建表（幂等） | 无 |
| 3 | 103/107/110 | `BEGIN IMMEDIATE` / `ROLLBACK` / `COMMIT` | `tx()` | 无 |
| 4 | 115 | `INSERT OR IGNORE INTO meta(key,value) VALUES(?,?)` | 初始化 schema 版本 | key,value |
| 5 | 117 | `INSERT INTO meta … ON CONFLICT(key) DO UPDATE SET value=excluded.value` | 写 meta（目前无调用方使用此分支） | key,value |
| 6 | 121 | `SELECT value FROM meta WHERE key=?` | 读 meta（`legacy_migrated`） | key |
| 7 | 131 | `SELECT COUNT(*) c FROM questions WHERE active=1` | `/api/info.bankCount`、`--import` 汇总 | 无 |
| 8 | 136 | `SELECT id, sheet, row_no, data, active FROM questions ORDER BY seq` | `bank()`（GET /api/bank，确认导入返回体） | 无 |
| 9 | 160 | `SELECT id, sheet, row_no, data, active FROM questions` | `plan_import`（只读） | 无 |
| 10 | 203 | `SELECT COUNT(*) c FROM stats WHERE question_id IN (?,?,…)`（占位符个数 = 将下架的题数，仅拼接 `?`） | 预览里的 `removed_stats` | 下架 id 列表 |
| 11 | 214-218 | `INSERT INTO questions(id,sheet,row_no,seq,data,active,added_at,updated_at) VALUES(?,?,?,?,?,1,?,?) ON CONFLICT(id) DO UPDATE SET sheet=excluded.sheet, row_no=excluded.row_no, seq=excluded.seq, data=excluded.data, active=1, updated_at=excluded.updated_at` | `apply_import`：新增/更新/恢复（**不更新 `added_at`**） | id,sheet,row,seq,data(JSON),t,t |
| 12 | 221 | `SELECT id FROM questions WHERE active=1` | 找出文件里没有的题 | 无 |
| 13 | 223 | `UPDATE questions SET active=0, seq=?, updated_at=? WHERE id=?` | 下架（seq = 本次题数 + 序号） | seq,t,id |
| 14 | 224 | `INSERT INTO imports(at,filename,added,updated,removed,restored,skipped,total) VALUES(?,?,?,?,?,?,?,?)` | 导入记录 | 见 §6 |
| 15 | 230 | `SELECT * FROM imports ORDER BY id DESC LIMIT 1` | `last_import()`，供 `bank().file/mtime` | 无 |
| 16 | 235 | `SELECT * FROM imports ORDER BY id DESC LIMIT ?` | `import_history(12)`（GET /api/imports，server.py:259） | 12 |
| 17 | 241 | `SELECT * FROM stats`（**无 ORDER BY**） | `progress().questions` | 无 |
| 18 | 242 | `SELECT * FROM attempts ORDER BY id DESC LIMIT ?` | `progress().history` | 60 |
| 19 | 262 | `INSERT OR IGNORE INTO stats(question_id) VALUES(?)` | 确保行存在 | id |
| 20 | 264 | `UPDATE stats SET attempts=attempts+1, correct=correct+1, streak=streak+1, last=1, at=? WHERE question_id=?` | 答对 | t,id |
| 21 | 266 | `UPDATE stats SET attempts=attempts+1, wrong=wrong+1, streak=0, last=0, at=? WHERE question_id=?` | 答错 | t,id |
| 22 | 267 | `INSERT INTO attempts(at,sheets,mode,total,correct,percent,seconds) VALUES(?,?,?,?,?,?,?)` | 成绩历史 | 见 §7 |
| 23 | 271 | `DELETE FROM attempts WHERE id NOT IN (SELECT id FROM attempts ORDER BY id DESC LIMIT ?)` | 只留最近 60 场 | 60 |
| 24 | 276-277 | `DELETE FROM stats` / `DELETE FROM attempts` | `reset_progress` | 无 |
| 25 | 295 | `SELECT (SELECT COUNT(*) FROM stats) + (SELECT COUNT(*) FROM attempts) n` | 迁移前守卫 | 无 |
| 26 | 300 | `INSERT OR REPLACE INTO stats(question_id,attempts,correct,wrong,streak,last,at) VALUES(?,?,?,?,?,?,?)` | 迁移旧 progress.json | 见 §9 |
| 27 | 304 | `INSERT INTO attempts(at,sheets,mode,total,correct,percent,seconds) VALUES(?,?,?,?,?,?,?)` | 迁移历史 | 见 §9 |
| 28 | 307 | `INSERT INTO meta(key,value) VALUES('legacy_migrated', ?) ON CONFLICT(key) DO UPDATE SET value=excluded.value` | 置迁移标志 | now() |

- **[事实] SQL 注入面**：所有值都是参数；唯一的字符串拼接是 #10 的 `','.join('?'*n)`，只含问号。表名/列名全是字面量。
- **[事实] #10 的上限**：`IN (?,…)` 受 `SQLITE_MAX_VARIABLE_NUMBER` 限制。本机 SQLite 编译选项 `MAX_VARIABLE_NUMBER=32766`；实测一次下架 33000 题时 `plan_import` 抛 `OperationalError: too many SQL variables`。现实题库（所有者的两份各 25/50 题）远低于此，但 Rust 版应分块或改用子查询。
- **[事实] #8 的 `ORDER BY seq` 并列**：已下架的行保留旧 `seq`，可能与新上架行的 `seq` 相同（实测：`b05` 与 `a00` 同为 5，结果交错）。上架题的 `seq` 在一次导入内是 0..n-1 且唯一，所以**题目顺序是确定的**；并列只影响「已下架行」的相对顺序，进而影响 `bank().sheets` 中只含下架题的工作表的位置。Rust 若直接用同一条 SQL，行为一致；不要「顺手」加别的排序键后又声称行为不变。
- **[事实] #12 的顺序**决定下架行的新 `seq`（`base+i`）；`SELECT … WHERE active=1` 没有 ORDER BY，顺序由 SQLite 计划决定（不同版本可能不同）。只影响下架题之间的相对顺序，用户不可见。

---

## 4. 事务边界与并发

**[事实] `tx()`（db.py:100-110）**：

```python
with self.lock:                       # 进程内 RLock
    conn.execute('BEGIN IMMEDIATE')   # 取 RESERVED 锁
    try: yield conn
    except BaseException: conn.execute('ROLLBACK'); raise
    else: conn.execute('COMMIT')      # COMMIT 在 else 分支
```

| 操作 | 是否在 `tx()` 里 | 备注 |
|---|---|---|
| `apply_import` 的 upsert+下架+imports 行 | 是（一个事务） | 但 `plan_import` 在事务**之前**执行 |
| `record_attempt` 的 stats+attempts+剪枝 | 是（一个事务） | 返回前的 `progress()` 在事务之外 |
| `reset_progress` | 是 | 只删 stats、attempts |
| `migrate_legacy_progress` 的写入 | 是 | 守卫检查在事务之前（有 TOCTOU 窗口，单进程无害） |
| 所有 GET | 否（自动提交的单条 SELECT，持 `self.lock`） | `bank()` 里 `last_import()` 是另一次取锁，两次读之间可能插入一次导入 |
| `Store.__init__` 的 PRAGMA / executescript | 否 | |

### 4.1 实测发现的缺陷（真实代码，临时库，`probe_concurrency.py` → `concurrency_run.txt`）

| 编号 | 场景 | 结果 |
|---|---|---|
| T1 | 连接 A 持写事务 2 秒；连接 B 读 / 写 | B 读立即返回（0.000 s）且看不到 A 未提交的行；B 写等了 2.08 s 后成功 |
| T2 | A 持写事务 7 秒；B 写（默认 busy_timeout 5000） | B 在 5.54 s 后 `OperationalError: database is locked`；B 的连接 `in_transaction=False`（失败发生在 BEGIN，状态干净） |
| T3 | B 持有一个未读完的 SELECT（SHARED 锁）；A `record_attempt` 的 `COMMIT` 拿不到 EXCLUSIVE | 第一次调用 `database is locked`；**`A.conn.in_transaction` 仍为 True，没有 ROLLBACK**（COMMIT 在 `else` 分支，失败后异常不会走 `except`）。之后**每次**写都报 `cannot start a transaction within a transaction`，且 A 持着 RESERVED 锁，第三个进程连 `Store(p)` 都在 `PRAGMA journal_mode=DELETE` 上超时失败（7.43 s）。只有 `close()`（回滚）或进程重启才能恢复 |
| T4 | `apply_import` 先算 `plan_import`，再 `BEGIN IMMEDIATE`；两者之间另一连接改了库 | `imports` 表记录的 `added/updated/removed/restored` 是**事务前的旧计划**，与事务实际做的不符（实测：导入记录写 0/0/0/0，实际重新上架 5 题、下架 1 题）；同时返回给前端的 `result` 也是这份旧计划 |
| T5 | 文件被别的客户端留在 WAL 模式 → 用 `Store` 打开 | 打开后 `journal_mode=delete`（`PRAGMA journal_mode=DELETE` 会把它转回去）。**注意**：我没有在打开前单独打印确认文件确实处于 WAL，该项只算部分验证 |

- **说明**：T1-T4 用的是**同一 Python 进程内的两个独立 `sqlite3` 连接**（线程模拟）。SQLite 的文件锁语义对「两个连接」与「两个进程」相同，但我没有真的起两个 OS 进程。
- **触发 T3 的现实条件**：另一个进程持 SHARED 锁超过 5 秒（例如 `docker exec … python server.py --import` 与服务同时运行，或外部工具对库做长查询）。概率低，但后果是服务永久写失败直到重启。
- **[建议] Rust**：用 `rusqlite::Transaction`（Drop 即回滚）或在 COMMIT 失败时显式 ROLLBACK；`plan` 必须在同一写事务内（`BEGIN IMMEDIATE` 之后）重算，以修复 T4；明确连接所有权（一个专用的 DB 线程/`spawn_blocking` + `Mutex<Connection>`），因为 Python 版的 `RLock` 语义（进程内串行）要保留。
- **[事实] 两个进程同时打开同一文件**：SQLite 文件锁保证不损坏；读者不阻塞写者的 RESERVED 阶段，但会阻塞 COMMIT；`busy_timeout=5000` 之后报错。rusqlite 的默认忙等待同样是 5000 ms（我从 rusqlite master 的 `inner_connection.rs` 读到 `sqlite3_busy_timeout(db, 5000)`；未核对你将锁定的具体版本）。
- **[事实]** 日志模式 DELETE 会在**数据库所在目录**创建 `xexam.db-journal`，所以数据目录对运行用户必须可写（不只是文件）。容器里用户 uid 10001（Dockerfile:11）；Rust 镜像必须用同一 uid 或事先 `chown`，否则无法写日志。

---

## 5. `questions.data` 的 JSON（真实解析结果，`probe1.py`）

存入时：`json.dumps({k:v for k,v in q.items() if k not in ('id','sheet','row')}, ensure_ascii=False)`（db.py:213）。即 **UTF-8 原文，不转义非 ASCII**，键序为 Python 字典插入序（不排序）。`id`、`sheet`、`row` 不在 data 里：`id`=`questions.id`，`sheet`=`questions.sheet`，`row`=`questions.row_no`，读出时由 `bank()` 追加在最后（键序 `…, id, sheet, row`，db.py:146）。

| 键（存储顺序） | 类型 | 说明 |
|---|---|---|
| `kind` | string | `"single"` \| `"multi"` \| `"hotspot"` |
| `tag` | string | 热点题为 `"ホットスポット"`，否则 `""`（xlsx_parser.py:352） |
| `stem` | string | 题干（可含 `\n`） |
| `options` | array of `{key: "A".."H", text: string}` | 单选/多选；热点题为 `[]` |
| `items` | array of `{text: string, choices: [string,…]}` | 热点题的小项；选择题为 `[]` |
| `answer` | array | 选择题：**排序后的字母**数组 `["B"]`/`["A","C"]`；热点题：**每个小项选中 `choices` 的下标（整数）** `[0,1,0]` |
| `translation` | string | B 列的中文原文，无则 `""`（始终是 string，不会是 null） |
| `zh` | object \| null | 与日语结构对齐的中文：选择题 `{stem, options:[string…]}`；热点题 `{stem, items:[string…]}`；解析失败/对不上则 `null`（前端据此不显示中文切换，app.js:589） |
| `explanation` | string | 「解析」段，无则 `""` |
| `mnemonic` | string | 「口诀」段，无则 `""` |
| `note` | string | 答案括号里的备注，`；` 连接，无则 `""` |

- 无 float、无 null（除 `zh`）、无嵌套更深的结构；样例见 `golden_sample_outputs.json` 与 `sample.db`。
- 所有者两份题库的实测规模（仅统计，不含题目文字）：每题 data 平均约 3.2 KB，最大约 4.2 KB。
- **是否「变了」的判断**（`_q_hash`, db.py:78-80）：把行内容还原成 `{id, sheet, row, …data}` 后，去掉 `row`，`json.dumps(sort_keys=True, ensure_ascii=False)` 比较字符串。Rust 等价做法：比较解析后的 `serde_json::Value`（对象相等与键序无关），**忽略 `row`**，**包含 `id` 与 `sheet`**。注意整数 `1` 与字符串 `"1"` 不相等。
- **兼容提示**：Rust 新写入的 `data` 字节序列（键序/转义）可能与 Python 不同，但只要解析后相等，行为不变；对比测试应比较解析后的 JSON，不要比较字节。
- 前端实际依赖的字段：`id, sheet, kind, stem, options[].key/text, items[].text/choices, answer, translation, zh.stem/options/items, explanation, mnemonic, note`（app.js:142-156, 589-608, 785-830）；`tag`、`row` 前端未直接使用（`grep` 未发现 `q.tag`，未逐一确认 `q.row`，**未核实**）。

---

## 6. `plan_import`（只读）与 `apply_import`（写）

### 6.1 `plan_import(parsed_sheets, parsed_questions)`（db.py:157-196）

- 只做 #9 一次 SELECT（持 `self.lock`，不开事务），之后若有下架题再做 #10。**实测：调用后 questions/imports/stats/attempts 行数全为 0（预览不写业务数据）**。
- 分类规则（以 `id` 匹配）：

| 库里 | 文件里 | 结果 |
|---|---|---|
| 无此 id | 有 | `added` |
| 有且 `active=0` | 有 | `restored`（**不比较内容**，即使内容也变了也只算 restored） |
| 有且 `active=1`，`_q_hash` 不同 | 有 | `updated` |
| 有且 `active=1`，相同 | 有 | `unchanged` |
| 有且 `active=1` | 无 | `removed` |
| 有且 `active=0` | 无 | 不计 |

- 返回值键：`sheets[]`（每个 `{name,total,usable,skipped[],added,updated,removed,restored}`；先按解析出的工作表顺序，后追加只有下架题的工作表）、`added, updated, unchanged, restored, removed, skipped(=所有 skipped 条数), usable(=len(parsed_questions)), removed_samples(前 5 个下架题 stem[:40]), removed_stats`。`removed_stats` 是「有 stats 行的下架题**道数**」，不是答题次数。
- 切片 `[:40]` 按 **Unicode 码点**；Rust 用 `chars().take(40)`，不是字节。

### 6.2 `apply_import(filename, parsed_sheets, parsed_questions)`（db.py:205-226）

- 先 `plan_import`（事务外，见 T4），`t = now()`，然后**一个事务**：
  1. 对**每个**解析出的题（含「未变」的）执行 #11 upsert：`seq = 枚举序号`（0..n-1）；`row_no`、`sheet`、`data`、`updated_at` 一律被覆盖，`active` 置 1，`added_at` 保留。**实测：未变的题 `updated_at` 也被刷新，`seq` 被重写。**
  2. #12 取出当前 `active=1` 且不在 `new_ids` 的 id，逐个 #13 下架（`seq = n + i`）。
  3. #14 写 `imports` 一行（数字取自**旧 plan**，`total = len(parsed_questions)`）。
- **只触碰 `questions` 与 `imports`**；绝不碰 `stats`、`attempts`、`meta`、`sqlite_sequence` 以外的表（`imports` 的 AUTOINCREMENT 会推进 `sqlite_sequence`）。
- 空题库保护在 server.py:350-351（`not it['questions']` → 400），不在 `Store` 里；`Store.apply_import` 传空列表会把所有题下架。Rust 要把该守卫放在 service 层并保留。
- 实测流程 demo → demo_v2 → demo：第二次导入 `added 2, updated 1, removed 1`；回到 demo 时 `restored 1, removed 2, updated 1`（因为 v1 的某题 data 与库里的 v2 版本不同）。题目 `4ae830b3bd98` 下架后 stats 行保留，重新出现时 `active` 回到 1，学习记录自动回来。

### 6.3 `bank()`（db.py:133-155）响应形状

`{file, mtime, error:"", questions:[…], sheets:[{name, usable, inactive, total(=usable), skipped:[]}]}`。`file/mtime` 来自最后一条 `imports`（`filename`, `at`），库空则 `''`。`questions` 只含 `active=1`，按 `seq`。键序已实测。

---

## 7. 学习记录语义（`record_attempt` / `progress` / `reset_progress`）

### 7.1 输入（server.py:132-145 先清洗，再进 db）

`results` 最多 2000 条，且必须是 `{id:str, correct:bool}`，`id` 截 64 个**码点**；`sheets` 每项截 80、最多 50 项；`mode` 只能是 `all/weak/retry`，否则 `all`；`seconds` 夹到 0..604800；无有效结果 → 400。**这些限制决定了写进库的值，Rust 必须一致。**

### 7.2 `record_attempt`（db.py:255-272）

- `total = len(results)`，`good = 答对条数`，**`percent = round(100*good/total)`**。
- 对每条 result **按顺序**：`INSERT OR IGNORE` stats → 答对：`attempts+1, correct+1, streak+1, last=1, at=t`；答错：`attempts+1, wrong+1, streak=0, last=0, at=t`。**同一 id 在一次请求里出现多次会被依次累加三次**（实测 T,F,T → attempts 3, correct 2, wrong 1, streak 1）。
- **不校验 id 是否存在于 questions**（实测未知 id 会产生 stats 行）。
- 然后插入一行 attempts，再剪枝（#23）。全部同一事务；返回 `progress()`（事务外）。
- 直接调用 `record_attempt` 传空 results 会 `ZeroDivisionError`（API 层先拦掉，不可达）。
- **`percent` 的四舍五入是 Python 的银行家舍入（round-half-to-even）**：`1/8 → 12.5 → 12`，`3/8 → 37.5 → 38`，`1/40 → 2.5 → 2`，`1/200 → 0.5 → 0`。Rust 的 `f64::round()` 是「远离零」，会得到 13/38/3/1，**与 Python 不一致**。我验证了一个纯整数公式（`num=100*good; q,r=divmod(num,total); 若 2r>total 或 (2r==total 且 q 为奇数) 则 q+1`）与 Python `round(100*good/total)` 在 total ≤ 2000 的全部 2,003,000 组 (good,total) 上 **0 处不一致**；其中 2600 组是恰好 .5 的平局，1300 组「`round()` ≠ `floor(x+0.5)`」。**[建议]** Rust 用该整数公式，并把它写进测试。

### 7.3 `progress()`（db.py:239-253）响应

`{"version":1, "questions":{id:{attempts,correct,wrong,streak,[last:bool],[at:str]}}, "history":[{at,sheets(数组),mode,total,correct,percent,seconds}…最新在前, 最多 60], "masterStreak":2}`。

- `last` 仅当库里非 NULL 时输出，并转成 JSON 布尔；`at` 仅当真值（非 NULL 且非空串）时输出；记录内键序 `attempts, correct, wrong, streak, last, at`。
- `questions` 键序 = `SELECT * FROM stats` 的行扫描序（无 ORDER BY；实测为插入序）。前端按 id 取值，不依赖顺序。
- `history[].sheets` 是 `json.loads(attempts.sheets)`；`history` 条目**不含 `id`**。
- `masterStreak` = `MASTER_STREAK = 2`（db.py:23）。

### 7.4 错题本规则（谁实现）

**[事实]** 服务端不计算错题本。前端（app.js:107-112）`inNotebook(id) = stat 存在 && wrong>0 && streak < masterStreak`（`masterStreak` 来自 `/api/progress`，缺省 2）。所以 Rust 后端只需**精确维护 `wrong` 与 `streak` 并返回 `masterStreak:2`**。前端对 stats 里 `last` 的使用：在 app.js 里 `grep '\.last\b'` 没有命中（`last`/`at` 属于契约但目前前端未消费）。

### 7.5 历史剪枝与 id 增长（实测，`probe_semantics.py`）

- 连续 65 场后 attempts 表 60 行，`min(id)=6, max(id)=65`；`/api/progress.history` 长 60；**stats 不剪枝**（q0 的 attempts 计数 65）。
- `AUTOINCREMENT` 使 id 永不复用：`reset` 后再记一场，`attempts.id = 66`，`sqlite_sequence` 不被清零。

### 7.6 `reset_progress`（db.py:274-278）

删除：`stats` 全部、`attempts` 全部。保留：`questions`、`imports`、`meta`（含 `legacy_migrated`）、`sqlite_sequence`。返回空的 `progress()`（`questions:{}`，`history:[]`，`masterStreak:2`）。实测确认。

---

## 8. 时间戳与类型存储

| 项 | 事实 |
|---|---|
| 生成 | `datetime.now().strftime('%Y-%m-%d %H:%M')`（db.py:74-75）：**本地时区、无时区标记、精度到分钟** |
| 示例 | `2026-11-10 06:01` |
| 使用处 | `questions.added_at/updated_at`、`attempts.at`、`stats.at`、`imports.at`、`meta.legacy_migrated` 的值 |
| 时区来源 | 进程的本地时区。compose 设 `TZ: Asia/Tokyo`（docker-compose.yml:32）；Dockerfile 没有安装 tzdata，**基础镜像是否带 tzdata 我没验证**（若没有，`TZ=Asia/Tokyo` 会静默回落 UTC） |
| 可排序性 | 同一时区内字典序 = 时间序；**不唯一**（同一分钟内多条）；跨 DST 的时区会出现重复/倒退（东京无 DST）。历史排序用的是 `attempts.id`，不是 `at` |
| 前端 | 直接显示该字符串（app.js:356 `b.mtime`，396/399 历史，567-569 导入记录） |
| 迁移来的 `at` | 来自旧文件的任意字符串；`h.get('at','')` 可为空串 |

- **布尔**：不存布尔。`stats.last` 存 `1`/`0`/`NULL`（NULL = 从未答过或旧数据没有 `last`），`questions.active` 存 `0`/`1`。API 输出时 `last` 转 `true/false` 或省略。
- **整数**：SQLite INTEGER（64 位）；实测 `typeof(row_no/seq/active)='integer'`，`typeof(data)='text'`。
- **NULL**：只有 `stats.last`、`stats.at` 允许 NULL，其余全 NOT NULL。
- **文本编码**：库编码 UTF-8；JSON 用 `ensure_ascii=False`，所以日语/中文以原文存放，不是 `\uXXXX`。`attempts.sheets` 同样。
- **Rust 要小心**：Python 的切片/截断按码点；`id[:64]`、`stem[:40]`、`sheet[:80]`、`filename[:120]`（server.py:127）都要用 `chars()`。

---

## 9. `migrate_legacy_progress(path)`（db.py:281-308）

- **输入格式**：UTF-8 JSON 对象，必须有 `questions`（dict）和 `history`（list）。`questions[id] = {attempts, correct, wrong, streak, last?, at?}`；`history`（**最新在前**）元素 `{at, sheets, mode, total, correct, percent, seconds?}`。
- **守卫顺序**：① meta `legacy_migrated` 已有 → 不迁移；② 文件不存在 → 不迁移；③ 读/解析失败（`OSError, ValueError, KeyError, AssertionError`）→ 返回说明；④ stats+attempts 行数 > 0 → 不覆盖；⑤ 一个事务：`INSERT OR REPLACE` stats，`reversed(history)` 逐条插入 attempts（所以 id 递增 = 时间从旧到新），最后写 `legacy_migrated=now()`。
- `last` 转换：`(1 if r.get('last') else 0) if 'last' in r else None`（键存在但值为 null 也会变成 0）。数值用 `int(…)`（Python 的 `int` 接受 `"5"`、`3.7`、`True`）。
- **实测（合成 progress.json，不是所有者的文件）**：
  - 全新库：迁移成功，历史 id 1,2 按时间升序，meta 写入标志；第二次调用返回「之前已经迁移过」；reset 后第三次仍为已迁移（标志在）。
  - **幂等性缺口**：库里已有学习记录时返回「没有覆盖」且**不设标志**；之后用户 `reset` 清空记录，下次服务启动 `open_store` 又会调用迁移，**旧 progress.json 会被重新导入（复活）**。实测。
  - **启动崩溃风险**：畸形但可解析的文件（`questions` 里的值不是 dict；history 缺 `total`；`attempts` 为非数字字符串）会让 `migrate_legacy_progress` 抛 `AttributeError/KeyError/ValueError`，而 `server.open_store`（server.py:382-387）没有捕获 → 服务启动失败。
  - UTF-8 BOM 文件 → 读失败（用的是 `encoding='utf-8'`，不是 `utf-8-sig`），返回「读不了」。
  - 缺 `questions`/`history`、类型不对的文件 → 返回说明且不抛异常。
- `open_store`（server.py:378-388）按 `data/progress.json`、`<程序目录>/progress.json` 的顺序找，找到**第一个存在的文件**就尝试并 `break`（不管结果）。Docker 镜像的 `.dockerignore` 排除了 `progress.json`。
- **[建议]** Rust 版**不要**自动运行此迁移；改成显式的一次性 CLI 子命令（带 `--dry-run`），并在「库里已有记录」时也设标志或要求显式 `--force`，避免复活。是否保留这个功能是**待所有者决定**（§13）。

---

## 10. 题目身份（`make_id`）——最关键

### 10.1 逐步算法（`xlsx_parser.py:245-247`，经 `parse_question` 在 :351 调用，去重在 :401-405）

```python
def make_id(sheet, stem, first):
    raw = sheet + '\n' + re.sub(r'\s+', '', stem)[:300] + '|' + re.sub(r'\s+', '', first)[:200]
    return hashlib.sha1(raw.encode('utf-8')).hexdigest()[:12]
```

1. `sheet`：工作表名，**原样**（不 strip、不规范化、区分大小写；可含任意字符，包括换行）。
2. `stem`：**解析后的题干**（不是整格文本）：`parse_options`/`parse_hotspot` 之后的 `stem`，已 `_norm_nl`（`\r\n`、`\r` → `\n`，`strip()`）并按解析规则截掉选项/小项部分。热点题的首行 `ホットスポット` 已被去掉。
3. `first`：**第一个选项的文本**（选择题 `options[0]['text']`，多行用 `\n` 连接）；热点题则是 `items[0]['text']`（xlsx_parser.py:326, 341）。
4. 对 `stem`、`first` 各自 `re.sub(r'\s+', '', …)`：删除**所有** Unicode 空白（见 10.2），**不是**折叠成一个空格，是直接删除。
5. 然后按**码点**截断：`stem'[:300]`、`first'[:200]`（先删空白、后截断）。
6. 拼接 `sheet + "\n" + stem' + "|" + first'`（字面量 LF 与竖线）。
7. UTF-8 编码 → SHA-1 → 十六进制小写 → 取前 **12** 位（48 位）。
8. **去重后缀**（`parse_workbook`，xlsx_parser.py:401-405）：`seen` 集合**整个工作簿共用**；若 `id in seen`：`id = base-2`，仍冲突则 `-3`……。顺序 = 解析顺序（工作表按 workbook.xml 顺序，行按 sheet XML **文档顺序**，不是按行号排序）。
9. **不做**的事（已 `grep` 验证全仓库没有 `unicodedata/casefold/normalize/NFKC/NFC`）：无 Unicode 规范化、无大小写折叠、无全半角折叠、无标点剥离。`str.lower()` 只用在答案比较 `canon()`（:118）和首行判断（:293），**不在 id 里**。

### 10.2 Python 特有行为清单（Rust 移植必须复刻）

| # | Python 行为 | 影响与 Rust 对策 |
|---|---|---|
| 1 | `re.sub(r'\s+','',…)` 的 `\s`（str 模式，Unicode）= **29 个码点**：U+0009–000D, **U+001C–001F**, U+0020, **U+0085**, U+00A0, U+1680, U+2000–200A, U+2028, U+2029, U+202F, U+205F, U+3000。我枚举了 0..0x10FFFF：`re` 的 `\s` 与 `str.isspace()` 完全相同（29 个）；而 Unicode `White_Space` 属性只有 25 个，**少了 U+001C–001F** | Rust 的 `char::is_whitespace` 就是 `White_Space`（25 个）。**必须自定义谓词**：`c.is_whitespace() \|\| ('\x1c'..='\x1f').contains(&c)`。U+001C–1F 经 XLSX 不可达（XML 1.0 禁止，实测 Python 解析器拒绝），但保持一致更安全。`regex` crate 的 `\s` 行为我**没有在 Rust 里验证**（本机没有 rustc） |
| 2 | **不是**空白的「像空白」字符：U+200B ZWSP、U+FEFF、U+180E、U+2800、U+3164、U+FFA0、U+00AD、U+200C/D、U+2060 全部**保留**（实测） | Rust 不要把它们当空白（`is_whitespace` 同样不含，正确） |
| 3 | 先删空白，**后**截断 | 顺序不能反 |
| 4 | 截断按**码点**（`str[:n]`） | Rust：`chars().take(n)`；**不是**字节切片，也不是 UTF-16 单元。向量里有非 BMP（𠮷、🍎）的 305/205 个字符用例 |
| 5 | 无规范化：NFC `é` 与 NFD `e+U+0301` 得到**不同** id；半角片假名与全角不同；全角 `？` 与 `?` 不同；全角数字/字母保留 | **不要**「顺手」做 NFKC。所有者的题库：25 题里 22 题、50 题里 47 题含全角 ASCII 形式（U+FF01–FF5E），NFKC 会改变它们，**若加 NFKC，绝大多数现有 id 会变，错题本全部失联** |
| 6 | 大小写保留（`AWS` ≠ `aws`） | 不要 `to_lowercase` |
| 7 | 分隔符 `\n` 和 `\|` 是字面量，没有转义：`stem="a\|b", first="c"` 与 `stem="a", first="b\|c"` 得到**同一个** id（已有向量） | 不要「修复」。若修复必须同时迁移全部 id（见 §11） |
| 8 | `sheet` 里可以有换行/竖线，同样不转义；空白**不**从 sheet 删除 | 原样拼接 |
| 9 | `hashlib.sha1(...).hexdigest()[:12]` | `sha1` crate，十六进制小写，取前 12 个字符 |
| 10 | 去重后缀依赖**行的文档顺序**；重复题互换位置 → 后缀跟着位置走，**学习记录会换到另一道内容不同的题上**；删掉第一份 → 第二份继承无后缀 id（向量 `p_dup_reordered`、`p_dup_removed_first`） | 保持同样的顺序语义（XML 文档顺序，非按行号排序） |
| 11 | `id` 由**解析结果**决定（`stem`、`options[0].text`、`items[0].text`），不是原始单元格 | Rust 的 XLSX 解析/题干切分必须与 Python **逐字符一致**才能得到相同 id。这是 id 兼容性对「parser 维度」的硬依赖，必须用 §10.3 的 pipeline 向量做回归 |
| 12 | Python `str.strip()` / `str.split('\n')` / `re` 的 `\s` 在解析里到处用 | 与 #1 同一个空白集（`strip()` 也剥 U+001C–1F、U+0085） |
| 13 | `datetime.now()` 本地时间（§8） | 与 id 无关，但与 `at` 字符串有关 |

### 10.3 测试向量（`vectors.json`，`gen_vectors.py` 重新生成）

- **A 类：54 个直接 `make_id(sheet, stem, first)` 向量**，每个带 `hash_input_utf8_hex`（被哈希的确切字节）。覆盖：日语、全角/半角（数字、字母、`？`、片假名）、NFC/NFD（平假名浊点、`é`、工作表名）、末尾空格/换行、制表符、U+3000、NBSP、U+2003、U+0085、U+001C、U+2028、CRLF、ZWSP/BOM 保留、大小写、`ß`、`İ`、非 BMP（𠮷、🍎）、空 sheet/stem/first、纯空白、`|` 碰撞、300/301/360 码点截断、先删空白再截断、200/201 的 first 截断、空选项。脚本里每个向量都与独立的 `sha1` 重算断言一致。
- **B 类：12 个 pipeline 用例（31 个 id）**：构造 xlsx → 真实 `parse_workbook` → id。覆盖：重排行 id 不变（已断言）、同题干在两个工作表（id 不同）、单工作表内 3 份重复（`base`、`base-2`、`base-3`）、重复重排、跨工作表重复、删掉第一份、全角数字、单元格内 U+3000/制表符/CRLF、选项末尾空格、NFD 题干、日语工作表名。
- 合计 **85 个 id 断言**（≥ 60）。`vectors.json` 还包含 Python `\s` 的 29 个码点列表。
- 已核对的关系：末尾空格/换行 = 无；CRLF = LF；制表符 = U+3000；第 301 个字符被忽略；300 之后的改动不影响 id；纯空白 first = 空 first；重排行后 id 集合相同。

### 10.4 所有者真实题库的统计（只读，仅计数；`explore_workbooks.py` → `workbook_stats.txt`）

| 项 | 工作簿 1 | 工作簿 2 |
|---|---|---|
| 工作表 / 可用题 | 2 / 25（第 2 个表 25 行全部「没找到答案」被跳过） | 2 / 50 |
| 单选 / 多选 / 热点 | 12 / 5 / 8 | 20 / 22 / 8 |
| `zh` 非 null | 25 | 50 |
| 带 `-N` 去重后缀的 id | 0 | 0 |
| 题干去空白后最大长度 | 292（<300） | 292 |
| 首选项去空白后最大长度 | 82（<200） | 85 |
| 非 BMP / 组合记号 / 非常规空白 | 0 / 0 / 无 | 0 / 0 / 无 |
| 含全角 ASCII 形式 | 22 | 47 |
| 与独立重算的 id 公式一致 | 是 | 是 |
| 工作簿 1 的 id 出现在工作簿 2 | 25/25 | — |

**含义**：现有数据没有触发去重后缀与截断；所以「后缀稳定性」「300/200 截断」的风险目前是潜在的，但向量必须覆盖。全角 ASCII 的普遍存在说明**任何 NFKC 类的「改进」都会破坏现有 id**。

---

## 11. 迁移风险（数据）

| 风险 | 级别 | 说明 / 建议 |
|---|---|---|
| id 算法在 Rust 里细微不同 | **高** | 用 `vectors.json` 做 CI 门禁；先在副本上对 `sample.db` 与（所有者授权后）真实库做「重新解析 → id 集合」比较 |
| 空白集合（U+001C–1F）、舍入（银行家）、码点截断 | 高 | 见 §7.2、§10.2 |
| 现有 DB 文件名 | **高（待确认）** | 见 §13 第 1 条：v0.1（HEAD）打开的是以旧名命名的文件，v0.2 工作树打开 `xexam.db`，**代码里没有改名/复制逻辑**。所有者的 `data/` 里实际是哪个文件，我按规则不能查看 |
| `Store` 里隐含的行为（T3/T4 缺陷） | 中 | Rust 不要照搬缺陷，但要保留「预览不写、确认单事务、不重复应用」；差异必须文档化为「已修复的缺陷」，不能静默变化 |
| `meta.schema` 从未检查 | 中 | 一旦改 schema 必须自带版本检查 |
| 时间戳本地时区、分钟精度 | 中 | 保留格式；容器里的 tz 数据必须存在，否则 `at` 会从东京时间变成 UTC |
| journal_mode / synchronous | 中 | Rust 连接必须显式执行 `journal_mode=DELETE`（rusqlite 的 `execute` 对返回行的 PRAGMA 会报错，用 `pragma_update`/`query_row`）、`synchronous=FULL`、`foreign_keys` 可保持 ON（无外键，无实际影响）、`busy_timeout=5000` |
| 数据目录权限 | 中 | DELETE 日志要在目录里创建文件；uid 10001 |
| `percent` 舍入 | 中 | 见 §7.2 |
| 复活旧 progress.json | 低-中 | 见 §9 |
| `IN (?…)` 超过 32766 | 低 | 见 §3 |
| `ORDER BY seq` 并列 | 低 | 见 §3 |

### 11.1 与另一个 SQLite 客户端的兼容（回答「Python 写的库能否原样被别的客户端打开」）

**[事实] 能。** 依据：
1. 全部列声明为 `TEXT`/`INTEGER`；实测 `typeof` 与声明一致（integer / text）。
2. `grep register_adapter|register_converter|detect_types|PARSE_DECL|pickle|marshal|memoryview|BLOB|bytes(` 在 `app/db.py`、`app/xlsx_parser.py` 里**零命中**（`server.py` 里唯一的 `bytes` 用途是上传内容检查，不进库）。没有 BLOB，没有 pickle，没有 Python 专有适配器。
3. JSON 列（`questions.data`、`attempts.sheets`）是合法 UTF-8 JSON 文本。
4. `sqlite3.Row` 是客户端类，不影响文件。
5. 文件使用 DELETE 日志（无 `-wal/-shm`）、`page_size=4096`、UTF-8；`user_version=0`；`application_id=0`。
6. 一个**普通新连接**看到 `foreign_keys=0`，但 schema 里没有外键，所以无影响。
7. 用到的 SQL 特性：UPSERT（`ON CONFLICT DO UPDATE`，SQLite ≥ 3.24）、`AUTOINCREMENT`。Rust 内置的 SQLite 版本更新，没问题。
8. `sample.db` 由真实代码生成并用 `sqlite3` 以只读 URI 重新打开读取成功。

---

## 12. 产物清单（均在 `（审计时的临时目录）`）

| 文件 | 说明 | md5 |
|---|---|---|
| `db-draft.md` | 本文 | — |
| `vectors.json` | 54 直接 + 12 pipeline（31 id）= 85 个 id 断言 | 4ee14eba0c532149798cbe11510b33b5 |
| `gen_vectors.py` | 重新生成向量（`python -I -B gen_vectors.py [repo] [out]`） | 8752f5d9fbf4a7407cdf3cfdb04a8ce0 |
| `sample.db` | 真实 `Store` + 虚构的 demo.xlsx / demo_v2.xlsx 生成：7 题（6 上架 1 下架）、5 条 stats（含一道仍在错题本里的题）、3 场成绩、2 条导入、meta schema=1；**时间戳用固定假时钟**（`db.now` 打补丁，保证可复现）；`integrity_check` = ok；DELETE 日志、无遗留 `-journal` | cf6610dfdf78d774ac9bfa5bf1d04bfe |
| `make_sample_db.py` | 生成 sample.db | ad34b5d43ed81d336cd7bcf94aa8aecc |
| `golden_sample_outputs.json` | 对 `sample.db` 的**副本**调用真实 `bank()/progress()/import_history()/plan_import(demo.xlsx)` 的输出，用作新旧对比基准 | 3806c18d1739efd396501dc290ad560a |
| `dump_schema.py` / `schema_dump.txt` | schema 与 PRAGMA 转储 | cf00383d… |
| `probe_semantics.py` / `semantics_run.txt` | 行为探测（舍入、stats、剪枝、导入、seq 并列、迁移、变量上限、重开） | fcf833c5… |
| `probe_concurrency.py` / `concurrency_run.txt` | 并发探测 T1-T5 | ef1c923a… |
| `explore_workbooks.py` / `workbook_stats.txt` | 所有者题库的只读统计（只含计数） | 97547dae… |
| `work/` | 探测用的临时库 | — |

---

## 13. 开放问题与差异（需要所有者/协调者回答）

1. **规格 vs 现实（数据库文件名）**：规格写 `data/<old-name>.db`，实际文件是 `xexam.db`（server.py:380）。更进一步：Git 历史显示最后一次推送（HEAD 8cec408）以及更早的所有提交打开的是以旧名命名的文件；工作树（v0.2）改成了 `xexam.db`，**代码中没有重命名/复制/回退逻辑**（我在 server.py、db.py 里搜索过）。README 的 v0.2 更新日志只写「数据库文件 xexam.db」，没有给已有数据的改名步骤。因此：**如果所有者在本机 `data/` 里的库仍是旧名，v0.2 会无声地创建一个新的空 `xexam.db`**。我不能查看 `data/`，请所有者确认实际文件名，并决定是在迁移期做一次受控的改名/复制，还是 Rust 版启动时检测旧名文件并**拒绝启动并提示**（而不是静默新建）。
2. 环境变量也已改名（server.py:393 只读 `XEXAM_DATA`），旧变量不再生效。
3. 所有者的真实库里：`meta.legacy_migrated` 是否已设？是否有孤儿 stats（下架或不存在的题）？`sqlite_sequence` 的值？这些我没有检查（规则 2）。**[建议]** 由所有者在副本上运行只读脚本（我可以提供）输出聚合数字。
4. 是否要保留 `progress.json` 自动迁移（及其「复活」行为）？建议 Rust 版只保留为显式的一次性命令。
5. `Store` 中发现的缺陷（T3 提交失败后事务悬挂；T4 计划在事务外；`record_attempt` 不校验 id；`migrate` 可能让启动崩溃；`apply_import` 本身不拒绝空题库）在 Rust 版里是「修复并记录」还是「逐字保持」？我的建议是修复 T3/T4 与启动崩溃，其余保持，并全部写进迁移文档。
6. 去重后缀的不稳定（重复题重排会让学习记录换题）是否接受？这是现状，建议保持并在文档中写明，不要在迁移中悄悄改变。
7. 容器时区：Rust 镜像是否包含 tzdata，并继续用 `TZ=Asia/Tokyo` 写本地分钟字符串？还是改写 UTC？（后者会改变既有数据的含义，我建议保持本地时间格式。）

## 14. 未验证项（notVerified）

- 本机没有 `rustc/cargo`：**没有运行任何 Rust 代码**；关于 `regex` crate `\s`、`char::is_whitespace` 的说法来自文档知识，未在 Rust 里验证。
- Docker 守护进程未运行：镜像内的 Python 3.12、SQLite 版本、tzdata、uid 10001 与卷权限均未验证。所有实测在 Python 3.10.11 + SQLite 3.40.1 上完成（`\s` 码点集合与 `str.isspace` 在 3.12 上应相同，但我没有在 3.12 上运行）。
- 并发测试是同一进程内的多连接，没有起真正的多个 OS 进程；Windows 文件锁与 Docker 绑定挂载上的行为未验证。
- T5（WAL → DELETE）没有在打开前单独确认文件处于 WAL。
- 没有检查所有者真实数据库与 `progress.json`（规则 2）。
- 前端是否使用 `q.tag`/`q.row` 未逐一核实。
- 审计期间 README 被他人修改（md5 变化）；`server.py`、`app/db.py`、`app/xlsx_parser.py` 在整个期间 md5 未变。
- 工作簿统计仅限计数，没有核对解析结果的正确性（属解析器维度）。
