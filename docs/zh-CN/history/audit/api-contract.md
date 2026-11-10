# Xexam Phase 0 草稿：HTTP API 契约与黄金响应（维度 api）

范围：只读审计 `server.py` 的完整 HTTP 行为，并用真实 `server.py` 抓取黄金响应。
引用格式 `file:line` 指向审计时读到的源码；"捕获 NNN" 指 `golden/NNN_*.json`。
所有断言要么来自逐行读代码（带行号），要么来自实际运行（捕获文件 / 命令结果）。

## 0. 版本与校验和（捕获时刻）

| 文件 | md5 | 行数 |
|---|---|---|
| server.py | 48306e1f716d5dadeeedb63e805cebae | 477 |
| app/db.py | 02846cbc21b21da42bb6f680f8638792 | 308 |
| app/xlsx_parser.py | 239b594ef8cbba5206e9a6f116178afe | 410 |
| web/app.js | 0dd0e1139626fe020f483ff03a02de07 | 865 |

- 读文件前、抓取前、抓取后三次核对，md5 一致（`golden/index.json` 也记录了它们）。
- VERSION 文件内容为 `0.2`；`/api/info` 返回 `"version":"0.2"`。
- 运行环境：Windows 11，Python 3.10.11（`Server: Xexam/0.2 Python/3.10.11`）。
  线上镜像是 `python:3.12-slim`（Dockerfile:2），Linux。**差异未验证**，见第 12 节。
- 规格里数据库文件写作 `<old-name>.db`；真实文件是 `xexam.db`（server.py:380）。这是与规格的一处不一致，已按要求报告，应以 `xexam.db` 为准。

## 1. 线路层（所有响应共有）

| 项 | 事实 | 依据 |
|---|---|---|
| HTTP 版本 | 响应首行 `HTTP/1.0`。`BaseHTTPRequestHandler.protocol_version` 默认 `HTTP/1.0`，server.py 没有覆盖 | 进程内检查 `Handler.protocol_version == 'HTTP/1.0'`；全部捕获的 `http_version` |
| keep-alive | 没有。HTTP/1.0，每个请求一个连接，响应后关闭。正常响应不带 `Connection` 头；501 错误页带 `Connection: close` | 捕获 033 的 headers |
| 线程模型 | `ThreadingHTTPServer`，一个连接一个线程；`daemon_threads=True`，`request_queue_size=5`，Handler.timeout=None（无读超时） | server.py:426；进程内检查 |
| `Server` 头 | `Xexam/0.2 Python/3.10.11`（`server_version='Xexam/'+VERSION`，server.py:195，加上标准库的 sys_version） | 捕获 001 |
| `Date` 头 | 标准库自动加（易变） | 捕获 |
| `_send` 固定加的头 | `Content-Type`、`Content-Length`、`Cache-Control: no-store`、`X-Content-Type-Options: nosniff` | server.py:200-206；捕获 |
| 没有的头 | `X-Frame-Options`、CSP、`Referrer-Policy`、HSTS、任何 `Access-Control-*`（无 CORS 支持，OPTIONS 直接 501）、`Allow` | grep 全文无；捕获 039 |
| JSON 编码 | `json.dumps(obj, ensure_ascii=False).encode('utf-8')`：默认分隔符 `", "` 和 `": "`（有空格），不缩进，键顺序 = Python dict 插入顺序，非 ASCII 原样输出（UTF-8）。`Content-Type: application/json; charset=utf-8` | server.py:210-211, 187-191；捕获 |
| 文本 403/404 | `text/plain; charset=utf-8` | server.py:247-248, 267 |
| 日志 | `log_message` 被改成空操作：无访问日志 | server.py:197-198 |
| 访问来源头 | `X-Forwarded-*`、`Forwarded`、`X-Real-IP` 从不读取；客户端 IP 从不使用 | 全文读过，无引用 |
| 客户端 IP / 会话 | 不存在。没有登录、没有 cookie、没有 session | 全文读过 |

## 2. 守卫（guard）的执行顺序

### GET / HEAD（server.py:236-270）

1. `do_HEAD` 设 `_head_only=True` 后直接调用 `do_GET`（响应头与 GET 完全相同，包括真实 `Content-Length`，只是不写正文；捕获 006：`content-length: 73`，`body_len: 0`）。
2. Host 检查 `_host_ok()`（245）。失败 -> 403 `text/plain`（见 3.1）。**GET 不检查 Origin。**
3. `path = urlparse(self.path).path`（249）：查询串和 `#fragment` 被丢弃（捕获 008、031）。
4. 精确匹配 `/api/info`、`/api/bank`、`/api/progress`、`/api/imports`（251-259）。没有尾斜杠兼容：`/api/bank/` 走到静态文件分支 -> 404 文本（捕获 009）。
5. 其余一律按静态文件处理（见第 6 节）；包括 `/api/upload`、`/api/shutdown`、`/api/nope`，它们 GET 得到 404 `not found` 文本（捕获 010-012，**不是** JSON、也不是 405）。

### POST（server.py:308-373）

1. `_deny_reason()`（309）：先 Host，后 Origin。任何一项失败 -> **403 JSON** `{"error": "<中文原因>"}`。
   - Host 先于 Origin：两者都坏时报 Host（捕获 055）。
   - 这一步在路径、Content-Type、大小、JSON 解析之前（捕获 069：upload 用 `text/plain` + 恶意 Origin -> 403 而非 415）。
