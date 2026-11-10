# Xexam Phase 0 安全审计草稿：安全控制、信任模型、Nginx 之后会变什么

> 维度：security。只读审计，没有改动仓库、没有碰 `data/`、`progress.json`、私人题库。
> 事实 = 代码做了什么（带 file:line 或命令结果）；SPEC = 规格书要求什么；建议 = 我的推荐。三者分开写。
> 命名：规格书里把数据库文件写成"旧名字.db"，真实文件是 `xexam.db`（`server.py:380` 快照版）。这是一处规格书与实际不符，已按要求这样报告。

## 0. 我依赖的文件及指纹（本草稿写作期间仓库在变，务必看这一节）

| 文件 | 我读/测的版本 md5 | 写作结束时工作区 md5 | 说明 |
|---|---|---|---|
| `server.py` | `48306e1f716d...`（快照，下称"快照版"） | `fb9e06bf74b8...`（HEAD `1ee64c8` = v0.2 提交） | **两者不同**。本文行号全部指快照版。差异见第 8 节 |
| `app/db.py` | `02846cbc21b2...` | `3ea04694b314...` | 差异：COMMIT 失败时补 ROLLBACK；旧 progress.json 迁移更健壮 |
| `app/xlsx_parser.py` | `239b594ef8cb...` | 同 | 无变化 |
| `Dockerfile` | `b031d35f...`（第一次读） | `1e7c33378939...` | 变化：HEALTHCHECK 改为不走代理、端口跟随 `XEXAM_PORT`（`Dockerfile:23-24`） |
| `docker-compose.yml` | `160023431bbe...` | 同 | |
| `docker-compose.build.yml` | `368a22bbd8c5...` | 同 | |
| `web/app.js` | `0dd0e1139626...` | `da9c02aca7d2...` | 我读的是前者；后者的变化我没看。**前端行号可能偏移** |
| `web/index.html` / `style.css` / `icon.svg` | `738e7167...` / `7e5a5233...` / `c1b10c05...` | — | |
| `scripts/smoke-test.sh` | `c242b5de...` | — | |
| `.github/workflows/docker.yml` | `5c52d527...` | — | |
| `start.bat` | `82c61a98...` | — | |
| `VERSION` | `15085825...`（内容 `0.2`） | 同 | |

实测用的是快照版的副本：`（审计时的临时目录）pyapp/`（md5 与上表"快照版"相同，只拷了代码，没拷任何数据）。所以第 4 节的实测结果**是对快照版的**，不是对当前 HEAD 的。当前 HEAD 已修掉其中好几项（第 8 节），需要用 `probe.py --spawn-python <仓库目录>` 重跑一遍才能确认（我没有重跑，见 notVerified）。

## 1. 结论摘要

1. **信任模型**：程序自己没有登录、没有会话、没有用户概念。能连到端口并且 `Host`/`Origin` 过检的人，可以读全部题库与成绩、上传导入、清空成绩（`README.md:168, 778-780`；代码里没有任何认证逻辑）。防护 = 默认只绑本机 + `Host` 白名单（防 DNS 重绑定）+ 写请求 `Origin` 检查（防跨站）+ 前面的 Cloudflare Access。迁移必须原样保留这个模型，不发明认证。
2. **X-Forwarded-\* 与客户端 IP**：对 `*.py *.js *.html *.css *.yml *.sh *.bat` 做 grep（`client_address|X-Forwarded|Forwarded|remote_addr|REMOTE|getpeername|X-Real`）零命中。应用不读客户端 IP，也不识别 HTTPS，`log_message` 是空函数（`server.py:197-198`），没有访问日志。所以 Nginx 放前面不会改变"客户端 IP"相关行为，因为本来就没有。
3. **放到 Nginx 后真正会出问题的只有一件事**：后端看到的 `Host`。Nginx 默认转发 `Host: $proxy_host`（= `backend:8765`，已按 nginx 文档核实），后端的 `Host` 白名单会让**所有请求 403**，而且浏览器的 `Origin`（公网域名/localhost）与 `Host`（backend）不同，所有写请求也 403。必须显式写 `proxy_set_header Host $host;`，并让 Nginx 自己先按 `server_name` 把未知 Host 拒掉（第 5 节）。
4. **发现的真实缺陷**（快照版，已实测，除标注外）：
   - XML 实体过滤可绕过：UTF-16 编码的 DOCTYPE/ENTITY、扩展名不是小写 `.xml/.rels` 的部件，都能通过 `check_xlsx`，而且内部实体真的被展开进了入库的题干（探针 E01 `expanded_in_bank=True`）。expat 的放大保护挡住了十亿笑声（U21），外部实体不解析（U22）。
   - 伪造 ZIP 头**不能**绕过内存上限（zipfile 按声明大小截断读取，见 3.2），但"诚实的大文件"仍可：单部件 ≤100 MiB、总计 ≤300 MiB 解压后进内存，条目数无上限（F08，3 万条目通过）。
   - 正则灾难回溯：`xlsx_parser.py:154` 的 `HINT_RE` 对"一个非空白字符 + 一长串空格"呈立方级耗时，正则层面实测 500 空格 3.6 s、600 空格 6.9 s、1000 空格超过 20 s。经 `parse_hotspot`（`xlsx_parser.py:192`）可由上传内容触达（读代码确认）；**端到端经 /api/upload 的整条路径我没有跑**。CPython 的 `re` 不释放 GIL，推断整个服务会被卡住（推断，未实测）。
   - 单一全局 Pending 槽：两个标签页互相作废；任何人都可无令牌调用 cancel。
   - 快照版里有若干"异常不回包、直接断连"的路径（探针 EMPTY：非 ASCII token、`results` 不是列表、`seconds=Infinity`、JSON 嵌套 20 万层）。**当前 HEAD 的 diff 显示这些已被修**，但我没重跑确认。
5. **必须在 Rust 里重新实现的**：Host 白名单、Origin 检查、两种 403 文案、静态路径防穿越（或交给 Nginx）、Content-Type 检查、请求体大小（独立于 Nginx 的真实字节计数，含 chunked）、`check_xlsx` 的全部拒绝项、Pending（含一次性、恒定时间比较、30 分钟过期）、`/api/shutdown` 门控、attempt 清洗、SQL 参数化、SIGTERM 优雅退出。
6. **移到 Nginx 的**：未知 Host 的第一道拦截、`client_max_body_size 20m`、安全响应头、超时、`merge_slashes`、`autoindex off`、隐藏文件拒绝、静态文件服务与路径穿越（由 Nginx 的 root/alias 承担）、`Server` 头隐藏、chunked 请求体的缓冲。

## 2. 控制项清单（逐项：规则 / 位置 / 怎么测 / 迁移去向）

行号为快照版 `server.py`（除另有标注）。"探针"指 `probe.py` 里的编号，见第 4、10 节。