2. 路径 `urlparse(self.path).path`（312）。
3. 若路径 == `/api/upload`（316）：
   1. Content-Type 必须"包含"子串 `application/octet-stream`（317，大小写敏感的子串匹配，不是解析）。否则 415 `{"error":"bad content type"}`。
   2. `_body(MAX_UPLOAD=20*1024*1024)`（319）。超限 -> 413（见 3.6）。
   3. `clean_filename(X-Filename)`（322）。
   4. `check_xlsx(body)` 然后 `parse_workbook`（324-325）：`ValueError` -> 400 中文消息；其他任何 `Exception` -> 400 `读取这个 Excel 时出错：<异常类名>: <异常文本>`（328-329）。
   5. `plan_import` + `Pending.put` -> 200 预览。
4. 其他所有 POST 路径（包括未知路径）先要求 Content-Type 子串含 `application/json`（334），否则 415。**因此未知路径 + 错误类型得到 415 而非 404**（捕获 074）。
5. `_body(MAX_JSON=1_000_000)`（336）。超限 -> 413 `{"error":"too large"}`。
6. `json.loads(raw or b'{}')`（340）：捕获 `ValueError`（含 `UnicodeDecodeError`）-> 400 `{"error":"bad json"}`；解析结果不是 dict -> 400 同样消息（343-344）。空正文 = `{}`（捕获 081）。
7. 按路径分发：`/api/import/confirm`、`/api/import/cancel`、`/api/attempt`、`/api/reset`、`/api/shutdown`；否则 404 `{"error":"not found"}`（373；捕获 075-076）。`POST /api/info` 也是这个 404。

### 其他方法

`BaseHTTPRequestHandler.handle_one_request` 在没有 `do_<METHOD>` 时直接 `send_error(501)`。PUT / DELETE / OPTIONS / PATCH / TRACE / CONNECT 全部得到 **501** 和标准库的 HTML 错误页（`Content-Type: text/html;charset=utf-8`，注意没有空格；`Connection: close`；**不带** `Cache-Control` / `nosniff`）；因为这一步发生在 `do_*` 之前，**Host 守卫不适用**（代码事实；用恶意 Host 测 501 未抓取）。CORS 预检（OPTIONS + `Access-Control-Request-Method`）同样 501 且无 CORS 头（捕获 033-039）。正文是英文 HTML，含 `Error code: 501` 和 `Message: Unsupported method ('PUT').`。

## 3. 逐端点契约

### 3.1 Host / Origin 规则（默认配置：只允许本机）

- 允许的 Host 名集合 = `{localhost, 127.0.0.1, ::1} ∪ XEXAM_ALLOWED_HOSTS`（437-440）。`XEXAM_ALLOW_ANY_HOST` 为 `1/true/yes/on` 时 Host 不再检查（435、218-219）。
- `host_name()`（88-96）：去空白、转小写、去端口（`rsplit(':',1)`）、`[ipv6]` 取方括号内、去末尾的点。**坑**：`localhost:1:2` 变成 `localhost:1`，不在集合里 -> 403（捕获 050 证实）。`LOCALHOST.` 通过（046）。Host 头缺失或为空 -> 403，消息里主机名为空（043-044）。
- 允许的 Origin 规则（222-234）：没有 Origin 头 -> 放行；`origin_host()`（99-107）只接受 `http://` / `https://`（大小写不敏感），含 `@` 或为空 -> `None` -> 拒绝；否则**只比较主机名**：等于 Host 头的主机名，或在 `allowed_origins` 里。协议和端口不比较（捕获 063-066：不同端口、`HTTPS`、大写主机、带路径都放行）。`allowed_origins = XEXAM_ALLOWED_ORIGINS ∪ XEXAM_ALLOWED_HOSTS`（440）。
- 403 的消息会回显请求里的 Host / Origin：去控制字符（`[\x00-\x1f\x7f]`），截断到 80 个字符（246、294-295）。GET 的 403 是 `text/plain`，格式 `Forbidden host: <名字>。通过别的地址访问……`（247-248）；POST 的 403 是 JSON `{"error": "..."}`。
- 三种 POST 403 消息（捕获 054、056、057-062）：
  - Host 不在列表：`被拒绝：这个请求的 Host 是「<原始Host头,<=80>」，不在允许的列表里。请把它加进 XEXAM_ALLOWED_HOSTS（或设 XEXAM_ALLOW_ANY_HOST=1，只在前面有身份验证时）。`
  - Origin 不是 http(s) 网址：`被拒绝：这个请求的 Origin 是「<值>」，不是一个网址。`（含 `null`、`file://`、`ftp://`、带 `@`、空串、`https://`）
  - Origin 主机与 Host 不同：`被拒绝：这个请求来自「<origin主机>」，和服务看到的 Host「<host主机>」不是同一个主机。如果你在 Cloudflare Tunnel 等反向代理后面且代理改写了 Host，请把「<origin主机>」加进 XEXAM_ALLOWED_ORIGINS。`
- 只有默认配置被抓取；`XEXAM_ALLOWED_HOSTS` / `ALLOWED_ORIGINS` / `ALLOW_ANY_HOST` 的变体由所有者的 `host_test.py`（55 项）覆盖，本次未重跑。

### 3.2 `GET /api/info`

- 请求：无体、无特殊头。Host 守卫。
- 200：`{"version": str, "arch": str, "canShutdown": bool, "bankCount": int}`（252-253）。
  - `version`：VERSION 文件，须匹配 `[0-9A-Za-z.+-]{1,20}`，否则 `dev`（44）。易变。
  - `arch`：`platform.machine() or 'unknown'`（Windows `AMD64`，Linux Docker 通常 `x86_64`/`aarch64`）。易变。
  - `canShutdown`：启动参数 `--allow-shutdown`。
  - `bankCount`：上架题数（`COUNT(*) WHERE active=1`）。
- 示例（捕获 001）：`{"version": "0.2", "arch": "AMD64", "canShutdown": false, "bankCount": 0}`
- 前端用：`version`（页脚）、`canShutdown`（显示"关闭程序"按钮，app.js:372-374）。`arch`、`bankCount` 前端不读；Dockerfile 的 HEALTHCHECK 和 smoke-test.sh 用本端点（smoke-test 用 grep `"bankCount": 0`，所以**依赖带空格的 JSON 格式**）。

### 3.3 `GET /api/bank`

200 对象，键顺序固定（db.py:151-155）：`file`、`mtime`、`error`、`questions`、`sheets`。

| 键 | 类型 | 含义 |
|---|---|---|
| file | str | 最近一次导入的文件名；从未导入为 `""` |
| mtime | str | 最近一次导入的时间 `YYYY-MM-DD HH:MM`（服务器本地时间，TZ 相关）；空为 `""`。易变 |
| error | str | 永远是 `""`（遗留字段，前端 app.js:337 仍读） |
| questions | list | 仅"上架"题，按 `seq` 排序 |
| sheets | list | 每个工作表一项：`{name, usable, inactive, total, skipped}`；`total == usable`；`skipped` 永远 `[]`；仅 `usable>0` 或 `inactive>0` 的表（db.py:154） |

每道题（捕获 162）的键顺序：`kind, tag, stem, options, items, answer, translation, zh, explanation, mnemonic, note, id, sheet, row`。前 11 个来自库里存的 JSON（db.py:213 排除 id/sheet/row），后 3 个由 `bank()` 追加（db.py:146）。

| 键 | 类型 | 含义 |
|---|---|---|
| kind | "single" / "multi" / "hotspot" | 题型 |
| tag | str | 热点题为 `"ホットスポット"`，否则 `""` |
| stem | str | 题干（可含 `\n`） |
| options | list | 选择题 `[{key:"A", text:str}]`；热点题为 `[]` |
| items | list | 热点题 `[{text:str, choices:[str,...]}]`；选择题为 `[]` |
| answer | list | single/multi：字母数组 `["B"]`、`["A","C"]`（升序）；hotspot：每个小项被选中的 choices 下标 `[0,1,0]` |
| translation | str | B 列中文翻译原文（可为 `""`） |
| zh | object / null | 解析成功时 `{stem, options:[str]}`（选择题）或 `{stem, items:[str]}`（热点题）；选项字母或小项数对不上则为 `null`（前端据此不提供中文切换） |
| explanation | str | 【解析】段 |
| mnemonic | str | 【口诀】段 |
| note | str | 答案括号内的备注，用 `；` 连接 |
| id | str | `sha1(sheet\nstem[:300]\|first[:200])[:12]`，重复时加 `-2`、`-3`… |
| sheet | str | 工作表名 |
| row | int | Excel 行号 |

空库（捕获 002）：`{"file": "", "mtime": "", "error": "", "questions": [], "sheets": []}`。v2 导入后（捕获 162）：`sheets=[{name:"demo",usable:6,inactive:1,total:6,skipped:[]}]`。

### 3.4 `GET /api/progress`

200：`{"version": 1, "questions": {<question_id>: rec}, "history": [...], "masterStreak": 2}`（db.py:253）。

- `rec`（db.py:245-249）：`{attempts:int, correct:int, wrong:int, streak:int}` + `last: bool`（仅当库里 `last` 不为 NULL）+ `at: str`（仅当非空；易变）。键顺序 attempts, correct, wrong, streak, last, at。
- `history`：最近 60 场，**最新在前**（`ORDER BY id DESC LIMIT 60`），每项 `{at, sheets:[str], mode, total, correct, percent, seconds}`（db.py:251-252）。捕获 158：写了 60+ 场之后长度正好 60，首项 seconds=60，末项 seconds=1。
- `masterStreak`：常量 2（错题连续答对几次后移出错题本）。
- 空库（捕获 003）：`{"version": 1, "questions": {}, "history": [], "masterStreak": 2}`。
- `questions` 的键包含库里不存在的 id（未知 id 也会被记录，见 3.8）。

### 3.5 `GET /api/imports`

200：`{"imports": [...]}`，最近 12 条，新的在前（db.py:233、server.py:259）。每项键（来自 `SELECT *`，表列顺序）：`id, at, filename, added, updated, removed, restored, skipped, total`（捕获 164，全是 int 除 `at`、`filename`）。前端只显示前 8 条（app.js:564 附近）。

### 3.6 `POST /api/upload`

- 请求头：`Content-Type` 必须含子串 `application/octet-stream`；`X-Filename`（可缺，URL 编码；前端用 `encodeURIComponent`）。`Content-Length` 必须有（见下）。正文 = 原始 xlsx 字节（不是 multipart）。
- `X-Filename` 处理 `clean_filename`（124-127）：`unquote` -> 反斜杠换成 `/` -> 取最后一段 -> 去控制字符 -> `strip` -> 截 120 字符；为空则 `未命名.xlsx`（捕获 112）。**服务端不检查扩展名**（只有前端 app.js:441 检查 `.xlsx`）。
- 大小 `_body(limit)`（272-290）：
  - `Content-Length` 缺失或不是数字 -> n=0 -> 读 0 字节 -> 空正文（对 upload 即 400"文件是空的"，对 JSON 即 `{}`；捕获 088）。**不支持 `Transfer-Encoding: chunked`**：没有 Content-Length 的分块正文被当成空正文，未读完的数据留在 socket 上（代码事实，未抓取）。
  - `n < 0` -> 返回 None -> 413（捕获 089）。
  - `n > limit` -> 继续读掉并丢弃最多 `min(n, 4*limit)` 字节，设置 `close_connection`，返回 None -> 413。边界：恰好 20 MiB 不拒绝（`>` 而不是 `>=`），20 MiB+1 -> 413（捕获 111）；21 MB -> 413（捕获 110）。恰好 20 MiB 本身未抓取。
  - 整个正文一次性读入内存；无读超时。