| # | 控制项 | 精确规则（事实） | file:line | 怎么测 | 迁移去向 |
|---|---|---|---|---|---|
| C1 | Host 白名单 | 默认集合 `{localhost,127.0.0.1,::1}` ∪ `XEXAM_ALLOWED_HOSTS`；`host_name()`：去空白、转小写、`[` 开头取括号内、否则含 `:` 就 `rsplit(':',1)[0]`、去末尾 `.`。`XEXAM_ALLOW_ANY_HOST` 取 `1/true/yes/on`（大小写不敏感）时直接放行。GET 和 POST 都检查，静态文件也检查（H26）。HEAD 走 GET 路径。**未知方法（PUT 等）在检查之前就 501**（M-PUTevil） | `server.py:88-96, 110-121, 213-220, 245, 434-440` | H01-H27、H30-H36（hosts 档）；anyhost 档全 200 | **Rust 重新实现**（SPEC 明确不得关闭）；同时 Nginx `server_name` 做第一道 |
| C2 | Host 解析的宽松处 | 实测 `host_name('localhost:80@evil.com')='localhost'`（放行）、`'user@localhost'` 不放行、`'[::1]x'='::1'`、`'localhost:abc'` 放行（H07）、`Host: 127.0.0.1.` 放行（H20）。浏览器不会发出这类 Host，所以不是可利用的洞，但 Rust 若换成严格解析会产生"差异" | `server.py:91-96` | 本机直接调用 `host_name`（我跑过） | 建议：Rust 用严格解析并把差异记入文档，作为有意加固，不算缺陷 |
| C3 | 双 Host 头 | `self.headers.get('Host')` 取**第一个**（H21 放行，H22 拒绝） | `server.py:220` | H21/H22 | Nginx 对重复 Host 的处理我**没有**在本机验证（本机无 nginx），见 notVerified。Rust 侧 hyper/axum 的行为也待测 |
| C4 | Origin 检查（仅写请求） | `Origin` 头缺失 → 放行（命令行工具）。存在时：`origin_host()` 只认 `http://`/`https://`，主机部分不能为空、不能含 `@`；取主机名后与 `host_name(Host)` 相等，或在 `allowed_origins`（= `XEXAM_ALLOWED_ORIGINS` ∪ `XEXAM_ALLOWED_HOSTS`）里 → 放行。协议和端口**不比较**（O03/O04）。空值 `Origin:` → 403（O13）、`null` → 403（O12）、无协议 `evil.com` → 403（O11）、带 userinfo → 403（O15） | `server.py:99-107, 222-234, 440` | O01-O28、O30-O37 | **Rust 重新实现**。注意：`ALLOWED_HOSTS` 同时算合法 Origin（O32） |
| C5 | 两种 403 与检查顺序 | POST：`_deny_reason()` 先 Host 后 Origin，403 JSON `{"error": 中文说明}`，文案带被拒的 Host/Origin（去控制字符、截 80 字符）和该加哪个环境变量。GET：Host 不对 → 403 `text/plain`，文案 `Forbidden host: <名>。通过别的地址访问……XEXAM_ALLOWED_HOSTS……XEXAM_ALLOW_ANY_HOST=1……`。Host 与 Origin 都错时只报 Host（O25）。`GET` **不检查 Origin**（O23） | `server.py:246-248, 292-311` | H27、O24-O26 | Rust 重新实现，文案要逐字一致（前端和 README 都引用） |
| C6 | POST 内部顺序 | ① Host ② Origin ③ （上传）Content-Type 415 → 体大小 413 → check_xlsx/解析 400 → plan → 放进 Pending；（JSON）Content-Type 415 → 体 413 → JSON 400 → 路由（未知路径 404）。**所以未知路径 + 非 JSON 类型得到 415 而不是 404**（M-POSTunknownCT） | `server.py:308-373` | M-POSTunknown、M-POSTunknownCT | Rust 要保持同序 |
| C7 | 静态路径穿越 | `rel = unquote(path).lstrip('/')`；`full = normpath(join(WEB_DIR, rel))`；`commonpath([full, WEB_DIR]) == WEB_DIR` 且 `isfile`，否则 404 `not found`。`ValueError`（不同盘符）算不在内。无扩展名白名单，`web/` 下任何文件都发（含点文件，但现在 `web/` 里没有）。不解析符号链接（用 normpath 不用 realpath） | `server.py:260-270` | T01-T22（全 404）、T30/T31（目录内的 `.` 和 `a/..` 合法解析 200）、T08 `%00` → 404 | **移到 Nginx**（root + `try_files`）。Rust 不再服务静态文件，只需保证 `/api/` 之外不处理 |
| C8 | `//` 开头路径 | 标准库把 `//x` 折成 `/x`（`http/server.py:338`），所以 `//api/info` 返回 200（探针 M-//info；在 part1 结果里它是唯一被标记 SURPRISE 的项）。这是行为差异点 | 标准库 | M-//info | Nginx `merge_slashes on`（默认，文档核实）同样折叠；Rust 直连时 `//api/info` 会 404，只影响绕过 Nginx 的直连 |
| C9 | Content-Type 检查 | 快照版：上传要求 `'application/octet-stream' in ctype`（子串）；JSON 路由要求 `'application/json' in ctype`（子串）。实测 `text/plain;application/json` 通过（C03）、`application/jsonp` 通过（C06）、`Application/JSON` 被拒（C04，大小写敏感）。`text/plain;application/json` 属于浏览器 CORS 安全列表类型，跨站可不预检直接发——**所以真正挡 CSRF 的是 Origin 检查，不是 Content-Type** | `server.py:317, 334` | C01-C10 | Rust 重新实现；建议精确匹配媒体类型（当前 HEAD 的 JSON 路由已改成只比较分号前的主类型，`server.py` HEAD 差异第 356-357 行；上传仍是子串） |
| C10 | 请求体读取 `_body` | 解析 `Content-Length`：非数字 → 当 0；负数 → 返回 None（413）；超限 → 先读掉最多 `min(n, limit*4)` 字节再 413 并关连接；否则读 n 字节。**忽略 `Transfer-Encoding: chunked`**（按 0 字节处理：B10/B11 小 JSON 得 200；B24 上传 21 MiB chunked → 连接被重置而不是 400）。无 Content-Length 的 21 MiB → RESET（B23）。`Content-Length: 1_0`（Python `int()` 接受下划线）→ 线程永远等第 10 个字节，探针 TIMEOUT（B08）。两个 Content-Length → 用第一个（B09）。**没有读超时**（`BaseHTTPRequestHandler.timeout` 为 None，我打印核实），慢速连接会无限占线程 | `server.py:272-290` | B01-B26 | **Rust 独立重做**：真实字节计数（`DefaultBodyLimit`/`RequestBodyLimitLayer` 之类），含 chunked；Nginx 的 `client_max_body_size 20m` 是第二道 |
| C11 | 大小常量 | `MAX_JSON=1_000_000`（恰好 1,000,000 字节通过 B01，1,000,001 → 413 B02）；`MAX_UPLOAD=20*1024*1024`（20 MiB 通过大小检查后被内容检查拒 B20；20 MiB+1 → 413 B21；21 MiB → 413 B22）。标准库头部限制：单行 >65536 → 431/414，头字段 >100 个 → 431（B30-B32） | `server.py:48-49` | 同上 | Rust 与 Nginx 都要设；两者的 20 MiB 边界要一致（Nginx 的 `20m` = 20 MiB，文档核实写法为"超过即 413"） |
| C12 | `check_xlsx` 全部拒绝项 | ① `len<100` →"文件是空的"（U 档 25/26）② 前两字节不是 `PK`：OLE2 魔数 `D0CF11E0A1B11AE1` →"旧版 .xls 或有密码"；否则"不是 .xlsx"③ `BadZipFile` →"已损坏"④ `namelist` 里没有 `xl/workbook.xml` ⑤ 对每个条目累计 `file_size`（**声明值**）：单条 >100 MiB 或累计 >300 MiB →"解压后太大" ⑥ 条目名以 `.xml`/`.rels`（**小写、区分大小写**）结尾的，`z.read` 整个读进内存，找字节串 `<!ENTITY` 或 `<!DOCTYPE`（**大写 ASCII 精确匹配**）→"含 XML 实体声明"。`check_xlsx` 之外还有 `parse_workbook` 的 `BadZipFile/KeyError/ParseError` → 400 | `server.py:63-85`；`app/xlsx_parser.py:374-381` | U02-U05、U10-U22、F01-F08 | **Rust 重新实现**，且应加固（见 3.2、3.3） |
| C13 | 上传后的异常出口 | `ValueError` → 400 中文说明；**任何其他异常** → 400 `读取这个 Excel 时出错：<异常类名>: <异常文本>`，会把内部信息带出去，例如 F06 的 `RuntimeError: File <ZipInfo filename='xl/wor…`、F07 的 `NotImplementedError: That compression method…`、F03 的 `BadZipFile: Bad CRC-32 for file …`。CLI 导入路径 `--import` 只捕获 `OSError/ValueError` | `server.py:326-329, 409` | F03、F06、F07 | Rust：返回稳定的错误码和中文说明，不回传内部类型名/成员名（SPEC：无堆栈/路径） |
| C14 | Pending 槽 | **全局只有一个槽**（`self.item`）。`put` 覆盖旧的。令牌 `secrets.token_urlsafe(16)`（128 位，22 字符，P01）。`take(token)`：持锁，`compare_digest(it['token'], str(token or ''))` 且 `expires >= time.time()` 才取出并清槽；**令牌不对或已过期都返回 None 且不清槽**（所以错令牌不会消耗，P08）。30 分钟 TTL（`PENDING_TTL=1800`），用墙钟 `time.time()`，没有后台清理，过期项留在内存直到被覆盖/取走/取消。`drop()` 无条件清空，**取消不需要令牌** | `server.py:50, 150-174` | P01-P20 | Rust 重新实现；决策点：是否保持单槽（见 openQuestions） |
| C15 | 确认的一次性与并发 | 取出即清槽，随后才执行 `apply_import`。**即使写库失败（500）或没有可用题（400，P17）令牌也已消耗**（P18 再确认 → 409）。16 个并发 confirm 同一令牌 → 恰好 1 个 200，其余 409（P19，数据库 `imports` 表只多 1 行） | `server.py:346-356`；`app/db.py:205-226` | P08-P19 | Rust 要用互斥/原子 take 保持同样语义 |
| C16 | 令牌比较 | 快照版：`compare_digest` 作用在 `str` 上，**非 ASCII 令牌抛 TypeError，连接被直接断开（P07 EMPTY）**；HEAD 已改为先 `.encode('utf-8')` 再比较 | `server.py:167`（HEAD 差异） | P07 | Rust：字节比较，类型不对一律 409 |
| C17 | 令牌的暴露面 | 令牌只在 POST JSON 请求体里（前端 `post('/api/import/confirm',{token})`），不在 URL、不在 Cookie，所以 Nginx 默认访问日志不会记录它。**别把 `$request_body` 或 `$http_x_filename` 加进 `log_format`** | `web/app.js`（confirmImport）、`server.py:347` | 读代码 | Nginx 配置约束 |
| C18 | `/api/shutdown` 门控 | 仅当命令行带 `--allow-shutdown`（`cfg.allow_shutdown`）才执行；否则 403 `在 Docker 里请用 docker compose stop 停止容器。`；`/api/info` 报告 `canShutdown`，前端据此显示"关闭程序"按钮。Dockerfile 的 CMD 没有这个标志（`Dockerfile:26`）→ 容器里恒为 403（X01）；`start.bat:19` 带了 → 本机直接运行时，通过 Host/Origin 的请求能把服务关掉。关闭是 `threading.Thread(target=self.server.shutdown)`（X02/X03 实测会退出） | `server.py:367-372, 397, 438, 252-253` | X01-X03 | Rust：保留标志与 403 文案；在 Docker 里永不开启 |
| C19 | attempt 校验 | `results` 取前 2000 条，仅保留 `id` 是 str 且 `correct` 是 bool 的，`id` 截 64；全部被丢弃 → 400 `没有成绩可保存`；`sheets` 仅 str、每个截 80、最多 50；`mode` 不在 `all/weak/retry` → `all`；`seconds` 转 int 夹到 `[0, 7*86400]`，转换失败 → 0（A09-A14）。**不检查 id 是否真的存在于题库**：2000 个不存在的 id 全部写进 `stats`（A19，stats 行数 4003）。快照版 `results` 为 dict/int、`sheets` 为 int、`seconds=Infinity` 会未捕获异常 → 断连（A04/A05/A15）；HEAD 已加 `isinstance(list)` 和 `OverflowError` | `server.py:132-145`；`app/db.py:255-272` | A01-A20 | Rust 重新实现；"不存在 id 也入库"是行为，保持还是收紧要问主人 |
| C20 | SQL 参数化 | `app/db.py` 与 `server.py` 里所有 `execute` 都用 `?` 占位绑定。唯一的字符串格式化是 `db.py:203`：`'... IN (%s)' % ','.join('?'*len(ids))`，插入的只是 `?` 占位符，值仍绑定，安全。没有 f-string/`.format` 拼 SQL（grep 核实）。`executescript(SCHEMA)` 是固定常量。潜在边角：`IN (…)` 里占位符个数超过 SQLite 变量上限（本机 SQLite 3.40.1 默认 32766）时会抛 `OperationalError`，`plan_import` 在 try 之外，会断连。需要库里一次下架 3 万多道题才触发，**未实测** | `app/db.py:96, 203` 及全部 execute | grep（见 3.5） | Rust 用 rusqlite 参数；`IN` 列表同样要分批 |
| C21 | 响应头 | 所有 `_send` 的响应：`Content-Type`、`Content-Length`、`Cache-Control: no-store`、`X-Content-Type-Options: nosniff`（R01-R04，R07）。**没有** CSP、X-Frame-Options、Referrer-Policy、Permissions-Policy、HSTS；没有任何 CORS 头（O27 OPTIONS → 501；O28 无 ACAO）。`Server: Xexam/0.2 Python/3.10.11`（R05，版本泄露）。501 页面是标准库的 HTML，没有 nosniff/no-store（M-hdr）。协议是 HTTP/1.0（每个请求一个连接） | `server.py:195, 200-208` | R01-R07、M-hdr | **移到 Nginx**：加安全头、`server_tokens off` |
| C22 | 错误披露 | 500 写库失败返回 `写入数据库失败，题库没有任何改动：<异常文本>`（`server.py:355`，sqlite 异常文本可能带内部信息）；未捕获异常 → 标准库把完整堆栈（含绝对路径）打到 stderr，客户端只看到断连（server_default.log 里可见） | `server.py:354-355` | 日志检查 | Rust：500 用固定文案，细节只进日志 |
| C23 | SIGTERM/SIGINT | 信号处理器启动线程调 `httpd.shutdown()`；`serve_forever` 返回后 `server_close()`、`store.close()`（`close` 先拿 `RLock`，所以等当前事务结束）。`ThreadingHTTPServer.daemon_threads=True`（核实），`server_close` 不会 join 请求线程。**只读代码确认，Windows 上无法实测 SIGTERM**；`smoke-test.sh:86-91` 在 CI 里测 `docker stop` 少于 9 秒 | `server.py:454-473` | smoke-test | Rust：axum `with_graceful_shutdown` + 等待 `spawn_blocking` 事务完成 + 关库 |
| C24 | 容器用户与端口 | uid 10001 的 `app`，`USER app`；`XEXAM_HOST=0.0.0.0` 在镜像里；compose 发布 `${XEXAM_BIND:-127.0.0.1:8765}:8765`，所以默认只对宿主机本机开放。**端口回退**：占用时 `port..port+19` 逐个试（`server.py:424-429`），容器里若 8765 被占会悄悄换端口而映射/健康检查对不上 | `Dockerfile:4-8, 11, 18, 20`；`docker-compose.yml:26`；`server.py:394, 424-429` | smoke-test.sh:43 | Rust：不做端口扫描，绑不上就退出；Docker 中只加入内部网络 |
| C25 | 数据库安全设置 | `journal_mode=DELETE`、`synchronous=FULL`、`foreign_keys=ON`（但 schema 里没有任何外键，该 PRAGMA 实际无约束作用）、单连接 `check_same_thread=False`、`threading.RLock` 串行化、写事务 `BEGIN IMMEDIATE`，异常回滚 | `app/db.py:88-97, 100-110` | 读代码 | Rust：同样的 PRAGMA、单写连接/专用线程，`spawn_blocking` |
| C26 | 前端防注入 | Excel 文本一律 `textContent`/`createTextNode`（`app.js:2`，`h()` 在 `:27-41`），grep 无 `innerHTML/insertAdjacentHTML/eval`；`fetch` 全是相对路径 `/api/...`（同源）；`index.html` 无内联脚本、`style.css` 无外部 URL。**但 `h()` 用 `setAttribute('style', …)` 写内联样式（`app.js:34, 363, 396, 401, 505, 514, 527`）**，严格的 `style-src 'self'` CSP 会拦掉它们 | `web/app.js`、`web/index.html` | grep | 第 5.6 节 CSP 建议；TS 重写时可改成 CSSOM（`el.style.x=`，如 `app.js:584`）或 CSP 放行 `style-src 'unsafe-inline'` |
| C27 | 上传文件名 | `clean_filename`：URL 解码、反斜杠换斜杠、取最后一段、去控制字符、截 120、空则 `未命名.xlsx`（N02/N03）。**服务端不检查扩展名**（N01 `evil.exe` + 合法 xlsx 字节照收）；扩展名只在前端检查（`app.js` 约第 441 行 `/\.xlsx$/i`；HEAD 版 app.js 已变，行号可能偏移）。文件名只存进 `imports.filename`，不碰文件系统 | `server.py:124-127` | N01-N03 | SPEC 要求"只收 .xlsx"：Rust 如要服务端校验扩展名，就是新增行为，需主人确认（见 openQuestions） |