- 413 正文（捕获 110）：`{"error": "文件太大了（最大 20 MB）。题库通常只有 1–2 MB，请确认选对了文件。"}`（注意 `1–2` 里是 en dash）。
- `check_xlsx`（63-85）依次判断（400，`{"error": 消息}`，捕获 096-109）：
  1. 长度 < 100 字节 -> `文件是空的，或者没有上传完整。`（空体、99 字节都是这条，捕获 096-097）
  2. 前 2 字节不是 `PK`：若前 8 字节是 OLE 签名 `D0CF11E0A1B11AE1` -> `这是旧版 .xls 格式，或者是设置了密码的 Excel 文件。请在 Excel 里去掉密码，并另存为「Excel 工作簿(.xlsx)」再上传。`；否则 -> `这个文件不是 .xlsx（可能是 .csv、.pdf 或别的格式）。请在 Excel 里另存为「Excel 工作簿(.xlsx)」再上传。`（101、102）
  3. `zipfile.BadZipFile` -> `这个 .xlsx 文件已损坏，打不开。请用 Excel 重新保存一份再上传。`（103）
  4. 没有 `xl/workbook.xml` -> `这个文件里没有 Excel 工作簿的内容，不是有效的 .xlsx。`（104）
  5. 任一成员**声明**的 `file_size` > 100 MiB 或累计 > 300 MiB -> `文件解压后太大，已拒绝。`（109）。只看 zip 目录里声明的大小，不是实际解压量。
  6. 对每个以 `.xml` / `.rels` 结尾的成员（大小写敏感）整体解压到内存并检查字节串 `<!ENTITY` 或 `<!DOCTYPE`（大小写敏感）-> `文件里含有不被允许的内容（XML 实体声明），已拒绝。正常的 Excel 文件不会有这个。`（105）
- `parse_workbook` 的失败（400）：`zipfile.BadZipFile` / `KeyError` / `ET.ParseError` 转成带中文的 ValueError；格式 `这个文件缺少 .xlsx 必需的内容（<KeyError文本>），可能已损坏。`（捕获 107，里面含 Python 的 KeyError 文本和内部成员路径 `xl/_rels/workbook.xml.rels`）、`文件里的 XML 读不了：<expat 文本>`（捕获 106：`syntax error: line 1, column 0`）。其他未预料异常 -> `读取这个 Excel 时出错：<类名>: <文本>`。这些消息包含 Python/expat 内部文本，**Rust 不可能逐字复现**（见 openQuestions）。
- 成功 200：预览负载（见 4.1）；副作用：`Pending.put` 替换内存里唯一的待确认项（**会顶掉别人的上一个预览**），**不写数据库**（捕获 115：预览后 `/api/bank` 仍为空）。
- 无库里数据的"合法 zip 但无题"：照样 200，`usable:0`（捕获 108）；错误留到 confirm。

### 3.7 `POST /api/import/confirm`

- 头：`Content-Type` 含 `application/json`；正文 `{"token": "<预览里的 token>"}`。
- 流程（346-356）：`Pending.take(data.get('token'))`：
  - 无待确认项 / token 不对 / 过期 -> 409 `{"error": "这次上传的预览已经过期或已被处理，请重新上传。"}`（捕获 092、116-117、119、121）。token 为 int/None/缺失一律 409（`str(token or '')`）。
  - **token 含非 ASCII 字符 -> `secrets.compare_digest` 抛 `TypeError` -> 未捕获 -> 连接被直接断开，没有 HTTP 响应**（捕获 118 `RemoteDisconnected`；进程内检查同样得到 `TypeError`）。这是一个缺陷，不是契约。
  - 错 token **不会**消耗待确认项（`take` 在比较失败时不清空；进程内 `wrong_token_keeps_item: true`）。正确 token 一次性：取出即清空（捕获 120 之后 121 为 409）。
  - 过期项在 `take` 里被判为 `None`，但**不会被清除**（留在内存里直到被替换/取消；进程内 `expired_item_NOT_cleared_still_in_memory: true`）。
- 取出后若 `questions` 为空 -> 400 `{"error": "这份文件里没有读到任何可用的题，没有导入。"}`（350-351）；**此时 token 已被消耗**。（未抓取，代码事实）
- `apply_import` 异常 -> 500 `{"error": "写入数据库失败，题库没有任何改动：<异常文本>"}`（354-355）。（未抓取）
- 成功 200：`{"result": <plan>, "bank": <完整 /api/bank 负载>}`（356；捕获 120、132、161）。`result` 的键与预览相同但**没有** `filename`/`token`，且各表的 `skipped` 是完整列表，**没有** `skippedMore`/`skippedCount`（因为 `apply_import` 内部重新调用 `plan_import`，不经 `preview_payload`）。
- 副作用：单个事务 `BEGIN IMMEDIATE` ... `COMMIT`（db.py:100-110）：upsert 全部题、把文件里没有的上架题置 `active=0`、插入一条 `imports` 记录。注意 `plan_import` 在事务之前计算（db.py:207），所以 `result` 与 `imports` 行里的计数是事务外算出来的。

### 3.8 `POST /api/import/cancel`

正文任意 JSON 对象（甚至空）。无条件清空待确认项，**不需要也不检查 token**（358-359）。总是 200 `{"ok": true}`（捕获 094、127）。含义：任何能通过 Host/Origin 守卫的客户端都能取消别人的预览（没有会话隔离）。

### 3.9 `POST /api/attempt`

输入清洗 `clean_attempt`（132-145）：

| 字段 | 规则 |
|---|---|
| results | `(d.get('results') or [])[:2000]`，只保留 `{id: str, correct: bool}` 的 dict；`id` 截 64 字符；其余元素静默丢弃。`correct` 必须是真 bool（`1` 不算，捕获 147）。过滤后为空 -> 400 `{"error": "没有成绩可保存"}`（捕获 090-091、148-149）。 |
| sheets | `[s[:80] for s in (d.get('sheets') or []) if isinstance(s,str)][:50]`。传字符串 `"abc"` 会被逐字符遍历成 `["a","b","c"]`（捕获 153）。 |
| mode | 仅 `all` / `weak` / `retry`，其余一律改成 `all`（捕获 139）。 |
| seconds | `max(0, min(int(d.get('seconds') or 0), 604800))`；`int()` 抛 TypeError/ValueError 时为 0。捕获：巨大值 -> 604800（140）；负数 -> 0（141）；`12.7` -> 12（142）；`"77"` -> 77（143）；`"abc"` -> 0（144）；`NaN`（JSON 字面量）-> 0（146）。 |
| 其他字段 | 忽略（捕获 156）。 |

未捕获异常（连接被断开，无响应）：`seconds: Infinity`（`int(inf)` 抛 `OverflowError`，只捕获 TypeError/ValueError，捕获 145）；`results` 为 int 或 dict（`[:2000]` 对其抛 `TypeError`，捕获 150-151）；`sheets` 为 int（迭代抛 `TypeError`，捕获 154）；深度嵌套 JSON `[[[...` 抛 `RecursionError`（捕获 087）。**这些不是契约，是缺陷**；Rust 的合理行为是 400。

写入 `record_attempt`（db.py:255-272），一个事务：每条 result 先 `INSERT OR IGNORE INTO stats`，对的：`attempts+1, correct+1, streak+1, last=1, at=now`；错的：`attempts+1, wrong+1, streak=0, last=0, at=now`。插入 `attempts` 一行：`total=len(results)`，`correct=对的数量`，`percent=round(100*good/total)`，`seconds`。然后只保留最近 60 场（`DELETE ... NOT IN (最新60)`）。

- **未知 id 也会被记录**（捕获 138：`does-not-exist` 和被截成 64 字符的长 id 都进了 `questions`）。
- 同一请求里重复 id 各计一次，且都计入 `total`（捕获 155：total 2、correct 1）。
- **`percent` 用 Python `round()` = 银行家舍入（四舍六入五成双）**。本机验证 `round(0.5)=0, round(1.5)=2, round(2.5)=2, round(12.5)=12, round(37.5)=38, round(62.5)=62, round(87.5)=88`。Rust 的 `f64::round()` 是"远离零"，会得到 13、38、63、88 之中的不同值，**必须单独实现**，否则 1/8、5/8 等比例会不一致。
- 200 返回完整的 `/api/progress` 负载（所以 `history[0]` 就是刚写入的那场）。
- 捕获 135-137：5 题全对 -> `percent:100`；3 题 1 对 -> `percent:33`。

### 3.10 `POST /api/reset`

忽略正文内容（但仍要求 JSON 类型和合法 JSON 对象）。一个事务 `DELETE FROM stats; DELETE FROM attempts;`，**不动** questions / imports / meta（db.py:274-278）。200 返回空的 progress 负载（捕获 166）；重复调用同样 200（167）；`/api/bank` 不变（168）。

### 3.11 `POST /api/shutdown`

- 未启用（默认，Docker）：403 `{"error": "在 Docker 里请用 docker compose stop 停止容器。"}`（367-369；捕获 095）。注意它排在 JSON 守卫之后。
- 启用（`--allow-shutdown`，start.bat 用）：先回 200 `{"ok": true}`，再开线程 `server.shutdown()`（370-372）；之后端口拒绝连接（捕获 171-172，用第二个实例抓取）。
- 这是规格列表之外的第 10 个端点（规格只列了 9 个）。

## 4. 载荷细节

### 4.1 预览载荷（`POST /api/upload` 200）

键顺序（捕获 114、159）：`sheets, added, updated, unchanged, restored, removed, skipped, usable, removed_samples, removed_stats, filename, token`。

| 键 | 含义 |
|---|---|
| sheets[] | 对上传文件里的每个工作表 + 因本次导入而"全部下架"但不在文件里的旧表：`{name, total, usable, skipped, added, updated, removed, restored, skippedMore, skippedCount}`。`total`=读到的题行数（含读不出来的），`usable`=读出的题数，`skipped`=最多 30 条 `{row:int, reason:str}`，`skippedCount`=真实总数，`skippedMore`=被截掉的条数（server.py:51、180-181）。不在文件里的旧表 `total/usable=0, skipped=[]`（db.py:185-187）。 |
| added / updated / unchanged / restored / removed | 与库里现有题（按 id）比较的结果；`unchanged` 与 `updated` 的判定忽略 `row`（db.py:80） |
| skipped | int，所有表读不出来的题的总数 |
| usable | int，上传文件里读出的题总数 |
| removed_samples | 最多 5 条，被下架题的题干前 40 字符 |
| removed_stats | int，被下架的题里"有答题记录（stats 行）"的题数 |
| filename | 清洗后的文件名 |
| token | `secrets.token_urlsafe(16)`，22 字符 URL-safe base64（进程内检查长度 22，字符集 `[A-Za-z0-9_-]`）。易变 |