## 3. 逐点深挖

### 3.1 请求体：Content-Length、chunked、"21 MB"
- 事实：Python 后端只认 `Content-Length`（C10）。`Transfer-Encoding: chunked` 被当作 0 字节。
- Nginx 在前面时：文档核实"开启请求缓冲时，整个请求体读完后才发给上游"，且"chunked 请求体无论如何都会被缓冲，除非对上游用了 HTTP/1.1 或 2"（`proxy_request_buffering` 文档）。所以保持默认 `proxy_request_buffering on`，后端看到的是完整请求体（Nginx 会补 `Content-Length`，这一点来自我的经验，**未在本机验证**）。
- 建议（SPEC 已要求）：Rust 后端**不信任** Content-Length 和 Nginx，自己对实际读到的字节计数，超过 20 MiB 立刻 413，chunked 也一样。
- 413 的文案：Python 返回 JSON `{"error":"文件太大了（最大 20 MB）。题库通常只有 1–2 MB，请确认选对了文件。"}`（`server.py:321`）。Nginx 自己的 413 默认是 HTML。前端对非 JSON 响应会退化为 `上传失败（HTTP 413）。`（`app.js:452-454`），**友好文案会丢失**。建议用 `error_page 413 = @too_large;` 回同样的 JSON。

### 3.2 伪造 ZIP 头能否绕过大小检查（问题明确要求）
读 CPython `zipfile`（本机 3.10.11）源码并结合实测：
- `zipfile` 的 `ZipInfo.file_size/compress_size/CRC` 取自**中央目录**。打开成员时（`ZipFile.open`）只读本地头并核对**文件名**，本地头里的尺寸被忽略。
- 读取（`ZipExtFile._read1`）：`self._left = zipinfo.file_size`；每次 `data = data[:self._left]` 截断，`_left<=0` 即 EOF；解压用 `decompress(data, n)` 带最大输出长度。**结论：声明值比真实值小，读出来的就是被截断的声明长度，不会超出。** 在 EOF 处核对 CRC，不匹配抛 `BadZipFile("Bad CRC-32 …")`。
- 实测：F03（声明 10 字节、CRC 不改）→ 400 `BadZipFile: Bad CRC-32`；F04（真实 30 MiB、头里写 200 字节并让 CRC 对上截断前缀）→ `check_xlsx` 通过，后续 XML 解析因被截断而失败，400，0.08 s。
- 所以：伪造"偏小"的头**不能**让更多数据进内存。声明值偏大（F01 150 MiB、F02 4×90 MiB）被拒。
- 仍然成立的风险（诚实大文件）：单部件上限 100 MiB、总计 300 MiB 都按**声明值**计，解压后全部进内存；`check_xlsx` 对每个 `.xml/.rels` 部件 `z.read` 整块读入，解析器再读一次并构建 ElementTree，内存峰值我**没有测量**。实测的"诚实 30 MiB 高压缩工作表"（4391 KiB 的 zip）耗时 2.39 s 且被接受（F05）。DEFLATE 理论压缩比约 1000:1，20 MiB 上传在 300 MiB 总上限内可展开约 15 倍。条目数无上限（F08：3 万个空条目 0.22 s 通过）。
- 建议（Rust）：对**实际解压字节数**计数（边读边累加，超限即中止），而不是信任声明值；限制条目数、单部件和总量、路径长度；拒绝加密位和未知压缩方法并给出统一的 400，而不是把异常文本回传（C13）。