捕获 114：新库上传 demo.xlsx -> `added:5, usable:5, removed:0`；捕获 126：相同文件再传 -> `added/updated/removed/restored` 全 0、`unchanged:5`；捕获 159：demo_v2.xlsx -> `added:2, updated:1, removed:1, unchanged:3, removed_stats:1, removed_samples:["信号が赤のとき、どうしますか？"]`。

### 4.2 前端实际依赖的字段（web/app.js）

`post()` 对非 2xx 抛 `HTTP <status>: <正文文本>`（86-88）；`confirmImport` 再从这段文字里解析 JSON 取 `error`（486）。`uploadFile` 读响应 `Content-Type` 是否含 `json`，读 `d.error`、`d.token`（452-456）；预览用 `usable, added, updated, restored, removed, unchanged, skipped, removed_stats, removed_samples, sheets[].{name,usable,added,updated,restored,removed,skippedCount,skipped,skippedMore}`（498-526）。启动并行拉 `/api/bank`、`/api/progress`、`/api/info`，**不检查 `r.ok`**（853-857），所以 403 文本会在 `r.json()` 处抛错 -> 页面显示"连不上服务"。

## 5. 常数一览

`MAX_JSON=1_000_000`（48）、`MAX_UPLOAD=20*1024*1024`（49）、`PENDING_TTL=30*60`（50）、`SKIP_SHOWN=30`（51）、`HISTORY_LIMIT=60`（db.py:24）、`MASTER_STREAK=2`（db.py:23）、imports 列表 12（server.py:259）、results 2000 / id 64 / sheets 50 / 单个 sheet 80 / seconds 604800（134-142）、文件名 120（127）、解压 100 MiB / 300 MiB（80）。

## 6. 静态文件路由（server.py:260-270）

`rel = 'index.html' if path in ('', '/') else unquote(path).lstrip('/')`；`full = normpath(join(WEB_DIR, rel))`；`commonpath([full, WEB_DIR]) == WEB_DIR` 且 `isfile(full)` 才放行；否则 404 `not found`（`text/plain`）。整个文件读入内存后回送，`Cache-Control: no-store`。

- MIME 表：`.html .css .js(text/javascript) .svg .png .ico .json`，其他 `application/octet-stream`（187-191）；扩展名取 `lower()`。
- 捕获（Windows）：`/`、`/index.html`、`/?a=1` -> 200 同一页面（956 字节）；`/%61pp.js`（百分号解码）、`//app.js`（`lstrip('/')` 吃掉所有前导斜杠）、`/./app.js`、`/app.js#frag` -> 200，body md5 与 web/app.js 相同；`/web`（目录）、`/missing.js`、`/app.js%00.png` -> 404；`/../server.py`、`/%2e%2e/server.py`、`/..%2fserver.py`、`/..\server.py` 全部 404（捕获 020-032）。
- `/APP.JS` 在 Windows 上 200（NTFS 不区分大小写）；Linux 上预期 404，**未在 Linux 上验证**。
- 代码事实但未抓取：`/app.js/`（尾斜杠）会被 `normpath` 去掉 -> 也返回 app.js；`unquote` 对无效 UTF-8 用替换字符，不报错；任何隐藏文件只要在 `web/` 里都会被提供（没有白名单）；没有目录列表。
- 静态文件当前只有 `index.html`、`app.js`、`style.css`、`icon.svg`、`apple-touch-icon.png`（`ls web`）。

## 7. 副作用与状态汇总

| 端点 | 写 SQLite | 写内存 | 备注 |
|---|---|---|---|
| GET 全部 | 否 | 否 | |
| POST /api/upload | 否 | `Pending.item` 被替换 | 全局单槽，非每用户 |
| POST /api/import/confirm | 是（questions/imports，一个事务） | 清空 `Pending.item` | |
| POST /api/import/cancel | 否 | 清空 `Pending.item` | 无需 token |
| POST /api/attempt | 是（stats/attempts，一个事务） | 否 | |
| POST /api/reset | 是（删 stats/attempts） | 否 | |
| POST /api/shutdown | 否 | 停止服务 | 仅 `--allow-shutdown` |

并发：`Pending` 用 `threading.Lock`，`Store` 用 `RLock`，事务 `BEGIN IMMEDIATE`。进程内 50 线程同时 `take` 同一个 token，恰好 1 个成功（`concurrent_take_winners: 1`）。HTTP 层的并发确认未抓取。

## 8. 黄金响应捕获

位置：`tests/baseline/golden/`。