### 3.3 XML 实体/DOCTYPE 拒绝：扫描了哪些、漏了什么
事实（C12⑥）：只扫描文件名以小写 `.xml` 或 `.rels` 结尾的部件，在**原始字节**里查大写 ASCII 的 `<!ENTITY`、`<!DOCTYPE`。实测：

| 情形 | 结果 | 探针 |
|---|---|---|
| UTF-8 的 DOCTYPE+ENTITY，在 `sheet1.xml`、`workbook.xml`、`workbook.xml.rels`、无关的 `docProps/app.xml` | 全部 400（拒绝） | U10-U14 |
| 只有 DOCTYPE 没有 ENTITY | 400 | U11 |
| DOCTYPE 文字出现在非 xml 部件（`note.txt`） | 200（不扫） | U15 |
| **UTF-16-LE / UTF-16-BE / 带 BOM 的 UTF-16 的 DOCTYPE+ENTITY** | **200，通过过滤** | U16-U18 |
| UTF-8 的 DOCTYPE+ENTITY，但工作表部件叫 `sheet1.dat`（关系指向它） | 200 | U19 |
| 同上，部件叫 `sheet1.XML`（大写扩展名） | 200 | U20 |
| UTF-16 编码的内部实体并被引用进题干，确认导入 | **实体被展开，写进了题库**（`expanded_in_bank=True`） | E01 |
| 十亿笑声（10^9）放在 UTF-16 工作表里 | 400，0.07 s，`limit on input amplification factor`（expat 2.5.0 的保护，`pyexpat.EXPAT_VERSION` 我核实过） | U21 |
| 外部实体 `SYSTEM "file:…"` | 400，`undefined entity`（ElementTree 不解析外部实体） | U22 |

判断：这个过滤**能挡常规攻击，但不是真正的防线**；真正挡住十亿笑声的是 expat 自带的放大保护，而这依赖于运行时的 libexpat 版本（容器里的 `python:3.12-slim` 用的 libexpat 版本我**没有核实**）。
建议（Rust，属于对原行为的有意加固）：用 XML 解析器的事件流，**见到任何 DOCTYPE 事件就拒绝**（与编码无关）；扫描范围覆盖压缩包里所有部件而不看扩展名；不启用自定义实体解析。quick-xml 对自定义实体的默认行为我**没有核实**，Phase 3 前需要确认。

### 3.4 Pending：双标签页、过期、取消
- 两个标签页：标签页 A 上传得到令牌 `tA`；标签页 B 上传得到 `tB`，覆盖槽；A 点确认 → 409（P10）。更隐蔽：B 只要打开再关闭"题库管理"页，前端 `closeAdmin` 在存在预览时会 `POST /api/import/cancel`（`app.js:430`），而取消不需要令牌，会作废 A 的预览（P15）。
- 过期：代码读取确认 `expires < time.time()` 时 `take` 返回 None（C14）。**30 分钟过期我没有实测**（要么等 30 分钟，要么 mock 时间，时间不够）。
- 令牌错误不消耗；正确令牌取走即消耗，即使后面写库失败或没有题（C15）。
- "隔离"：没有用户/会话概念，所以不存在跨用户隔离问题；唯一的"隔离"是令牌本身。Rust 如果改成按令牌索引的 map，两个标签页将不再互相作废，这是**行为差异**，也会改变 `cancel` 的语义（无令牌的取消要清什么？）。建议保持单槽以保持兼容，决策交给主人。

### 3.5 SQL 参数化核对（grep 结果）
对 `app/` 搜索 `.execute( / executescript / executemany / %s / .format( / f'/f"`：`db.py` 所有 execute 都是 `?` 绑定（行 115-117, 121, 131, 136, 160, 203, 214-218, 221, 223, 224, 230, 235, 241-242, 262-271, 276-277, 295, 300, 304, 307），唯一的 `%` 格式化是 `db.py:203` 的占位符串。`xlsx_parser.py` 里的 `%` 只用于错误消息。无 SQL 拼接用户输入。

### 3.6 X-Forwarded-\*、客户端 IP
见 1.2。结论：没有任何读取。Rust 后端不得**引入**对 `X-Forwarded-*` 的依赖。尤其注意：`axum-extra` 的 `Host` 提取器按 `Forwarded` → `X-Forwarded-Host` → `Host` → URI 的顺序解析（文档核实，且标注已弃用）。**安全检查必须直接读 `Host` 头**，不能用这个提取器，否则任何客户端都能伪造 `X-Forwarded-Host` 通过白名单。

### 3.7 ReDoS（顺带发现）
- `xlsx_parser.py:154`：`HINT_RE = [\t\u3000 ]+([^\t\u3000/]+?(?: +/ +[^\t\u3000/]+?)+)[ \t\u3000]*$`。我在 `explore1.py` 里直接对正则计时：`'x'+' '*n`：n=100 0.011 s，200 0.131 s，300 0.536 s，400 1.551 s，500 3.63 s，600 6.87 s，n=1000 超过 20 s 被我杀掉。立方级。
- 触达路径：`parse_question`（无选项时）→ `parse_hotspot`（`xlsx_parser.py:299-300`）→ 对每一行调用 `HINT_RE.search(text)`（`:192`）。单元格内容完全由上传者控制。**端到端（经 /api/upload）没有跑**。
- 影响（推断）：`re` 匹配期间持有 GIL，其他请求线程饿死，Nginx 在 60 s（默认 `proxy_read_timeout`）后会向客户端返回 504，而后端仍在计算，Nginx 的超时不会取消它。信任模型里上传者就是主人自己，风险低，但 SPEC 点名要"parsing time limits"。
- Rust：`regex` 库是线性时间，没有这个问题；但原代码里有 lookbehind（`xlsx_parser.py:216`），`regex` 不支持，需要手写或 `fancy-regex`（这属于 parser 维度，仅提示）。

## 4. 实测结果（对快照版 `server.py` md5 48306e1f，端口全部在 8710-8719 内）

执行方式：`probe.py --spawn-python …/pyapp`，4 个配置档（default / hosts / anyhost / shutdown），每档在 `run/workN/data_*` 的临时目录里起一个服务，结果在 `run/results_part1.json`、`results_body.json`、`results_upload.json`、`results_rest.json`（共 340 条，其中被探针自己标记为 SURPRISE 的只有 3 项：M-//info、B23、B24）。除第 0 节说明外，下表"结果"栏是实际观测值。

> 注意：`probe.py` 不是我写的。我自己的 Write 因文件已存在被拒绝，随后发现该文件由本次运行中的另一个进程写入并反复修改（md5 先后为 `ee29ca07…`、`1c05f0c0…`、`0c278320…`），随后它跑出了上面的结果文件。我读了它的文件头和命令行参数、用 `ast.parse` 确认可解析，**没有逐行审阅其全部逻辑**。表中数字我是从它产出的 JSON 里读的。

| 项 | 请求 | 观测结果 | 探针 |
|---|---|---|---|
| Host 基本 | `Host: localhost:PORT` / `127.0.0.1` / `[::1]:PORT` / `LOCALHOST.` | 200 | H01-H05 |
| Host 异常端口 | `localhost:99999999`、`localhost:abc` | 200（端口被忽略） | H06、H07 |
| 陌生 Host | `evil.example`、`evil.example:80`、无 Host（HTTP/1.0）、空 Host、`user@localhost`、`localhost@evil.com`、`::1` 无括号、`[::1` 括号未闭合、`localhost.evil.com`、`"localhost, evil.com"` | 全部 403 | H10-H19 |
| 两个 Host 头 | localhost 在前 evil 在后 / 反过来 | 200 / 403（取第一个） | H21、H22 |
| 绝对形式请求行 | `GET http://evil/.. + Host: localhost` | 200（只看 Host 头） | H23 |
| HEAD | 坏 Host / 好 Host | 403 / 200 且无正文、有 Content-Length | H24、H25 |
| hosts 档 | `xexam.example.com`、`XEXAM.Example.COM:8443`、`192.168.1.20:8080` | 200 | H30-H32 |
| hosts 档 | 只在 ALLOWED_ORIGINS 里的域名当 Host / 域名前缀欺骗 | 403 | H33、H35 |
| anyhost 档 | 所有 Host | 全 200（H10-H19 全 200） | |
| Origin 同源 | 无 Origin / `http://localhost:PORT` / `https://localhost` / `http://localhost:9999` / `HTTP://LOCALHOST` / 带路径 / 末尾点 | 200 | O01-O08 |
| Origin 异常 | `127.0.0.1` 对 Host `localhost`、**无协议** `localhost:PORT`、`null`、空值、`file://`、userinfo、前缀欺骗、`ftp://`、`http://` | 全 403 | O10-O19 |
| 双 Origin | localhost 在前 / 在后 | 200 / 403 | O20、O21 |
| GET + 外站 Origin | | 200（GET 不查 Origin） | O23 |
| Host 与 Origin 都错 | | 403，文案只说 Host | O25 |
| CORS | OPTIONS 预检 / 外站 GET 的 ACAO | 501 / 无 ACAO 头 | O27、O28 |
| hosts 档 Origin | `Host 192.168.1.20 + Origin https://pub.example.org`（在 ALLOWED_ORIGINS） | 200 | O31 |
| hosts 档 Origin | `Host 192.168.1.20 + Origin https://xexam.example.com`（ALLOWED_HOSTS 里的名字也算合法 Origin） | 200 | O32 |
| anyhost 档 Origin | Host 任意 + 同主机 Origin / 外站 Origin | 200 / 403（Origin 检查仍有效） | O35、O36 |
| 方法 | PUT/DELETE/PATCH/OPTIONS/TRACE | 501（标准库 HTML 页，且先于 Host 检查） | M-* |
| 路由 | GET 路径 `/api/upload` 等 POST 专用路径；`POST /api/nope`；无 JSON 类型的 `POST /api/nope` | 404；404 JSON；**415** | M-GET*、M-POST* |
| 路径细节 | `//api/info` | 200（SURPRISE，标准库折叠 `//`） | M-//info |
| Content-Type | 无 / `text/plain` / `text/plain;application/json` / `Application/JSON` / `application/json; charset=utf-8` / `application/jsonp` | 415 / 415 / **200** / 415 / 200 / **200** | C01-C06 |
| 上传 Content-Type | `application/json` / `multipart/form-data` / `text/plain;application/octet-stream` / 无 | 415 / 415 / **200** / 415 | C07-C10 |
| JSON 体大小 | 恰好 1,000,000 / 1,000,001 | 200 / 413 | B01、B02 |
| 非法 JSON / 数组 | | 400 / 400 | B03、B04 |
| Content-Length | `-1` / `abc` / `1_0` / 两个（2 与 9999999） | 413 / 200（当 0）/ **超时（线程挂起等字节）** / 200（取第一个） | B06-B09 |
| chunked JSON | 小体 | 200（忽略 TE，当空体） | B10、B11 |
| JSON 嵌套 20 万层 | | **EMPTY（断连，RecursionError）** | B12 |
| 上传体大小 | 20 MiB 垃圾（CL） / 20 MiB+1 / 21 MiB（CL） | 400 / 413 / 413 | B20-B22 |
| **21 MiB 无 CL / 21 MiB chunked** | | **RESET / RESET**（预测是 400，SURPRISE：服务端 0 字节判空后提前回包，客户端还在发，连接被重置） | B23、B24 |
| 空体 / 52 字节 | | 400 | B25、B26 |
| 头部限制 | 单行 70,000 字节 / 150 个头 / 70,000 字符路径 | 431 / 431 / 414 | B30-B32 |
| 上传格式 | 文本 / OLE .xls / PK+乱码 / 缺 workbook.xml | 400 / 400 / 400 / 400（各自中文提示） | U02-U05 |
| 伪造声明大小 | 声明 150 MiB / 4×90 MiB / 声明 10 字节 CRC 不改 / 真 30 MiB 头写 200 字节且 CRC 对上 | 400 拒 / 400 拒 / 400 `Bad CRC-32`（泄露类型名） / 通过检查，解析失败 400 | F01-F04 |
| 诚实大文件 | 30 MiB 高压缩工作表 | **200，2.39 s** | F05 |
| 加密位 / 未知压缩 | | 400，但回传 `RuntimeError…`、`NotImplementedError…` 文本 | F06、F07 |
| 3 万个空条目 | | 200，0.22 s | F08 |
| 文件名 | `evil.exe` / 路径穿越+CRLF+300 字符 / 无头 | 200，名字被清洗；无头 → `未命名.xlsx` | N01-N03 |
| 令牌 | 错 / 缺 / 数字 / null / 列表 | 409 | P02-P06 |
| 令牌 | **非 ASCII（有待确认项时）** | **EMPTY（TypeError 断连）** | P07 |
| 一次性 | 错令牌后用正确令牌 / 重放 / 取消后再确认 | 200 / 409 / 409 | P08、P09、P13 |
| 两个标签页 | 第二次上传后确认第一个令牌 / 标签页 B 取消后 A 确认 | 409 / 409 | P10、P15 |
| 零题上传 | | 200 且仍发令牌；确认 → 400 且令牌被消耗 | P16-P18 |
| 并发确认 | 16 并发同令牌 | 恰好 1 个 200，15 个 409，`imports` 表 +1 行 | P19 |
| attempt | 空 / 缺 / 字符串 / 全部类型错 | 400 | A02、A03、A06、A07 |
| attempt | `results` 是 dict、int；`seconds=Infinity` | **EMPTY（断连）** | A04、A05、A15 |
| attempt | 2001 条 / 超长 id、60 个 sheet、`mode=evil`、`seconds=10^12` / `-5` / `'abc'` / `1.9` / `True` / `[1]` | 截到 2000 / 截断与夹值 / 0 / 0 / 1 / 1 / 0 | A08-A14 |
| attempt | 同 id 两次 / 2000 个不存在的 id | 都被记录 / **stats 行数 4003** | A17、A19 |
| 历史上限 | 65+ 次 | 60 条 | A18 |
| 静态穿越 | `/../server.py`、`/..%2f`、`/%2e%2e/`、`/..%5c`、`/....//`、`//server.py`、`%2e%2e%2f%2e%2e%2fdata/xexam.db`、`%00`、`/app.js%00.png`、`%252e%252e`、`/app/db.py`、`/server.py`、`/VERSION`、`/xexam.db`、`/.git/config`、`/..\server.py`、`/C:/Windows/win.ini`、`%c0%ae%c0%ae`、`/web/`、`/./`、`/api`、`/a/b/c` | **全部 404** | T01-T22 |
| 静态合法 | `/./app.js`、`/sub/../app.js` | 200（目录内解析） | T30、T31 |
| 大小写 | `/APP.JS` | 200（**这是 Windows 文件系统**；Linux 为 404） | T32 |
| Range / HEAD | | Range 被忽略返回 200 全文；HEAD 无正文 | T33、T34 |
| 响应头 | `/api/info`、`/`、`/app.js`、`/nope` | 有 Content-Type、Content-Length、Cache-Control、X-Content-Type-Options、Server、Date；`Server: Xexam/0.2 Python/3.10.11` | R01-R07 |
| shutdown | 默认档 / 外站 Origin / `--allow-shutdown` 档 | 403 / 403 / 200 且服务退出（`alive=False`） | X01-X03 |
| 日志里的未捕获异常 | | default 档日志含 `ConnectionAbortedError` 等堆栈，各档不同 | server_*.log |