- `capture.py`：默认从仓库路径启动 `server.py`（`-I -B --no-browser --port 8701`，临时数据目录在 `api/work/` 下，结束后删除；另起 8703 的 `--allow-shutdown` 实例抓 shutdown），顺序执行场景，写 172 个 `NNN_<name>.json` + `index.json`（md5、环境）。
- 重跑对象可换：`python -I -B capture.py --base-url http://127.0.0.1:PORT [--shutdown-url http://127.0.0.1:PORT2] [--out DIR]`。要求目标是**全新的空库**、默认配置（无额外 Host/Origin 白名单、shutdown 关闭）；Host/Origin 场景的主机名取自 base-url 的主机名。脚本开头会断言 `/api/progress` 为空，防止误抓到真实数据。
- 每个捕获文件含：`request`（方法、路径、头、正文摘要）、`status`、`http_version`、`headers`（全部）、`headers_stable`（不含 Date/Server）、`volatile_headers`、`body_len`、`body_md5`、`raw_body`、`json`（原始）、`normalized`（易变键替换）。静态文件只记录 md5 和长度。无响应（服务端丢连接）记录 `status: null` 与 `no_response`。
- **易变值标记**：JSON 里键名为 `token`、`mtime`、`at`、`version`、`arch` 的非空字符串值在 `normalized` 中替换为 `"<VOLATILE>"`；头 `Date`、`Server` 视为易变（只在 `headers` 里保留，`headers_stable` 排除）。差异比较时只比 `status`、`headers_stable`、`normalized`、静态文件的 `body_md5`。注意 `Content-Length` 对含 `at`/`token` 的响应在不同实现间可能不同（时间字符串等长，token 等长；`version`/`arch` 长度可能变），差异工具应在这些响应上忽略它。
- 场景是**有状态的顺序脚本**（编号即顺序）：空库 -> 错误路径 -> 上传/确认 demo.xlsx -> 各种 attempt -> demo_v2 导入 -> reset -> shutdown。
- 涉及真实 demo：`tests/baseline/fixtures/demo.xlsx`（md5 0771c043b0433eee89438445513e88f3）、`demo_v2.xlsx`（5c7007f32a0dd260d0d33bb5b671b8d5）；全是虚构内容。没有读取所有者的两个私人题库。
- 已抓取（172 项）：空库四个 GET、HEAD、查询串、尾斜杠、未知路径；全部静态变体；6 种不支持的方法 + CORS 预检；Host（缺失/空/各种变体/过长/控制字符）与 Origin（evil、null、file、ftp、userinfo、空、仅协议、带路径、不同端口、大写、同源、无 Origin）；JSON 端点的 Content-Type 各种坏法、坏 JSON、非对象、空体、BOM、无效 UTF-8、NaN、1,000,001 字节、深度嵌套、Content-Length 非数字/负数；upload 的空体、99 字节、错 Content-Type、缺 Content-Type、子串绕过、`.xls` 签名、非 zip、损坏 zip、无 workbook、DOCTYPE、非 XML、缺 rels、无题、声明 101 MiB 的成员、21 MB、20 MiB+1、缺 X-Filename；错 token / int token / null token / 非 ASCII token / 重复 token / 取消后 token / 第二次上传顶掉第一次；attempt 的全对、混合、weak/retry、未知 id、坏 mode、巨大/负/浮点/字符串/乱码 seconds、过滤无效项、全无效、results 非法类型、sheets 截断、重复 id、多余字段、历史封顶 60；v2 导入（新增/更新/下架）；reset 两次；shutdown 关/开。
- 过期无法在 HTTP 上等 30 分钟：见 8.1。

### 8.1 过期测试（进程内，不起服务）

`golden/expiry_inprocess.py`（结果在 `expiry_inprocess.result.json`）：`import server`，把 `server.PENDING_TTL` 改成 -1（`Pending.put` 在调用时读取模块全局，server.py:160）；或直接把 `pending.item['expires']` 改到过去。结果：
`expired_take_none: true`、`expired_by_clock_take_none: true`、`take_twice_second_is_none: true`、`wrong_token_keeps_item: true`、`second_put_replaces_first: true`、`concurrent_take_winners: 1`、`non_ascii_token: TypeError`、token 长 22。
对新后端做 HTTP 级过期测试的办法：给后端加一个可配置的 TTL（环境变量），测试时设成 1 秒，上传 -> sleep -> confirm 应得 409；或用可注入的时钟。

## 9. 我看到的缺陷与技术债（API 层面，事实）

1. 未捕获异常导致连接被直接断开而不是返回 4xx：`seconds:Infinity`、`results` 为 int/dict、`sheets` 为 int、深度嵌套 JSON、非 ASCII 的 confirm token。（捕获 087、118、145、150、151、154）
2. 错误消息泄露内部细节：`读取这个 Excel 时出错：<异常类名>: <异常文本>` 和 KeyError 文本含内部成员路径（server.py:329、xlsx_parser.py:379）。与规格"不向用户暴露堆栈或路径"有张力。
3. `Server` 头暴露 Python 版本。
4. 不支持分块请求体；缺少 Content-Length 的上传被当作"文件是空的"。
5. 没有读超时（`Handler.timeout=None`），整体正文读入内存（20 MiB 以内）。
6. `Pending` 是全局单槽：多个客户端会互相顶掉对方的预览；`cancel` 不需要 token。与规格"pending isolated between users/sessions per the existing access model"一致的前提是：现有访问模型就是"无会话、单用户"。
7. 过期的待确认项不会主动清除；过期后仍占内存直到被替换。
8. XML 实体检查是对原始字节做大小写敏感的子串搜索，且只检查 `.xml`/`.rels` 后缀成员；`xlsx_parser` 再用 `xml.etree.ElementTree`。UTF-16 编码的 XML 是否能绕过这个字节检查，**未测试**（见 openQuestions）。
9. 解压大小只检查 zip 目录里声明的值；检查阶段会把所有 `.xml`/`.rels` 成员整个解压进内存。
10. `confirm` 取出后若题数为 0 返回 400，但 token 已被消耗；`apply_import` 的计数在事务之外计算。
11. 500/409 等路径对 HEAD 不适用（HEAD 只走 GET 路径）。
12. 启动时 `open_store` 会读取 `data/progress.json` 或**仓库根目录的 `progress.json`** 并一次性迁移进库（server.py:382-388）。副作用：在仓库根目录里用新的数据目录启动一个服务器，会把该文件迁移进新库。我自己的第一次探测启动就触发了这一点（见 notVerified 与下面的泄漏说明）；`capture.py` 之后预置了 `meta.legacy_migrated` 来避免。
13. 前端启动时的三个 GET 不检查 `ok`（见 4.2）。

## 10. 兼容性要点（给 Rust 后端）