我自己手工跑的两条 `curl`：`/api/info`（`Host: 127.0.0.1:8710`）→ 200，头为 `HTTP/1.0 200 OK, Server: Xexam/0.2 Python/3.10.11, Cache-Control: no-store, X-Content-Type-Options: nosniff`；`Host: evil.example` → 403 `Forbidden host: evil.example。…`。

## 5. Nginx 之后会变什么：Host/Origin 分析

### 5.1 后端会看到的 `Host`（按 Nginx 写法）

| Nginx 写法 | 后端收到的 Host | 对现有规则的影响 |
|---|---|---|
| 不写 `proxy_set_header Host`（默认） | `$proxy_host` = `proxy_pass` 里的 `名字:端口`，如 `backend:8765`（nginx 文档逐字核实） | `host_name`=`backend`，不在白名单 → **所有请求 403**。即使把 `backend` 加进白名单，写请求也会因 `Origin`（浏览器的公网域名/localhost）≠ `Host`(`backend`) 而 403。**不可用** |
| `proxy_set_header Host $host;` | 请求行里的主机，或 Host 头里的主机，或匹配的 `server_name`（文档核实优先级）；我的经验是小写且不带端口，**文档摘录没写这点，未在本机验证** | 等价于直连：白名单与 Origin 同主机比较都按"用户在浏览器地址栏看到的域名"工作 |
| `proxy_set_header Host $http_host;` | 原样的 Host 头（带端口） | 后端会去掉端口，效果同上；但没有 Host 头的 HTTP/1.0 请求会变成"没有 Host 转发"，后端 403（H12），反而是**更保守** |
| `proxy_set_header Host $proxy_host`（显式）/ 不写 | 同第一行 | 同第一行 |

两个细节：
- `$host` 会优先取**请求行**中的主机（绝对形式 URI）。而 Python 后端直连时只看 Host 头（H23）。用 `$host` 时，`GET http://evil/ + Host: localhost` 会被转成 `Host: evil` 然后 403，更保守。
- 未知 Host 应该在 Nginx 层就拒绝（`server_name` 明确列出 + `default_server` 返回 444），后端的检查作为第二层保留，这符合 SPEC（"不得关闭现有 Host/Origin 控制"）。

### 5.2 浏览器会发什么 `Origin`
浏览器对跨源请求和同源的 `POST` 都带 `Origin: scheme://host[:port]`（用户地址栏里的那个）。`Origin` 不会被 Nginx 默认改写（`proxy_pass_request_headers` 默认 on，文档核实），所以后端看到的 `Origin` 就是用户地址栏的来源，`Host` 取决于 5.1。同源判断是 `host_name(Origin) == host_name(Host)`（协议、端口都不比）。**只有当转发的 Host 等于用户在地址栏看到的主机名时，才不需要 `XEXAM_ALLOWED_ORIGINS`。**

### 5.3 四种部署

| 部署 | 浏览器 Host / Origin | Nginx 需要 | 后端环境变量 | 结论 |
|---|---|---|---|---|
| ① localhost 直连 Nginx（`http://localhost:8080`） | `Host: localhost:8080`，`Origin: http://localhost:8080` | `server_name localhost`；`Host $host` → `localhost` | 默认即可（`localhost,127.0.0.1,::1`） | 正常。`http://127.0.0.1:8080`、`http://[::1]:8080` 同理（`[::1]` 经 Nginx 的 `$host` 是否带括号我没验证，但后端两种写法都归一为 `::1`，见 C2） |
| ② 局域网 IP（`http://192.168.1.20:8080`） | `Host: 192.168.1.20:8080`，`Origin: http://192.168.1.20:8080` | `server_name` 里要有 `192.168.1.20`；对外绑定 `0.0.0.0` | `XEXAM_ALLOWED_HOSTS=192.168.1.20` | 正常。注意局域网里任何人都能直接访问，Cloudflare Access 保护不到这条路 |
| ③ 公网域名经 Cloudflare Tunnel（cloudflared 在局域网另一台机器 → Nginx，HTTP） | 浏览器 `Origin: https://xexam.example.com`。Host 取决于 cloudflared 的 `httpHostHeader`：不设则为公网域名（这是 `README.md:769` 的说法；Cloudflare 文档摘录里**没有写**默认 Host 行为，我没有独立核实）；设了则是源站地址 | `server_name xexam.example.com`；`Host $host` | `XEXAM_ALLOWED_HOSTS=xexam.example.com`。若 cloudflared 改写了 Host（如 `192.168.1.20:8080`）：再加 `XEXAM_ALLOWED_ORIGINS=https://xexam.example.com`（`README.md:770`，探针 O31/O32 验证了后端侧语义） | 推荐**不要**在 cloudflared 里改 Host。HTTPS 在 Cloudflare 终止，Nginx 只见 HTTP；应用不看协议，所以 `X-Forwarded-Proto` 无用 |
| ④ Docker 健康检查 | | 见 5.4 | | |

第 ③ 种的信任边界补充（事实 + 建议）：
- Cloudflare 文档核实：`X-Forwarded-For` 会追加（若请求已带则追加代理 IP，否则等于 `CF-Connecting-IP`）、`X-Forwarded-Proto` 若客户端设了会被覆盖、`CF-Connecting-IP` 只在边缘到源站的流量上发送。**这些头应用都不读**，因此 Nginx 不需要也不应该信任或转发它们。
- Cloudflare Access 的 JWT 校验文档的目的是"确保请求来自 Access 而不是恶意第三方"。当前部署里**局域网内任何主机都能绕过 Cloudflare Access 直接打 Nginx**（只要带对 `Host`，而 Host 由发起方自填）。这是信任模型决策，不是新增需求：可选缓解是主机防火墙只放行 cloudflared 那台机器的 IP、或 Nginx 只绑特定网卡，`README.md:779` 已提到防火墙。校验 JWT 需要额外组件，我不建议为此引入（SPEC 反对过度设计），交给主人决定。
- 不要把 `X-Forwarded-Host` 当 Host 用（见 3.6）。

### 5.4 Docker 健康检查
- 后端自己的 `HEALTHCHECK` 现在是容器内 `http://127.0.0.1:$XEXAM_PORT/api/info`，用 `urllib` 默认 Host `127.0.0.1:8765`，在默认白名单里，不受 Nginx 影响（`Dockerfile:23-24`）。Rust 镜像大概率没有 Python，需要自带子命令或 busybox `wget`，且 **Host 必须是 `127.0.0.1` 或 `localhost`**（否则被自己的 Host 检查 403）。
- Nginx 容器的检查：建议用 Nginx 自己返回的 `location = /healthz { return 200; access_log off; }`，不依赖后端；再单独做一条"经 Nginx 到后端"的端到端检查，带明确的 `Host: localhost`。
- 如果任何检查用服务名访问（`http://nginx/`、`http://backend:8765/`），Host 是服务名，会被 403（后端）或 444（default_server）。不要把服务名加进白名单去迁就检查。
- `depends_on: backend: condition: service_healthy`，让 Nginx 在后端健康之后启动。

### 5.5 推荐的最小、安全的头转发策略
Nginx（示意，`$` 变量原样）：

```nginx
# 未知 Host 直接断开，不到后端
server { listen 8080 default_server; server_name _; return 444; }

server {
    listen 8080;
    server_name xexam.example.com localhost 127.0.0.1 [::1] 192.168.1.20;   # 与后端 XEXAM_ALLOWED_HOSTS 同源（.env 渲染）
    server_tokens off;
    merge_slashes on;                       # 默认值，写明
    client_max_body_size 20m;               # 只在 /api/upload 放大也可；其它路径可更小

    location /api/ {
        proxy_pass http://backend:8765;
        proxy_http_version 1.1;
        proxy_set_header Connection "";
        proxy_set_header Host $host;                 # 唯一必须显式设的头
        # 应用不读这些，别转发，也不要让客户端自带的值穿过去：
        proxy_set_header X-Forwarded-For "";
        proxy_set_header X-Forwarded-Host "";
        proxy_set_header X-Forwarded-Proto "";
        proxy_set_header Forwarded "";
        proxy_set_header X-Real-IP "";
        # Origin、Content-Type、Content-Length、X-Filename 不动，默认就会透传
        proxy_request_buffering on;                  # chunked 由 Nginx 缓冲
        proxy_read_timeout 60s;  proxy_send_timeout 60s;  proxy_connect_timeout 5s;
        error_page 413 = @too_large;
    }
    location @too_large { default_type application/json; return 413 '{"error":"文件太大了（最大 20 MB）。题库通常只有 1–2 MB，请确认选对了文件。"}'; }
    location = /healthz { return 200 "ok"; access_log off; }
    location ~ /\. { deny all; }                     # 隐藏文件
    # 静态：root 指向前端构建产物；autoindex 保持默认 off；index.html 回退
}
```
理由：Nginx 空字符串头不会转发（文档核实"值为空串则该头不传给上游"），这样即使客户端自带 `X-Forwarded-*` 也不会进到后端。若以后真需要真实 IP，才改为 `$remote_addr`（单跳）并在后端只信任来自 Nginx 的值；当前没有需求，不建议做。

环境变量/默认值需要变的：
- `XEXAM_ALLOWED_HOSTS`：语义不变，但 `.env` 里这一个值要同时渲染进 Nginx 的 `server_name` 和后端环境（单一事实来源）。
- `XEXAM_ALLOWED_ORIGINS`、`XEXAM_ALLOW_ANY_HOST`：保留，默认空 / `0`，不要在 compose 里改默认。
- `XEXAM_BIND`：含义转到 Nginx 容器的端口映射（默认仍 `127.0.0.1:8080:8080`，对外仍需 `.env` 显式写 `0.0.0.0`）；后端用 `expose`，没有 `ports`。
- `XEXAM_HOST=0.0.0.0` 在后端容器里可以保留（只在内部网络里）。后端不要再做端口回退（C24）。
- Nginx 需同时在"对外网络"和"内部网络"，后端只在内部网络（`internal: true` 的网络没有外网出口，后端本来也不需要）。
- 数据卷：SPEC 要求 `./data:/data` 绑定挂载。`Dockerfile:11` 的 `chown app:app /data` 只对镜像里的目录和命名卷有效；**绑定挂载时宿主机目录的属主要与容器用户一致**（现在是 uid 10001），否则 SQLite 打不开/写不了。Rust 镜像的用户 uid 建议沿用 10001 以免迁移时权限错位。这条属于部署风险，不在我的主维度，但与安全相关。

### 5.6 CSP / 安全头建议（Nginx）
`X-Content-Type-Options: nosniff`（保留）、`Referrer-Policy: no-referrer`、`X-Frame-Options: DENY`（或 CSP `frame-ancestors 'none'`）、`Permissions-Policy` 关掉不用的能力、`Content-Security-Policy: default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self'; base-uri 'none'; form-action 'none'`。`style-src` 需要 `'unsafe-inline'` 是因为现有前端写内联 `style` 属性（C26）；TS 重写时若全部改成 CSSOM 就能去掉。不加 HSTS（Nginx 只见 HTTP，HTTPS 在 Cloudflare 终止；如要 HSTS 应在 Cloudflare 侧设）。缓存：现在每个响应都是 `no-store`（`server.py:204`），迁移到 Nginx 后 Vite 带哈希的静态资源可以长缓存，`index.html` 保持 `no-store`；这是行为变化，属于预期。

## 6. SPEC 安全清单逐项对照

标记：**有** = 原程序已有；**无** = 原程序没有；**不适用** = 在当前信任模型下不适用。我只写原程序做了什么，是否新增由主人决定。

| SPEC 项 | 状态 | 原程序实际情况 | 建议 |
|---|---|---|---|
| Host 校验 | 有 | C1-C5 | 保留（Nginx + Rust 两层） |
| Origin 校验 | 有（仅写请求） | C4；缺失 Origin 放行 | 保留，同样规则 |
| CORS | 无 / 不适用 | 不发任何 CORS 头，OPTIONS 501（O27/O28）；前端同源 | 保持无 CORS；Nginx 不加 `Access-Control-*` |
| 大小限制 | 有 | JSON 1 MB、上传 20 MiB、头部 431/414 | Nginx + Rust 各做一遍 |
| Content-Type 校验 | 有（弱） | 子串匹配（C9） | 改精确匹配主类型；这是有意加固 |
| HTTP 方法校验 | 部分 | 只有 GET/HEAD/POST，其余 501（先于 Host 检查）；POST 专用路径的 GET → 404 | Rust 返回 405/404 的选择要定；与旧行为不同要记录 |
| 路径穿越 | 有 | C7，T01-T22 全 404 | 交给 Nginx，保留测试 |
| 错误披露 | 部分 | C13、C22 | Rust 统一为稳定文案 |
| 安全响应头 | 部分 | 只有 nosniff 和 no-store | Nginx 补齐（5.6） |
| 代理信任边界 | 无（应用不感知代理） | 不读任何转发头 | 保持不读；Nginx 清空这些头 |
| 上传：扩展名 | 仅前端 | `web/app.js` 约第 441 行 `/\.xlsx$/i`（读的是 md5 0dd0e11… 版本；HEAD 版行号可能偏移）；服务端不查（N01） | 要不要服务端校验，需主人决定 |
| 上传：MIME | 弱 | 要求 Content-Type 含 `application/octet-stream`；真正的类型检查靠 `PK` 魔数 | 保持 octet-stream + 魔数 |
| 上传：ZIP 结构 | 有 | 魔数、`BadZipFile`、`xl/workbook.xml` | 保留；加条目数/路径长度上限 |
| 上传：ZIP bomb | 有（基于声明值） | 100/300 MiB，伪造偏小无效（3.2），诚实大文件仍可 | 改为统计实际解压字节 |
| 上传：XML 实体 | 部分 | 可绕过（3.3） | 加固为拒绝任何 DOCTYPE |
| 上传：解压限制/内存限制 | 部分 / 无显式内存限制 | 仅声明值上限 | 同上 |
| 上传：解析时间限制 | 无 | 无任何超时；有立方级正则（3.7） | `regex` 线性 + 解析总时限 |
| 上传：临时文件清理 | 不适用 | 全程内存（`BytesIO`），没有临时文件 | 保持内存或有界临时文件 |
| 上传：畸形 XLSX | 有 | `BadZipFile/KeyError/ParseError/兜底 Exception` | 保留，统一错误 |
| API：输入校验 | 部分 | attempt 清洗（C19），JSON 必须是对象 | 保留；快照版有 4 个未捕获路径（C19、C16） |
| API：SQL 注入 | 有（参数化） | C20 | rusqlite 参数绑定 |
| API：授权 | 无 | 没有登录 | **不发明**；写进文档 |
| API：令牌安全 | 有 | 128 位 CSPRNG、恒定时间比较、一次性 | 保留 |
| API：重放 | 有 | 取走即作废，并发只成功一次（P19） | 保留 |
| API：限流 | 无 | 没有任何限流 | 不适用于当前模型；Nginx 可选加 `limit_req`，**不是现状**，要不要加由主人定 |
| API：并发 | 有 | `RLock` + `BEGIN IMMEDIATE` | Rust 单写连接/专用线程 |
| 跨用户/会话 Pending 隔离 | 不适用 | 全局单槽，没有会话；任何人可无令牌取消 | 保持单槽以兼容；决策见 openQuestions |
| DB：事务 | 有 | apply 单事务（`db.py:205-226`） | 保留 |
| DB：文件权限 | 无（应用不设） | 依赖 umask 与卷属主 | 部署文档写清 |
| DB：日志模式 | 有 | `journal_mode=DELETE` + `synchronous=FULL`（`db.py:93, 95`） | 保留 |
| DB：备份 | 无（应用内） | README 要求停止后复制 | 迁移用，不新增 |
| DB：损坏处理 | 无 | 打不开直接崩溃 | 未测试 |
| DB：安全关闭 | 有（代码读） | C23 | 保留，补测试 |
| 认证 | 无 | 靠 Cloudflare Access | 保持 |
| 访问日志 / 审计 | 无 | `log_message` 空；HEAD 版只对被拒绝的请求打一行 | Nginx 访问日志可补 |