必须逐字节一致：JSON 空格风格（`", "` / `": "`，因为 smoke-test.sh 用 grep `"bankCount": 0`）、键顺序、`ensure_ascii=False`、`Content-Type` 字符串（含 `; charset=utf-8`）、`Cache-Control: no-store`、`X-Content-Type-Options: nosniff`、所有中文错误消息文本（前端直接展示 `error` 字段）、状态码（尤其 415 先于 404、Host 先于 Origin、Origin 先于 Content-Type）、`percent` 的银行家舍入、`token` 为 22 字符 URL-safe、`seconds` 的钳制规则、`sheets` 的 80/50 截断和字符串被逐字符迭代的行为（这是缺陷，需所有者决定是否保留）。
可以改进（需所有者确认后算"有意差异"）：第 9 节第 1 项各种断连改成 4xx；去掉 Python 版本；解析器错误消息中的 Python/expat 文本。
前端不依赖：`arch`、`bankCount`、`bank.error`（恒为空）、`sheets[].skipped`（恒为空）、`id` 的具体算法（只要稳定）。**但已有数据库里的题 id 必须保持不变**，所以 `make_id` 的 SHA-1 算法和规范化规则必须逐字符复现。
Nginx 相关：前端 HTTP/1.0 单连接无 keep-alive；后端在 nginx 后面时 Host 会被 nginx 改写，必须按所有者的 Cloudflare 部署笔记保持 `XEXAM_ALLOWED_HOSTS` / `ALLOWED_ORIGINS` / `ALLOW_ANY_HOST` 语义；后端从不读 `X-Forwarded-*`，建议保持。

## 11. 与其他维度的接口

题目解析的细节（`xlsx_parser.py`）、数据库表结构、Docker/Nginx 部署不在本维度范围内，这里只引用了它们对 HTTP 契约的影响。

## 12. 未验证（notVerified）与未抓取项

- **进程泄漏（需要协调者处理）**：我为了探测启动的第一个服务器（命令行含 `--port 8700 --data （审计时的临时目录）work/probe_data`，PID 10416）**仍在运行**。我两次尝试 `taskkill` 都被权限系统拒绝，按规则 4 没有再找别的办法。请协调者或所有者停止它。同时 `api/work/probe_data/xexam.db` 里含有启动时自动迁移进来的 `progress.json` 副本（所有者的个人学习记录，3 场成绩、25 题的答题记录；服务器启动日志证实）。我没有读取、打印或复制其内容，也没有修改源文件（`progress.json` 修改时间仍是 2026-11-09 12:39）。该目录在我的允许写入范围内，但建议在停掉 PID 10416 后删除 `api/work/probe_data`（我无法删除：文件被占用）。
- `api/work/cpython312_http_server.py` 不是我创建的（可能来自别的维度）。
- Python 3.12 / Linux / Docker 下的差异未验证：501 错误页的 HTML 文本、`Server` 头、静态路径大小写敏感性、反斜杠路径、`platform.machine()`。
- 未抓取：
  - 环境变量变体（`XEXAM_ALLOWED_HOSTS`、`ALLOWED_ORIGINS`、`ALLOW_ANY_HOST`）下的响应；
  - 恰好 20 MiB 的上传边界、分块上传、慢速上传；
  - `confirm` 的 400（空题）与 500（数据库失败）；
  - 预览里 `skippedMore` > 0（需要一个有 30 条以上读不出来的行的工作簿）和有 `skipped` 条目的预览；
  - `results` 超过 2000 条、`sheets` 超过 50 项的截断；
  - HTTP 层的并发确认；
  - 带恶意 Host 的 501 方法；`/app.js/` 尾斜杠；
  - 真实 30 分钟过期的 HTTP 层表现（只做了进程内测试）；
  - `--import` 命令行模式、信号退出（SIGTERM）。
- 没有读取或使用所有者的两个私人工作簿。
- 没有运行 `host_test.py`、`tunnel_sim.py`。

## 13. 开放问题（openQuestions）

1. 错误消息是否必须逐字一致？解析器的异常文本包含 Python/expat 内部消息，Rust 无法复现。建议的契约：状态码和 JSON 形状一致；"固定中文消息"逐字一致；"带内部异常文本"的几条（400 的 `读取这个 Excel 时出错：…`、`文件里的 XML 读不了：…`、`缺少 .xlsx 必需的内容（…）`）只比较前缀。需要所有者确认。
2. 未捕获异常导致的断连（第 9 节第 1 项）是否保留？建议改成 400/413，并记为"有意差异"。
3. `sheets` 传字符串时被逐字符拆分、`correct` 必须是严格 bool、未知题 id 被接受并写入 stats，这些要不要原样保留？（本草稿把它们作为必须复现的现状。）
4. UTF-16 编码的 XML 成员是否能绕过 `check_xlsx` 的字节检查并让 expat 展开实体？本次未测试，需要在 Phase 1 用恶意样本验证。
5. `_body` 在 Content-Length 缺失时读 0 字节：Nginx 对分块上传的行为（默认会缓冲并补上 Content-Length，`proxy_request_buffering on`）需在 Phase 5 验证；Rust 后端是否要自己支持分块并强制 20 MiB 上限（规格要求不信任 Content-Length）。
6. 规格里的 `/api/shutdown` 没有列出，但前端和 start.bat 在用：Rust 版是否保留？
7. 在 Nginx 之后，`Host` 会被改写：Rust 后端的 Host/Origin 守卫是配置成看 nginx 转发的 Host，还是看 `X-Forwarded-Host`？当前 Python 版从不读 `X-Forwarded-*`，保持这一点最安全。
8. 与规格的差异：规格把数据库文件写作 `<old-name>.db`，实际是 `xexam.db`。