## 7. 原程序里的缺陷与技术债（安全维度）

1. XML 实体过滤绕过（3.3）。严重度：低到中（单人场景；expat 兜底）。
2. `HINT_RE` 立方级回溯（3.7）。可冻结整个服务。
3. 单槽 Pending 被无令牌取消作废（C14），前端打开/关闭管理页就会取消（`app.js:430`）。
4. 解压大小只按声明值，诚实大文件仍进内存；条目数无上限（3.2）。
5. 上传异常把内部类型名/文本回传（C13）。
6. `attempt` 接受任意不存在的题目 id，可无限增长 `stats`（A19）。
7. 没有读超时，慢连接无限占线程；`Content-Length: 1_0` 可令线程永久挂起（B08）。Nginx 的 `client_body_timeout`/`client_header_timeout`（默认 60 s，文档核实）会替后端挡住这一类。
8. 未知方法 501 先于 Host 检查，且 501 页面缺少 nosniff/no-store（M-PUTevil、M-hdr）。
9. `Server` 头泄露 Python 与应用版本（R05）。
10. 快照版里的"断连"类异常路径（C16/C19/B12）——HEAD 已改，待复测。
11. 端口回退会让容器悄悄换端口（C24）。
12. `foreign_keys=ON` 没有对应的外键（`stats.question_id` 不引用 `questions`），这是 schema 事实（不属本维度但与 A19 相关）。

## 8. 快照版 → 当前 HEAD 的 `server.py` / `db.py` 差异（我只做了 `diff`，没有重新实测）

| 位置（快照行号） | 变化 | 对应的探针 |
|---|---|---|
| `server.py:113` | `XEXAM_ALLOWED_HOSTS` 由按逗号拆改为按 `[,\s;]+` 拆 | 新增分隔符，旧探针不覆盖 |
| `server.py:134-143` | `results` 不是列表 → `ValueError('成绩的格式不对')`；`sheets` 不是列表按空处理 | A04、A05 预期由 EMPTY 变 400 |
| `server.py:147` | `seconds` 捕获 `OverflowError` | A15 预期 200（→0） |
| `server.py:167` | `compare_digest` 前先 `.encode('utf-8')` | P07 预期 409 |
| `server.py:246` 后 | 拒绝 Host 时 `say(...)` 打日志 | |
| `server.py:291` 后 | 新增 `_drain_body()`：提前拒绝（403/415）前把客户端请求体读掉（上限 `MAX_UPLOAD+65536`）并关连接 | 对 403/415 的上传类请求连接行为变化 |
| `server.py:310-340` | 403/415 之前调用 `_drain_body`；拒绝时 `say` 一行 | |
| `server.py:334` | JSON 路由的 Content-Type 改为 `ctype.split(';',1)[0].strip().lower() != 'application/json'`（精确匹配主类型） | C03/C04/C06 预期变化 |
| `server.py:341` | `json.loads` 同时捕获 `RecursionError` | B12 预期 400 |
| `server.py:380` | 启动时若没有 `xexam.db` 但有别的 `.db`，打印提示 | |
| `db.py:110` | `COMMIT` 失败则补 `ROLLBACK` | |
| `db.py:288-307` | 旧 `progress.json` 迁移：`utf-8-sig`、更多异常类型、`_migrate_rows` | |

上传的 Content-Type 仍是子串匹配（差异里没有改它），`check_xlsx` 与 `xlsx_parser.py` 没变。

## 9. 对 Rust 迁移的约束总结（保持行为 / 有意加固 / 待决策）

**必须逐字保持**：两种 403 文案与触发条件、检查顺序（C6）、`/api/shutdown` 的 403 文案与开关、`check_xlsx` 各条中文提示、409 文案（`server.py:349`）、400/413/415 的状态码、令牌格式（22 位 urlsafe）和一次性语义、并发确认只成功一次、Pending TTL 30 分钟、`Host`/`Origin` 比较只比主机名。
**建议有意加固（记入文档，不算缺陷）**：严格解析 Host、Content-Type 精确匹配、拒绝任何 DOCTYPE、按实际解压字节计数、条目数上限、解析总时限、统一错误（不回传类型名）、不做端口回退。
**待主人决策（见 openQuestions）**：是否保持单槽 Pending；是否服务端校验 `.xlsx` 扩展名；是否对不存在的题目 id 的 attempt 收紧；是否加限流；是否给 `//api/info`、未知方法保持旧状态码。

## 10. `probe.py` 说明

路径：`（审计时的临时目录）probe.py`（见第 4 节开头的作者说明）。

- 纯标准库、原始 socket，不规范化路径与头，因此可以发重复 Host、绝对形式请求行、`Content-Length: 1_0`、chunked 等。
- 四个配置档：`default`、`hosts`（`XEXAM_ALLOWED_HOSTS=xexam.example.com,192.168.1.20`、`XEXAM_ALLOWED_ORIGINS=https://pub.example.org`）、`anyhost`（`XEXAM_ALLOW_ANY_HOST=1`）、`shutdown`（Python 的 `--allow-shutdown`，Rust 需要定义等价物）。
- 两种用法：`python probe.py --spawn-python <含 server.py 的目录> --out results.json`（本机起服务，端口默认从 8710 起）；`python probe.py --base http://127.0.0.1:8710 --profile default --out r.json --golden results_python.json`（对已在跑的 Rust 后端/Nginx 比对，用一次性数据库副本）。
- 探针 id 与第 2、4 节引用一致。`want` 列是读 Python 源码后的预测，`SURPRISE` 表示实际结果与预测不一致（也用来检验我的阅读，不只检验服务）。
- 注意：对 Nginx 前端运行时，B 档（体大小）和 H 档（重复 Host）等项的差异是**预期的**（Nginx 自己答 413/400）；差异需要人工判断，不是判决。
- 我没有运行过"对 Rust"的版本（Rust 后端还不存在）。

## 11. 未验证 / 不确定

见返回结果中的 `notVerified` 与 `openQuestions`。要点：当前 HEAD 的 `server.py`（md5 `fb9e06bf…`）没有重新跑探针；30 分钟过期未实测；Nginx 对重复 Host/重复 Content-Length 的行为未在本机验证（本机没有 nginx）；Docker 引擎未运行（`docker images` 连接失败），所以没有实测容器、`docker stop`、HEALTHCHECK；Windows 上无法测 SIGTERM；ReDoS 没有端到端经 /api/upload 跑；大文件内存峰值未测量；我启动的端口 8710 上的测试服务进程（PID 27120）在我的 `taskkill` 请求被权限系统拒绝后没能关掉，需要有权限的人关闭。
