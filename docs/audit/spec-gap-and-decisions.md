# Xexam 现代化 Phase 0：规格与现实的差距（gap）

维度：规格 vs 实际仓库的冲突、歧义与需要业主决定的事项。语气中立，只写读过代码或跑过命令得到的事实。
标记约定：**[事实]** = 代码 / 命令结果；**[规格]** = 业主规格的要求；**[建议]** = 我的推荐；**[默认]** = 业主未答时将采用的做法。

> **重要更新（来自协调者转述，我未能直接核实原话）**：业主已回复大意为"我去睡了；缺什么自己装，不要问我；按顺序做 Phase 0 审计 → Phase 1 基线测试 → Phase 2 Rust 后端（在旧数据库的**副本**上测试）→ Phase 3 TypeScript 前端 → Phase 4 Nginx + Docker → Phase 5 对旧版做完整回归；做完上传到 GitHub"。
> 所以 Phase 0 之后"停下来等批准"的闸门已被业主免除；本文的每个决策项都给出**将被采用的默认值**（第 9 节）。该回复**没有**提到规格里其余的停止点（删源码、覆盖数据库、改业务规则、删功能、暴露后端端口、公开部署、改访问模型），所以我在第 5 节把默认值设计成**不会触发其中任何一个**。

---

## 0. 证据基线

### 0.1 仓库状态（`git` 只读命令）
- 分支只有 `main`（跟踪 `origin/main`），**没有任何 tag**（`git tag` 为空）。`git rev-parse HEAD origin/main` 都是 `8cec408be81c…`（= v0.1）。远端 `git@github.com:Yumin-Space/xexam.git`。
- 历史共 5 个提交（01:23 → 03:18，2026-11-10，+0900）：`1cb7512` 首次提交 → `f5c592d` 加 CI → `98a6512` 修 CI → `892a47b` 改名 + 命名卷 + README + 只拉取 compose → `8cec408` v0.1。
- 工作区 = 已完成但**未提交**的 v0.2：11 个已修改文件（`.dockerignore`、`.github/workflows/docker.yml`、`.gitignore`、`Dockerfile`、`README.md`、`VERSION`、`docker-compose.build.yml`、`docker-compose.yml`、`server.py`、`web/app.js`、`使用说明.txt`）+ 未跟踪的 `scripts/`。
- 工作区 16 个文本文件是 CRLF，而仓库内是 LF（`git ls-files --eol`；`.gitattributes` 规定 `* text=auto eol=lf`、`*.bat eol=crlf`）。

### 0.2 我依据的文件及 md5（读取时的值；复查时未变）
| 文件 | md5 |
|---|---|
| server.py (477 行) | 48306e1f716d5dadeeedb63e805cebae |
| app/db.py (308) | 02846cbc21b21da42bb6f680f8638792 |
| app/xlsx_parser.py (410) | 239b594ef8cbba5206e9a6f116178afe |
| web/app.js (865) | 0dd0e1139626fe020f483ff03a02de07 |
| web/index.html | 738e7167a1c666410cc6830e834917a2 |
| web/style.css (344) | 7e5a52336451cdbea29fb0517d540c54 |
| README.md (812) | 567104f5fe72cf444500ab69025a0acc |
| VERSION (`0.2`) | 15085825464d9079e36d615be010435c |
| Dockerfile | b031d35fad7d2bae6c2777be520e0399 |
| docker-compose.yml | 160023431bbe53035860a022124df9ae |
| docker-compose.build.yml | 368a22bbd8c5915c845be0f2e5698a13 |
| .github/workflows/docker.yml (160) | 5c52d527176c24a57cd4b670dc96fb30 |
| scripts/smoke-test.sh (93) | c242b5deecc79fc23b728e4cf31ac2f1 |
| start.bat | 82c61a9865a9f4535e60136cc44b0eac |
| .dockerignore / .gitignore / .gitattributes | 4644a88b… / dc87897a… / d6e54b23… |
| 使用说明.txt (103) | 142446c631485d17810fdcb6dff08653 |

### 0.3 本机工具链（`command -v` + `--version`）
python 3.10.11、node v26.13.0、npm 12.2.2、docker 28.1.1、git 2.38.1、gh 2.103.3 **存在**；**cargo、rustc、nginx 不存在**。`docker volume ls` 只有 3 个无关的卷，**没有 `xexam_data`**；`docker ps -a` 为空（只读命令）。
→ 业主真实数据库**不在这台机器的 Docker 卷里**；它在别的 Docker 主机上，或在仓库里的 `data/`（我按规则没有读）。这决定了"在副本上测试"的副本从哪里来（见 O1）。

### 0.4 我没做的事
没有编辑 / 创建 / 删除仓库或桌面上的任何文件；没有 git 写操作；没有读 `data/` 与 `progress.json`；没有启动服务器或浏览器；没有安装任何东西。两份私人工作簿只以 **只读**方式统计了结构（单元格类型数量），没有输出任何文字。我的探针脚本（stdlib，`python -I`）在本文件夹：`ev_checks.py`、`ev_checks2.py`、`ev_entity.py`、`ev_wb_struct.py`、`ev_wb_cols.py`（文件夹里另有一个 `ev_parser_probe.py`，不是我创建的，我没有使用它）。

---

## 1. 一页结论

第 2 节把规格拆成 80 行（每行可含若干相关要求句；逐行标 A 现在就成立 / B 直接移植后成立 / C 现在不存在的新工作 / D 与现有实现或业主更早的明确要求冲突 / E 有歧义，有些行是混合类别，所以我不给出精确的各类计数）。第 3 节展开 9 个 D 项，第 4 节展开 14 个 E 项，第 9 节是 29 行决策表。

**业主应该最先看的 9 个冲突（D）**
1. D1 数据库文件名：规格写旧名称 `.db`；代码、CI、全部文档都是 `xexam.db`；业主有明确的"旧名称一个字母都不能留"规则。→ 一律 `xexam.db`。
2. D2 `./data:/data` 绑定挂载 vs 现在的命名卷 `xexam_data`：绑定挂载是 v0.0（首个提交）的做法，后来被**有意**改掉。
3. D3 "前端不含业务逻辑" vs 现实：判分、错题本规则都在浏览器里，服务端只存 `{id, correct}`，且 API 契约要求不变。
4. D4 两个运行时镜像 / 两个服务 vs 现在"一个镜像、只拉取、一个文件即可部署"，以及镜像名 `ghcr.io/yumin-space/xexam` 已被在用的 compose 引用。
5. D5 "待确认导入按用户/会话隔离" vs 现实是**全局唯一的一个槽位**，没有用户和会话概念。
6. D6 "不泄露错误细节" vs 现在有两处把异常文本原样返回给浏览器；而中文错误文案本身又是 UI 依赖的 API 行为。
7. D7 "禁止 XML 实体展开" vs 现有检查对 UTF-16 编码的文件**可被绕过**（我实测了）。
8. D8 规格要求测试夹具（`.xlsx`、`.db`）vs `.gitignore` / `.dockerignore` 明确忽略它们（业主"仓库里不放题库"的规则）。
9. D9 "前端只用 REST、Vite 打包"vs 业主的截图 / 验证工具依赖页面全局变量（`S`、`render`、`clearSaved`…）；模块化后这些工具会失效。

**最容易被忽略、但会造成"同输入不同输出"的 8 个细节（第 3.x 节展开）**
percent 四舍五入（Python 银行家舍入 vs JS / Rust）、`/api/*` JSON 里 `": "` 后的空格被 `smoke-test.sh` 用 grep 依赖、Python `re` 与 Rust `regex` 的 Unicode 空白差异（题目 id 的哈希依赖它）、`fancy-regex` 类的后行断言需求（`xlsx_parser.py:216`）、容器里没有 tzdata 时时间戳静默变 UTC、健康检查会被自己的 Host 守卫拒绝、Nginx 的 413 页面不是 JSON（前端因此不会显示"文件太大"提示）、`compare_digest` 遇到非 ASCII token 会抛 `TypeError`。

**"upload to GitHub"的陷阱**：`docker.yml` 在 `push` 到 `main` 时会**发布镜像并移动 `latest`**（workflow :3-7、:86、:124）。默认做法：推到分支并开 PR（PR 不发布，workflow :86），新镜像用**新名字**，这样即使将来合并也不会改变业主现有 compose 拉到的东西。

---

## 2. 规格逐句分类

分类：A / B / C / D / E。"证据"列给 file:line 或命令结果。

### 2.1 目标与目录

| # | 规格要求 | 类 | 现状证据 / 说明 |
|---|---|---|---|
| 1 | 现代化现有、可运行的应用，不是重设计 | A（原则） | v0.2 能跑；业务逻辑集中在 `app/xlsx_parser.py`、`app/db.py` |
| 2 | 前后端完全分离 | C | 现在 `server.py:244-270` 同一进程既出 API 又出静态文件；前端已只通过 `fetch` 调 REST（`web/app.js:85, 425, 448, 479, 854-856`） |
| 3 | 后端 Python → Rust | C | 全新实现；可复用的是**行为**（以测试固化），代码不可复用 |
| 4 | 前端 JS → TypeScript（strict）+ Vite | C | 现在是单个 865 行的经典 `<script src="app.js">`，无构建、无模块、全局变量（`web/index.html`） |
| 5 | Nginx 作为反向代理与静态服务 | C | 现在没有；cloudflared 直连容器端口（`docker-compose.yml:26`，README:742-761） |
| 6 | 保留 SQLite 与全部数据 | A / B | 同一 schema、`journal_mode=DELETE`、`synchronous=FULL`（`db.py:93-95`）→ 见 D1/D2 |
| 7 | 保留全部功能、API 行为、安全控制、业务规则 | A（原则）→ 与 D3/D6/D7 有交集 | 见各 D 项 |
| 8 | 绝不为架构整洁牺牲功能；不得假设，先读源码 | A（过程） | 本文即此 |
| 9 | 目标目录树：`frontend/{src/{api,components,pages,types},public,package.json,vite.config.ts}`、`backend/{src/{api,services,parser,database,security},Cargo.toml}`、`nginx/nginx.conf`、`data/<旧名>.db`、`docker-compose.yml`、`docs/` | C；其中 `data/<旧名>.db` 为 D1；与根目录现有 `web/ app/ server.py` 并存问题为 E4 | 仓库现有 `docs/` 只有 `docs/screenshots/`（12 张 PNG，`git ls-files`） |
| 10 | 允许额外文件（models/config/error.rs/tests/Cargo.lock；utils/styles/tsconfig/package-lock；docs 七篇） | B | 无冲突 |

### 2.2 技术栈

| # | 规格要求 | 类 | 证据 / 说明 |
|---|---|---|---|
| 11 | 前端 TS strict、Vite、HTML5/CSS3、原生 DOM，禁 React/Vue/Angular | C | 现状本来就是原生 DOM（`h()` 助手，`app.js:27-41`），移植风险低 |
| 12 | 集中的类型化 API 客户端 `api/client.ts, bank.ts, import.ts, progress.ts` | C | 现在 `post()`（:85）+ 散落的 `fetch` 共 9 处调用 |
| 13 | 集中类型；"OpenAPI 与 TS 类型生成" | C → E10 | 新工具链，选型见决策表 Q25 |
| 14 | 加载状态、友好错误 | A / B | 已有 `loading` / `fatal` 视图（`app.js:838-841`）、中文错误提示 |
| 15 | 前端只经 REST 与后端通信；无 DB 访问 | A | 全仓库 `fetch` 目标均为 `/api/*` |
| 16 | 前端**无业务逻辑**、无可信 token 生成、无权威校验 | **D3** | `grade()`（:149-154）、`inNotebook()`（:109-112）、`pool()`、`stateOf()` 都在浏览器；服务端 `clean_attempt`（`server.py:132-145`）只收 `{id, correct}`，**不验证对错** |
| 17 | 后端 Rust stable、Axum、Tokio、Serde、rusqlite、tracing、thiserror、anyhow、zip、quick-xml | C | 本机无 cargo/rustc → 见 Q27；`xlsx_parser.py:216` 用了后行/前行断言（唯一一处），Rust 标准 `regex` 不支持 → 需要 `fancy-regex` 或手写 |
| 18 | rusqlite 不得阻塞 async 执行器；清晰的连接所有权 | C | 现状：一个连接 + 一把 `RLock` 串行化所有访问（`db.py:88-89`, `100-110`）；Rust 里用专用线程 / `spawn_blocking`，语义保持"串行" |
| 19 | Nginx：唯一入口、托管前端、代理 `/api/`、`client_max_body_size 20m`、安全头、超时、日志、无目录列表、SPA fallback、只转发必要头、不盲信 XFF/Host/Proto | C | 现状仅发送 `X-Content-Type-Options: nosniff` 与 `Cache-Control: no-store`（`server.py:204-205`）；**没有**任何地方读取 `X-Forwarded-*`（grep 无结果）。无路由 / `pushState`（grep 无结果）→ SPA fallback 实际不需要 |
| 20 | 不得停用现有 Host/Origin 安全控制 | A → 接口见 E5 | `server.py:213-234`；Nginx 必须原样转发 `Host`，见 3.4 |
| 21 | 数据库：保留 schema、`journal_mode=DELETE`、`synchronous=FULL`；schema 变更需论证 + 迁移脚本 + 可逆；不得静默重建 | A / B | `db.py:26-71, 93-97`；schema 版本 `meta.schema = 1`（:22, 97），**没有迁移框架**；默认零 schema 变更 |

### 2.3 必须保留的行为

| # | 规格要求 | 类 | 证据 / 说明 |
|---|---|---|---|
| 22 | API：`GET /api/bank, /api/progress, /api/info, /api/imports`；`POST /api/upload, /api/import/confirm, /api/import/cancel, /api/attempt, /api/reset` | A；**规格漏列** `POST /api/shutdown`（`server.py:367-372`）、`HEAD`（:236-242）、静态文件路由（:260-270）和 `--import` 命令行（:398, 403-421）→ E7 | 精确格式由 api 维度给出；这里只记差异 |
| 23 | 新后端与现有前端契约向后兼容 | B | 含"JSON 里冒号后有空格"这种可观察细节，见 3.7 |
| 24 | XLSX：单选 / 多选 / 热点题 / 答案 / 解析 / 口诀 / 日语 / 中文翻译（B 列）同一结构 | B | `xlsx_parser.py:286-358`；翻译 B 列 → `parse_translation`（:257-283） |
| 25 | 翻译解析失败时中文切换不得暴露错位内容 | A | `parse_translation` 对不上返回 `None`（:262-283）；前端 `hasZh = !!q.zh`（`app.js:589`）置灰按钮 |
| 26 | 处理 ZIP 结构、共享字符串、sheet 关系、命名空间、缺失 / 空单元格、Unicode、畸形文档；**不假设 sheet1.xml** | A / B | 通过 `workbook.xml` + `.rels` 解析 sheet（`xlsx_parser.py:93-107`），跳过隐藏表（:99）；不假设 `sheet1.xml`。私有工作簿结构统计见 3.9 |
| 27 | 无 XML 实体展开 | **D7** | 见 D7 |
| 28 | 无 ZIP 炸弹；有界内存 | C（部分 A） | 只按**声明的**大小检查：单文件 ≤100 MiB，总计 ≤300 MiB（`server.py:78-81`），之后**整个解压进内存**（:83）；我实测见 3.8 |
| 29 | 待确认流程：上传→校验→解析→预览→暂存→确认/取消；只有确认才写库 | A | `server.py:316-332, 346-359`；预览阶段 `plan_import` 只读（`db.py:157-196`） |
| 30 | 30 分钟过期 | A | `PENDING_TTL = 30*60`（:50），在 `take()` 时才检查（:167），**没有后台清理** |
| 31 | 一次性确认码，CSPRNG | A（有一个边角缺陷） | `secrets.token_urlsafe(16)`（:158）= 128 位；`take()` 取出即清空（:169）。非 ASCII token 会让 `compare_digest` 抛 `TypeError`（我实测，见 D6/3.6） |
| 32 | 显式确认 / 取消；预览不写业务数据 | A | 同上；取消**不要求 token**（:357-359） |
| 33 | 确认对并发请求安全、永不应用两次 | A | `take()` 在锁内取出并清空；`apply_import` 在 `BEGIN IMMEDIATE` 事务里（`db.py:100-110, 209-225`）；先 `take` 后 `apply`——写库失败后预览已作废，需重新上传（:347-355） |
| 34 | 应用是一个事务、无部分数据 | A | 同上 |
| 35 | 待确认数据在用户 / 会话间隔离 | **D5** | 见 D5 |
| 36 | 不暴露内部临时数据 | A | 预览只含 `preview_payload`（:177-182）；没有临时文件（上传在内存 `BytesIO` 解析，:325） |
| 37 | 概念函数 parse_upload / validate_import / plan_import / create_pending_import / confirm_pending_import / cancel_pending_import / apply_import | B | 对应：`xlsx_parser.parse_workbook` + `check_xlsx` / `Store.plan_import` / `Pending.put` / `Pending.take` / `Pending.drop` / `Store.apply_import` |
| 38 | 上传校验：只 `.xlsx`；空文件拒绝；旧 `.xls` 拒绝；损坏拒绝；实体攻击拒绝；20 MB | A / E8 | 服务端**不检查扩展名**，只做内容检查（:65-85；OLE2 头 → "旧版 .xls"）；扩展名只在前端检查（`app.js:441`）|
| 39 | Nginx 与 Rust 各自独立地强制 20 MB；不信任 Content-Length；考虑分块与解压后大小 | C | 现状只信 `Content-Length`（:276-290）；**不支持分块**（无 Content-Length → 当 0 字节处理，得到"文件是空的"，按代码阅读推断，未起服务验证）。HTTP/1.0（实测 `protocol_version`） |
| 40 | SQLite：`journal_mode=DELETE`、`synchronous=FULL`；检查 schema/表/索引/约束/外键/事务/初始化/查询/导入与重置行为 | A（交 db 维度） | 注意：`PRAGMA foreign_keys=ON` 但**没有声明任何外键**（`db.py:94`；grep `REFERENCES` 无结果）；`stats` 故意不引用 `questions`；重置只清 `stats` 与 `attempts`（:276-277） |
| 41 | 新实现先在**副本**上工作；开发期间永不删除 / 重建 / 截断 / 覆盖原库 | C（流程） | 默认见第 5 节 |

### 2.4 安全评审范围

| # | 规格要求 | 类 | 证据 / 说明 |
|---|---|---|---|
| 42 | Host 校验 | A | `server.py:213-220`，默认 `{localhost,127.0.0.1,::1}` ∪ `XEXAM_ALLOWED_HOSTS`（:439），`XEXAM_ALLOW_ANY_HOST` 可关 |
| 43 | Origin 校验（写请求） | A | `server.py:222-234`：Origin 主机名 = Host 主机名，或在 `XEXAM_ALLOWED_ORIGINS`；无 Origin 放行 |
| 44 | CORS 配置 | E9（现状：无） | 没有 `Access-Control-*`（grep 无结果）；应用是同源的，Origin 检查充当 CSRF 防线 |
| 45 | 大小限制 | A / C | 上传 20 MiB、JSON 1 000 000 字节（:48-49）；见 39 |
| 46 | Content-Type 校验 | A（宽松） | `'application/octet-stream' in ctype`、`'application/json' in ctype`（:317, 334）是**子串**判断，不符 → 415 |
| 47 | 其它 MIME 校验"在合适处" | E8 | 服务端用魔数（`PK` / OLE2，:67-70）代替 MIME |
| 48 | HTTP 方法校验 | A（实现是 stdlib 副作用） | 只定义 `do_GET/HEAD/POST`；其它方法由 stdlib 返回 **501** + HTML 页（实测：`hasattr(do_OPTIONS)=False`，`handle_one_request` 走 `send_error(NOT_IMPLEMENTED)`）。Axum 默认是 405 → E 级差异，见 Q13 |
| 49 | 路径穿越 | A | `os.path.normpath` + `commonpath`（:260-266），只服务 `web/` |
| 50 | 错误泄露 | **D6** | `server.py:329`、`:355` |
| 51 | 安全头 | C | 现在只有 nosniff + no-store；`Server:` 头会带 Python 版本（`version_string = server_version + ' ' + sys_version`，实测源码） |
| 52 | 代理信任边界 | C | 现在没有代理逻辑；见 3.4 |
| 53 | 上传：扩展名、MIME、ZIP 结构、ZIP 炸弹、XML 实体、解压上限、内存上限、**解析时间上限**、**临时文件清理**、畸形 XLSX | 混合 | 扩展名 E8；ZIP 结构 A；炸弹 部分 A；实体 D7；解压上限 部分 A；**解析时间上限：不存在**（`server.py` grep `timeout` 无结果）；**临时文件：不存在**（全程内存）→ Rust 若保持内存方案则"清理"为 N/A |
| 54 | API：输入校验、SQL 注入、参数化 SQL | A | 全部为 `?` 占位符（`db.py` 全文）；`_removed_with_history` 里的 `IN (%s)` 只拼接 `?` 个数（:198-203），安全 |
| 55 | 授权 / 认证（若有） | A（**不存在**） | `server.py` / `db.py` grep `auth|login|password|cookie|session|bearer|api key` 无结果。信任模型 = 网络位置 + Host/Origin 守卫 + 外部的 Cloudflare Access（README:168, 778；记忆文件同） |
| 56 | token 安全、重放 | A / D6 | 128 位随机、一次性；边角缺陷见 3.6 |
| 57 | 速率限制"在合适处" | E11（现状：无） | grep `rate|throttle` 无结果 |
| 58 | 并发 | A | 单连接 + 全局 `RLock`；`ThreadingHTTPServer`（:426）。实际 DB 访问全部串行 |
| 59 | 数据库：事务、文件权限、日志模式、备份、损坏处理、安全关闭 | 混合 | 事务 A；日志模式 A；安全关闭 A（SIGTERM → `shutdown` → `store.close()`，:454-473）；**备份、`integrity_check`、损坏处理在代码里都不存在**（grep 无结果）；文件权限靠容器用户 uid 10001（Dockerfile:11） |
| 60 | 不发明认证系统；记录信任假设 | A（原则） | 默认：不加认证（Q20） |
| 61 | 不能仅因为是 Rust 就声称生产安全 | A（过程） | 文档措辞要求 |

### 2.5 Docker / 部署

| # | 规格要求 | 类 | 证据 / 说明 |
|---|---|---|---|
| 62 | 两个运行时服务（nginx、backend） | **D4** | 现在 1 个服务（`docker-compose.yml:20-38`） |
| 63 | 前端在镜像构建期构建并拷入 Nginx 镜像 | C | |
| 64 | 后端多阶段构建、小运行镜像、尽量非 root | C（现状 A：非 root uid 10001，Dockerfile:11） | Rust 运行镜像无 Python → 健康检查要重写（Dockerfile:22-23 现在用 `python -c urllib`）；且需带 tzdata（3.5） |
| 65 | SQLite 卷 `./data:/data` | **D2** | |
| 66 | 健康检查、重启策略 | A / C | 现有 `HEALTHCHECK`（Dockerfile:22）与 `restart: unless-stopped`（compose:23）；新增 nginx 的 |
| 67 | 无多余公开端口；专用内部网络 | C | 现在容器端口发布到宿主（compose:24-26，默认 `127.0.0.1:8765`） |
| 68 | 显式环境变量；正确的启动依赖 | B | 现有 `XEXAM_ALLOWED_HOSTS / ORIGINS / ALLOW_ANY_HOST / BIND / DATA / HOST / PORT`、`TZ` |
| 69 | 后端使用 `/data/<旧名>.db` | D1 | 实际 `/data/xexam.db`（`server.py:380`；`smoke-test.sh:63, 83`） |
| 70 | 永不把生产库拷进镜像；镜像无密钥 | 部分 A | `.dockerignore` 排除 `data`、`progress.json`、`*.xlsx`，**但不排除** `*.db`、`*.sqlite*`（`.dockerignore:1-18`），而 `.gitignore` 排除了（:6-9）→ C |
| 71 | 交付 `docker-compose.yml`、后端 Dockerfile、必要时 Nginx Dockerfile、`.dockerignore`、`.env.example` | C | 现在**没有** `.env.example`（`ls -a`）；`.env` 在 `.gitignore:12` |

### 2.6 阶段、数据保护、工程规则

| # | 规格要求 | 类 | 说明 |
|---|---|---|---|
| 72 | Phase 0：`docs/architecture-audit.md, migration-plan.md, risk-assessment.md, test-strategy.md`，之后停下等批准 | 闸门已被业主免除 | 见开头 |
| 73 | Phase 1：对**原版**做基线测试（每个端点 × 方法 × 有效/无效、XLSX 全题型含日 / 中、预览 / 确认 / 取消 / 过期 / 一次性、写库、重置、并发确认、安全校验；夹具；新旧对比） | C；夹具 D8 | 仓库**没有测试**（`git ls-files` 无任何测试文件；唯一带 test 字样的是 `scripts/smoke-test.sh`，CI 用的 Docker 冒烟测试）。业主的临时脚本在仓库外（见 3.10） |
| 74 | Phase 2：`docs/api.zh-CN.md` + OpenAPI；歧义时提问而不是发明 | C | 默认见 Q25 |
| 75 | Phase 3：Rust 后端模块 api/services/parser/database/security、类型化结构、参数化 SQL、显式事务、优雅关闭、tracing、绝不记录 token 或文档内容 | C | 优雅关闭现状 A（:456-473）且 CI 断言 `docker stop` < 9 秒（`smoke-test.sh:87-91`）|
| 76 | Phase 4 前端：保留界面、交互、观感 | C → D9 | |
| 77 | Phase 5/6/7：Nginx+Docker、集成与回归（`cargo fmt --check/check/clippy/test`、`tsc`、`vite build`、compose 端到端、重启后持久化、并发、超限上传、关闭）、文档 | C | |
| 78 | 数据保护：保留 Python 实现可恢复（legacy 目录或分支）；备份 + `PRAGMA integrity_check`；文档化回滚 | C | 提案见第 7 节 |
| 79 | 工程规则：不要过度设计；依赖少且有锁文件；安全关键解析不用无人维护的 crate；API 错误可预测；无堆栈 / 路径泄露；配置代替硬编码 | B / D6 | |
| 80 | 验收：一切真实测试过；新旧不明差异视为缺陷 | A（过程） | 因此第 3 节的"已知差异"必须登记为**有意差异**并配测试 |

---

## 3. D 项（冲突）逐项

每项：事实 → 选项（≥2）→ 建议 → 向业主提的准确问题 → 默认。

### D1 数据库文件名（规格写旧名称 `.db`，实际是 `xexam.db`）
**事实**
- `server.py:380`：`db.Store(os.path.join(data_dir, 'xexam.db'))`；`server.py:3-5, 393` 的文档字符串同；`smoke-test.sh:63, 83` 断言 `/data/xexam.db`；README:310 / 603 同；`使用说明.txt` 同。
- 业主规则（记忆 `xexam-naming-no-old-name`）：旧名称大小写都不能出现；"不要写'回退到旧名称'的代码"。工作区全文 grep 为 0 处；**但历史提交里仍有**：5 个提交中分别 7、10、9、9、8 个文件含旧名称（`git grep` 逐提交统计）。
- 规格里的 `data/<旧名>.db`（目录树与 Docker 两处）→ **按规则视为规格的笔误，报告为差异**，不照写。
**选项**：(a) 一律 `xexam.db`；(b) 照规格写旧名称。
**建议**：(a)。(b) 直接违反业主的明确规则，且现有数据库就叫 `xexam.db`，换名等于"静默重建数据库"（规格自己禁止）。
**问题**：规格里的 `<旧名>.db` 是否应理解为 `xexam.db`？
**默认**：`xexam.db`；新后端的路径 = `$XEXAM_DATA/xexam.db`（默认 `/data`）。另：历史提交里的旧名称无法也不应改写（改写 = 改历史 + 强推），文档只写"旧名称"。

### D2 `./data:/data` 绑定挂载 vs 命名卷 `xexam_data`
**事实**
- 首个提交 `1cb7512` 的 compose 就是 `./data:/data`（`git show 1cb7512:docker-compose.yml`，注释"备份 = 复制这个文件夹"）。
- 提交 `892a47b`（信息："keeps … in the named volume xexam_data, so it needs no other file and survives container updates … CI smoke test … checks that data survives deleting and recreating the container"）**有意**改为命名卷。现在 `docker-compose.yml:28-31, 40-41` 与 `docker-compose.build.yml:19-20, 27-28` 都用 `data:`；README:601-642 的备份 / 恢复 / 迁移步骤、`smoke-test.sh:27-28, 77-84` 的持久化断言都以命名卷为前提。
- `db.py:91-92` 的注释：不用 WAL，因为 Docker Desktop 把 Windows 文件夹挂进容器时共享内存文件不可靠——绑定挂载 Windows 目录正是这个场景（`synchronous=FULL` + `DELETE` 日志在其上也未经验证）。
- 绑定挂载由 Docker 自动创建时属 root，而容器用户是 uid 10001（Dockerfile:11 只对镜像内 `/data` chown）→ 非 root 后端首次写入会失败（README:642 已有同类说明）。
- 本机 Docker 里没有 `xexam_data` 卷（0.3），不知道业主真实库的位置（O1）。
**选项**：(a) 生产用命名卷 `xexam_data`（不变），开发 / 测试用 `./data` 绑定覆盖文件；(b) 全面改为 `./data:/data`，并把现有卷内容拷出来；(c) 两者都支持。
**建议**：(a)。(b) 需要搬动业主的真实数据库（规格里"覆盖数据库"的停止点），且要处理 uid 与 Windows 文件系统风险；规格的"data/ 目录"可由**开发期副本**满足。
**问题**：生产部署继续用命名卷 `xexam_data` 是否可以（`./data` 只用于开发和测试副本）？
**默认**：(a)。卷名、Compose 项目名 `xexam` 不变，所以升级到新栈时同一个卷自动接上。

### D3 "前端无业务逻辑"vs 判分与错题本在浏览器里
**事实**
- 判分：`grade()`（`app.js:149-154`）；错题本规则 `wrong > 0 && streak < masterStreak`（:109-112，`masterStreak` 来自服务端 `progress.masterStreak`，`db.py:23, 253`）；出题范围 `pool()`（:127-130）；打乱 `shuffle()`（:53-60）；成绩百分比 `Math.round(100*good/n)`（:738）。
- 服务端只收 `{id:str(≤64), correct:bool}` 最多 2000 条（`server.py:132-145`），**不知道用户选了什么，也无法验证 `correct`**；README:306 把这写成特性："不会上传你选了什么"。
- `/api/bank` 把正确答案发给浏览器（`q.answer`）——和浏览器判分是同一前提。
**冲突**：规格"前端无业务逻辑 / 无权威校验" + "API 行为不变" + "业务规则不变" 三句无法同时满足；把判分移到服务端必须改 `/api/attempt` 的请求体（要带所选答案），即**改 API**。
**选项**：(a) 判分留在前端（TS 里逐字移植 `grade` 等），在 `docs/architecture.zh-CN.md` / `security.zh-CN.md` 明说"这是单用户信任模型下的既有设计"；(b) 保留 `/api/attempt` 不变，**另加**一个可选的服务端判分端点（只增不改）；(c) 修改 `/api/attempt`。
**建议**：(a)。(b)(c) 都是新功能 / 改契约，且对"自己对自己判分"的单用户场景没有安全收益。
**问题**：是否同意判分和错题本规则继续留在浏览器里（只在文档中标注为已知的设计取舍）？
**默认**：(a)。

### D4 两个服务 / 两个镜像 vs 现在的单镜像、只拉取、一个文件部署
**事实**
- 现在：`ghcr.io/yumin-space/xexam:latest`（`docker-compose.yml:21`），业主可把这**一个文件**复制到任何机器上 `docker compose up -d`（README:455）。`docker-compose.build.yml` 用 `build: .` / `image: xexam:local`（:13-14）。
- CI：`docker.yml` 构建一个镜像 `ghcr.io/<owner>/xexam`，标签 `latest`（仅默认分支，:124）、`v<VERSION>`、`sha-*`；`amd64` 与 `arm64` 两个架构各自测试后合成清单（:43-80, :135-160）。
- 若把新的 nginx 镜像仍叫 `xexam`，已部署的 compose 下次 `docker compose pull` 会拿到一个**没有后端**的镜像，业主现有部署静默损坏。
- 另外：`docker-compose.yml:21` 要求 `docker login ghcr.io`（镜像私有，README:693）；新增的第二个镜像同样私有。
- 把 `nginx.conf` 烘进 nginx 镜像可保留"只需一个 compose 文件"的性质（否则要同时分发 `nginx/nginx.conf`）。
**选项**：(a) 新镜像名 `ghcr.io/yumin-space/xexam-web`（Nginx + 前端静态）和 `ghcr.io/yumin-space/xexam-backend`（Rust），版本标签同步；旧 `xexam` 冻结在 Python v0.2；(b) 沿用 `xexam` 名并换成 Nginx 镜像、后端另起名；(c) 单镜像里同时跑 Nginx 与 Rust（违反规格"两个运行时服务"）。
**建议**：(a)。
**问题**：新镜像用 `xexam-web` / `xexam-backend` 两个新名字是否可以？
**默认**：(a)；nginx.conf 烘进 web 镜像；compose 仍可单文件分发；同时保留 `docker-compose.build.yml` 的等价物（两个 `build:`）。

### D5 "待确认导入按用户 / 会话隔离" vs 全局唯一槽位
**事实**
- `Pending` 只有一个 `self.item`（`server.py:150-175`），`put()` 直接覆盖（:160），**只保留最近一次**（类文档字符串 :151）。
- 没有用户、没有会话、没有 Cookie（见 55）；`/api/import/cancel` **不验证 token**，无条件清空槽位（:357-359）；前端关闭管理页时也会发 cancel（`app.js:430`）。后果：开两个浏览器标签，后一个上传会使前一个的预览失效（确认时得到 409 "预览已经过期或已被处理"，:349）。
- 确认 / 取消互斥靠 `threading.Lock`（:154）。
**选项**：(a) 逐字保持单槽位 + 无 token 取消；(b) 以 token 为键的映射（可同时存在多个待确认项，各自 30 分钟过期；`cancel` 带 token 时只取消自己的）——只增不改，但改变了"只保留最近一次"的规则和 `cancel` 语义；(c) 基于 Cookie 的会话隔离——等于发明会话系统，规格明确不让。
**建议**：(a)，并在文档如实说明"隔离 = 无；同一时间只有一个待确认项"；若业主确实要多标签并发，再做 (b)。
**问题**：待确认导入是否保持"全局一个槽位"这一现有行为？
**默认**：(a)。同时把边角缺陷（非 ASCII token → 当前会抛 `TypeError`）修成返回 409，登记为"有意差异"。

### D6 "不泄露错误细节"vs 现有行为，以及"中文错误文案即 API"
**事实**
- `server.py:329`：`'读取这个 Excel 时出错：%s: %s' % (type(e).__name__, e)` 把任意异常类名 + 文本返回浏览器；`:355`：`'写入数据库失败…：%s' % e`。
- 前端原样展示 `d.error`（`app.js:454, 486`）；403 的拒绝文案含它看到的 Host / Origin（已去控制字符并截断到 80 字符，`server.py:294-305`，是业主要求的"告诉我怎么放行"，README:782）。
- 我实测（提取 `Pending` 类运行）：有待确认项时 `take('é…')` 抛 `TypeError`（`compare_digest` 不支持非 ASCII `str`）。在真实服务里这会成为未处理异常，连接被关闭（按 stdlib 行为推断，未起服务验证）。
**选项**：(a) 逐字保留全部文案与两处异常回显；(b) 保留所有**确定性**的中文文案（ValueError 类、403 的 Host/Origin 提示、413 等），把两处"任意异常回显"改成固定中文文案 + 服务端 `tracing` 记录详情；(c) 全部改英文通用文案。
**建议**：(b)。(c) 会改变 UI 可见行为；(a) 违反规格"无堆栈或路径"。
**问题**：把两处"任意异常文本回显"改成固定文案，其余中文错误文案逐字保留，可以吗？
**默认**：(b)；差异登记在 `docs/migration.zh-CN.md` 的"有意差异"。

### D7 "禁止 XML 实体展开"vs 现有检查可绕过
**事实**（`ev_checks2.py`、`ev_entity.py`，stdlib，均为实测）
- `check_xlsx` 只对 `.xml` / `.rels` 做**字节级**子串搜索 `b'<!ENTITY'` / `b'<!DOCTYPE'`（`server.py:82-85`）。
- 把 `xl/workbook.xml` 编成 **UTF-16**（带 DOCTYPE + 内部实体）：UTF-8 版被拒绝，**UTF-16 版被接受**。
- 随后 `xml.etree.ElementTree.fromstring`（expat 2.5.0）对该 UTF-16 文档**确实展开了内部实体**（嵌套两层，输出 16 个字符）。
- README:808 宣称"拒绝含 XML 实体声明的文件"，该说法对 UTF-8 成立，对 UTF-16 不成立。
- 业主的两份私有工作簿：workbook.xml 编码均为 UTF-8，全部部件无 DOCTYPE / ENTITY（见 3.9 的结构统计），不受影响。
**选项**：(a) Rust 解析层对**所有**编码在 XML 解析阶段拒绝 DTD / 实体声明（`quick-xml` 遇到 `DocType` 事件即报错），文案沿用现有中文提示；(b) 逐字复刻字节搜索（连同缺陷）。
**建议**：(a)，登记为"有意差异（修缺陷）"，并加 UTF-16 测试用例；"新旧对比"表里把这一项标为 *old accepts / new rejects*。
**问题**：Rust 版对 UTF-16 编码的实体声明改为拒绝（旧版会接受），可以吗？
**默认**：(a)。

### D8 测试夹具 vs 忽略规则
**事实**
- `.gitignore:4-9` 忽略 `*.xlsx *.xls *.db *.db-journal *.sqlite *.sqlite3`；实测 `git check-ignore -v tests/fixtures/demo.xlsx` 被 `.gitignore:4` 命中，`tests/fixtures/a.db` 被 `:6` 命中。→ 夹具会被**静默地**不提交。
- `.dockerignore:3` 忽略 `*.xlsx`；README:8 明确"仓库里没有题库"。
- 业主两份真实工作簿在 `<仓库的上一级目录>`（仓库**外**）。虚构演示工作簿在 `…/xexam_shots/files/demo.xlsx`（2 828 字节）、`demo_v2.xlsx`（2 918 字节），由 `make_demo.py` / `make_demo_v2.py`（仅用 `zipfile` + `xml.sax.saxutils`）生成。
**选项**：(a) 提交**虚构**夹具的生成脚本 + 生成结果，并在 `.gitignore` 里加反向规则 `!tests/fixtures/**`；真实工作簿只在本地、只读、只对比聚合数字（计数、id 哈希）；(b) 夹具全由测试代码在运行时生成，不提交二进制。
**建议**：(a)+(b) 混合：基础夹具运行时生成（脚本入库），少量回归用的小型虚构 `.xlsx` 入库（带反向规则）。真实工作簿**永不入库、永不出现在日志里**。
**问题**：测试用虚构夹具入库（含 `.gitignore` 例外规则）、真实工作簿只在本地做新旧对比，是否同意？
**默认**：同上。

### D9 Vite / ES 模块 vs 业主的截图与验证工具
**事实**
- 工具在仓库外：`xexam_shots/{shoot.py,scenes.py,make_icon.py,filesrv.py}`，`xexam_ci/{host_test.py,tunnel_sim.py,mm_check.py,…}`。
- `scenes.py` 通过 CDP 在页面里 `evaluate` 调用页面**全局**：`S.session`(13 次)、`render`(7)、`S.admin`(6)、`clearSaved`(4)、`S.view`(4)、`S.result`(4)、`S.progress`(4)、`openAdmin`(2)、`S.cfg`(2)、`S.lang`、`S.bank`、`LS_LANG`；以及 CSS 选择器 `.bubble .chip .hs-item .inline label .lang-btn .opt .pfoot .btn.primary .pill .rail .rv .rvhead .score .big .stem p`。Chrome 路径硬编码 `C:/Program Files/Google/Chrome/Application/chrome.exe`，CDP 端口 9333。
- Vite 打包后这些顶层 `const S` / 函数成为模块作用域，`evaluate` 够不到；类名若改，截图脚本也会失效。
**选项**：(a) 新前端保持**相同的 CSS 类名与 DOM 结构**，并在测试 / 截图构建里暴露一个 `window.__xexam = {S, render, …}` 钩子（生产构建不带）；(b) 重写场景脚本，只通过点击驱动 UI（同时变成端到端测试）。
**建议**：(b) 为主，(a) 作过渡；用同一份虚构演示题库对新旧 UI 做截图对比，作为"保留观感"的验收证据。
**问题**：是否同意把截图 / 验证工具迁入仓库（`tools/`），改成只靠点击驱动？
**默认**：(b)+(a)；CSS 类名与 DOM 结构原样保留；截图仍写到 `docs/screenshots/`。

---

## 3.x 其它会造成"同输入不同输出"的细节（新旧对比时必须逐项处理）

### 3.1 成绩百分比的舍入（也是 E 项，见 Q15）
- 服务端存库：`round(100 * good / total)`（`db.py:269`）= Python 3 **银行家舍入**（四舍六入五成双）。
- 浏览器成绩页：`Math.round(100 * good / n)`（`app.js:738`）= 半入。
- 实测 8 题答对 1/3/5/7 题：Python 存 12 / 38 / 62 / 88，JS 显示 13 / 38 / 63 / 88；Rust 的 `f64::round()`（半远离零）会得到 13 / 38 / 63 / 88。
- 即：**首页历史显示的是库里的数（`app.js:393, 398`），成绩页显示的是本地算的数**——现在就有不一致（既有缺陷）。新旧"同输入同输出"要求 Rust **复刻银行家舍入**。

### 3.2 题目 id 的稳定性（最高风险的业务逻辑）
- `make_id`（`xlsx_parser.py:245-247`）：`sha1(sheet + '\n' + 去掉全部空白(stem)[:300] + '|' + 去掉全部空白(first)[:200])[:12]`。id 是 `stats.question_id` 的唯一关联键；id 变了 = 该题的全部答题记录被孤立（`questions` 里变成"新题"，旧题"下架"，README:268-270）。
- 依赖 Python `re` 的 `\s`：实测它匹配 U+3000、U+00A0，**也匹配 U+001C**；`str.strip()` 同样会剥 U+001C（实测）。Rust `regex` 的 `\s` 是 Unicode `White_Space`，**不含 U+001C-U+001F**。
- 切片 `[:300]` / `[:200]` 是按**码点**；Rust 必须按 `char` 切，不能按字节。
- 另：Python 的 `\d` 会匹配全角数字（实测 `int('１２') == 12`，`NUM_PREFIX` 因此接受 `１．`）；Rust `regex` 的 `\d` 在 Unicode 模式下同样匹配 Nd，但需要测试逐一对照。
- `xlsx_parser.py:216` 是唯一用了前后断言的地方 → `fancy-regex`（依赖选择，需论证）或手写。
- **验收办法**：用业主两份工作簿 + 虚构夹具，对比新旧解析出的 id 集合（只比较数量与哈希，不输出文字）。

### 3.3 `_q_hash` 与"更新 vs 未变"计数
`_q_hash`（`db.py:78-80`）对 `json.dumps(sort_keys=True, ensure_ascii=False)` 做字符串比较；库里存的 JSON 是 Python 默认分隔符（`, ` / `: `，`db.py:213`）。Rust 应在 `serde_json::Value` 层比较，而不是比较字符串，否则预览里全部题目都会变成"更新"。

### 3.4 Host / Origin 与 Nginx 的接口
- Host 守卫只看 `Host` 头（`server.py:220`），Origin 守卫比较 Origin 主机名与 `Host` 主机名（:233）。Nginx 的 `proxy_pass` 默认把上游 `Host` 改写为 `$proxy_host`（后端容器名）→ 若不显式 `proxy_set_header Host $http_host;`，所有请求都会被后端 403。
- 若 Nginx 自己托管静态文件，`/`、`/assets/*` 不再经过 Host 守卫（现在 GET 静态文件也受保护，`server.py:245`）。静态文件本身是公开代码，API 仍受保护；差异需登记（Q20）。
- 后端健康检查：现在 `GET http://127.0.0.1:8765/api/info`（Dockerfile:22-23）因 `127.0.0.1` 在白名单里通过；Nginx 或 Docker 从别的容器以 `backend:PORT` 访问时 Host 不在白名单 → 403。需要健康检查显式带 `Host: localhost`，或后端提供免 Host 检查的 `/healthz`（后者是新端点，默认选前者）。
- cloudflared 现在连的是宿主机 `IP:8765`（README:761）。默认保持"宿主机发布 `127.0.0.1:8765` → nginx 容器"，业主的隧道配置、手机书签和 **localStorage（按 origin 隔离，`xexam.session.v1`、`xexam.lang.v1`，`app.js:4-5`）**都不用动。

### 3.5 时区
`now()` 用本地时间 `'%Y-%m-%d %H:%M'`（`db.py:74-75`，无时区）；compose 设 `TZ: Asia/Tokyo`（`docker-compose.yml:32`）。精简 Rust 运行镜像若没有 tzdata，会**静默变成 UTC**，历史记录时间差 9 小时。→ 运行镜像带 tzdata 或自行换算，并加测试。

### 3.6 token 比较的边角
见 D5/D6：`secrets.compare_digest` 对非 ASCII 抛 `TypeError`；无待确认项时 `not it` 先短路返回 409（实测）；错误 token 不会作废槽位（实测：`take(错误)` → `None`，槽位仍在）。Rust 用常量时间字节比较，并保持"错 token 不消耗槽位"。

### 3.7 JSON 序列化的可观察细节
`smoke-test.sh:49` 用 `grep -q '"bankCount": 0'`，`:51` 用 `grep -q "\"version\": \"$want\""`，依赖 Python `json.dumps` 的 `": "` 空格。`serde_json` 默认输出紧凑形式 `"bankCount":0`，会让这两条断言失败（需改脚本，或让断言对空格宽容）。另：Python `ensure_ascii=False`（`server.py:211`）直接输出非 ASCII 字符，serde_json 默认也如此。

### 3.8 ZIP 炸弹检查实测（`ev_checks.py`，Python 3.10，tracemalloc）
| 场景 | 结果 | 峰值内存 |
|---|---|---|
| 一个 60 MiB 全 `a` 的 `.xml`，文件仅 61 393 字节，声明大小如实 | `check_xlsx` **接受** | 144.2 MB |
| 同一文件，把声明大小篡改为 10 | `BadZipFile: Bad CRC-32` 拒绝（解压之后才拒绝）| 144.1 MB |
结论（仅限这两次测量）：61 KB 的上传能让**检查阶段**就占用约 144 MB；声明大小检查并不能限制实际分配。更大规模（接近 20 MB 压缩流）我没有测，不做断言。Rust 版需要**流式、带上限的解压**（按实际输出字节数中止）。

### 3.9 业主两份私有工作簿的结构（仅计数，未输出文字）
| | 工作簿 1 | 工作簿 2 |
|---|---|---|
| 工作表 | 2（0 隐藏） | 2（0 隐藏） |
| 工作表部件 | sheet1.xml、sheet2.xml | 同 |
| 总行数 | 53 | 53 |
| A 列单元格 | 53（50 共享串 + 3 数值） | 53（同） |
| B 列 | 28（25 共享串 + 3 数值） | 53（25 共享串 + 3 数值 + **25 inlineStr**） |
| C 列 | 28（同上） | 53（25 共享串 + 3 数值 + **25 inlineStr**） |
| D 列 | 27 个数值（解析器不读 D 列，`xlsx_parser.py:387-390`） | 27 个数值 |
| 公式 / DOCTYPE / ENTITY / rPh / 图片 / calcChain | 无 | 无 |
| workbook.xml 编码 | UTF-8 | UTF-8 |
两份合起来恰好覆盖"共享字符串"与"inlineStr"两条解析路径，是新旧对比的理想本地夹具；**只在本地只读使用**，不入库。

### 3.10 业主的临时测试脚本（不在仓库里）
`xexam_ci/host_test.py`：`ROOT` 硬编码为 `<仓库根目录>`，`import server as S` 直接调用 Python 纯函数（`host_name`、`origin_host`、`parse_allowed_hosts`），再用 `subprocess` 启动 `server.py`（端口 8921 起）发 HTTP 请求。grep 到 40 行 `ok(` 调用（运行时数量可能更多，协调者说 55，我没有运行它所以不核实）。`tunnel_sim.py`：HTTPS 代理（8943）→ `server.py`（8941），真实 Chrome。这些可以改造成"对任意 base URL 的 HTTP 层测试"重用于 Rust；纯函数部分必须在 Rust 里重新实现为单元测试。

### 3.11 其它小事实
- HTTP/1.0、每请求一连接（实测 `protocol_version`）。`Server:` 头泄露 Python 版本。
- `HEAD` 与 `GET` 同路径（`server.py:236-242`）。
- 端口自动递增：占用时依次试 `port … port+19`（:423-429），只对非 Docker 的本机运行有意义。
- JSON 体上限 1 000 000 字节（:48），超限 413 `{'error':'too large'}`（:337-338）；上传超限 413 中文提示（:321），且会先丢弃最多 4×上限的剩余数据（:282-288）。Nginx 的 `client_max_body_size 20m` 超限会返回 **HTML** 413；前端对非 JSON 响应只显示 `上传失败（HTTP 413）。`（`app.js:452-454`）→ 需要 `error_page 413` 返回同样的 JSON 与中文文案（按 Nginx 文档推断，未验证）。JSON 端点应单独限 1 MB。
- Nginx 默认缓冲请求体到磁盘临时目录（按 Nginx 文档；本机无 nginx，未验证）→ 非 root + 只读根文件系统时要给 `client_temp` 一个可写 tmpfs，这也是"临时文件清理"的一部分。
- 构建上下文：Windows 工作区是 CRLF（0.1）。本地 `docker build` 会把 CRLF 文件拷进 Linux 镜像；Shell 脚本 / `nginx.conf` / 入口脚本必须保持 LF（CI 从 git checkout 构建，无此问题）。

---

## 4. E 项（歧义）逐项

### E1 数据库初始化与"在副本上工作"
`Store.__init__` 对任何打开的库都会执行 `PRAGMA journal_mode=DELETE`、`CREATE TABLE IF NOT EXISTS …`、`INSERT OR IGNORE INTO meta`（`db.py:93-97`）。即使"只读"地用新后端打开真实库，也会短暂取得写锁。→ 所有测试必须先复制 + `PRAGMA integrity_check`。**问题见 O1**。

### E2 `progress.json` 迁移
`open_store`（`server.py:378-388`）在 `data_dir` 与 `APP_DIR` 两处找 `progress.json`，只迁移一次（`meta.legacy_migrated`，`db.py:283`）；`has` 已有记录则不覆盖（:295-297）。规格没提。
选项：(a) 在 Rust 里移植；(b) 删除。建议 (a)（≈30 行，保持"功能不丢"）。默认 (a)；`APP_DIR` 概念消失，只在数据目录找。

### E3 `--import` 命令行
`server.py:398, 403-421`，README:704 公开的功能；规格的 API 列表与目录树都没有。默认：Rust 二进制提供 `xexam-backend import <file.xlsx>` 子命令（复用 `apply_import`）。

### E4 仓库布局与 legacy
见第 7 节。

### E5 Nginx 是否也要执行 Host 白名单
选项：(a) 只由后端强制（单一事实来源，Nginx 只透传 `Host`）；(b) Nginx 再强制一遍（需要把 `XEXAM_ALLOWED_HOSTS` 渲染进 `nginx.conf`，两处会漂移）。建议 (a)。默认 (a)。

### E6 版本号、CI 与 `VERSION` 规则（业主规则：每次更新都写更新日志并带版本）
**事实**
- `docker.yml:36-40`：`VERSION` 必须匹配 `^[0-9]+\.[0-9]+(\.[0-9]+)?$`，且 `README.md` 里第一个匹配 `^### v[0-9]` 的标题必须等于 `VERSION`（README:650 现为 `### v0.2 — 2026-11-10`；:679 的注释写着格式约定）。
- 触发：`push` 到 `main` 与对 `main` 的 `pull_request`（:3-7）；PR 不发布（:86）。
- 后端版本现在运行时读 `VERSION` 文件（`server.py:37-47`），经 `/api/info` 返回（:252）；前端页脚显示它（`app.js:372`）。
- 新仓库会多出 `frontend/package.json` 与 `backend/Cargo.toml` 两个版本号来源，且它们要求三段式 semver，而 `VERSION` 允许两段（`0.2`）。
**选项**：(a) 根 `VERSION` 保持唯一事实来源，CI 的 `check` 作业扩展：规范化 `0.3` → `0.3.0` 后与两个清单比对；(b) 清单固定 `0.0.0`，构建时注入；(c) 多个版本各自演进（违反业主规则）。
**建议**：(a)。
**版本号本身**：整个现代化作为一个发布 **v0.3**（Python 线的最后一个是 v0.2）；每个合并到 `main` 的 PR 仍要有更新日志条目。
默认：(a)，现代化 = v0.3，更新日志用中文，格式 `### v0.3 — 日期`；新增的 README 小标题不得以 `### v` + 数字开头且位于更新日志之前。

### E7 规格未列出的现有功能：`POST /api/shutdown`
`server.py:367-372`，仅 `--allow-shutdown`（`start.bat` 用）时可用；Docker 里返回 403 `{'error':'在 Docker 里请用 docker compose stop 停止容器。'}`；前端只在 `info.canShutdown` 为真时显示按钮（`app.js:373`）。默认：保留路由，Docker 栈里恒返回同一个 403，`canShutdown` 恒 `false`。

### E8 "只接受 .xlsx"落在哪一层
服务端无扩展名检查，用魔数 + `xl/workbook.xml` 存在性判断（`server.py:65-76`）；`.xlsm` / `.xltx`（同为 OOXML）会通过；文件名只做清洗后存进 `imports.filename`（:124-127）。选项：(a) 保持内容检查为权威；(b) 若带了 `X-Filename` 且扩展名不是 `.xlsx` 就 400。建议 (a)（无 `X-Filename` 的 curl 用法照常工作）；默认 (a)，文档注明。

### E9 CORS 与 OPTIONS
现状无 CORS 头，也不响应 `OPTIONS`（501）。应用同源。默认：不发 CORS 头；`OPTIONS` 与其它未支持方法返回 405 + `Allow`（登记差异，原为 501）。

### E10 OpenAPI 与 TS 类型生成
选项：(a) 手写 `docs/openapi.yaml`（以 Python 黄金响应为准）+ `openapi-typescript` 生成类型 + 契约测试验证真实响应；(b) Rust 里用 `utoipa` 从代码生成 spec（多一个依赖，spec 跟随实现而不是契约）；(c) 手写 TS 类型，不生成。建议 (a)。

### E11 速率限制
现状无。所有流量都来自 cloudflared 一个源地址，按 IP 限流没有意义，只能做全局限流。默认：Nginx 对 `/api/` 宽松限流（例如 10 r/s，burst 30）、对 `POST /api/upload` 更严（例如 1 r/s，burst 5），后端本身不限；文档注明"单用户，Cloudflare 才是真正的防线"。

### E12 解析时间上限
现状无。Rust 里无法强杀阻塞线程，只能协作式：解析循环每 N 行检查 deadline，并限制总行数 / 单元格数 / 单部件解压大小。默认 `XEXAM_PARSE_TIMEOUT_SECS=30`，单部件 ≤100 MiB、总 ≤300 MiB（与现状一致），超时返回 400 中文提示。

### E13 文档语言与 README 的角色
现状：README、使用说明、代码注释、UI、错误文案全是中文；README 有 7 张 Mermaid 图（:111-440）。规格的 `docs/` 文件名全是英文。默认：文档正文中文、标识符 / 路径 / 命令英文；README 继续是入口与更新日志的唯一位置（业主规则），其中 Mermaid 图在 Phase 4 更新；`docs/*.md` 放详细内容。

### E14 Windows 开发与 `start.bat`（"不用 Docker"的运行方式）
**事实**：`start.bat` 检查 `python`，运行 `python server.py --allow-shutdown %*`（:8-19）；README:697-707、使用说明.txt:27-28, 83 都把它列为"方式 B"；它自首个提交就存在，`data/` 目录（仓库根）就是它的数据位置。规格没有等价物，目标栈（Nginx + Rust）也没有"双击即用"的形态。
**选项**：(a) 删除（功能丢失）；(b) `start.bat` 随 Python 实现移入 `legacy/python/`，继续可用（`cd /d "%~dp0"` 决定它与 `server.py` 同目录即可，:4）；两种实现共用同一 schema，数据库互相可读；(c) 为新栈提供 `scripts/dev.bat`（起 Rust 二进制 + Vite dev server）并另提供"仅 Docker"的说明。
**建议**：(b)+(c)：旧启动器保留在 legacy 里；新栈 Windows 开发脚本在 `docs/development.zh-CN.md` 给出。**不删除任何功能**。
默认：(b)+(c)。

---

## 5. 规格里的"停止点" → 迁移步骤映射

说明：这个清单来自任务说明，不是我在压缩版规格里逐字读到的。"闸门已免除"仅指 Phase 0 之后继续、自行安装工具、最后上传 GitHub；下面各项**没有**被业主明说免除，所以默认做法是**设计成不触发**。

| 停止点 | 会触发它的具体步骤 | 默认做法（使其不触发） |
|---|---|---|
| 删除源码前需批准 | ① 把 `server.py app/ web/ Dockerfile compose start.bat` 从根目录移走（第 7 节）；② 删 `web/app.js` 被 TS 取代；③ 删根 `Dockerfile` / `docker-compose*.yml` | **从不删除**：先打 tag + 建分支（见第 7 节），再用 `git mv` 移到 `legacy/python/`，历史保持连续；旧文件不 `rm` |
| 覆盖数据库前需批准 | Phase 1/2/3/5 的任何测试；把新栈指向业主的真实卷 `xexam_data`；`reset`、`import` 的测试 | 所有测试只用 `Temp` 下**经 `integrity_check` 的副本**或全新空库；测试 compose 不挂载真实 `data/` 或 `xexam_data`；不替业主执行"新栈切到真实卷"这一步，只写入 `docs/migration.zh-CN.md` |
| 改变业务规则前需批准 | 3.1 舍入；D5 单槽位；D6 错误回显；D7 UTF-16 实体；E8 扩展名；E12 新增上限 | 默认**复刻旧行为**（舍入、单槽位、扩展名）；仅 D6 / D7 / 非 ASCII token 三项作为"修缺陷"的**有意差异**，逐项登记 + 配测试 |
| 删除功能前需批准 | `/api/shutdown`、`--import`、`progress.json` 迁移、`start.bat`、`HEAD` | 全部保留（E2/E3/E7/E14） |
| 暴露后端端口前需批准 | Phase 4 compose；集成测试要直连后端 | 生产 compose 只 `expose`、不 `ports`；测试用单独的覆盖文件，仅绑定 `127.0.0.1` 且只在测试 / CI 使用，不进生产 compose |
| 公开部署前需批准 | "做完上传到 GitHub" → 推到 `main` 会触发 `publish` 作业，发布镜像并移动 `latest`（`docker.yml:86, 124, 135-160`）| 推到分支 `modernize/rust-ts` 并开 PR（PR 不发布）；不合并、不推 `main`；新镜像用新名字（D4），所以将来合并也不会改变业主现有 compose 拉到的东西。**不**替业主在任何机器上部署，不改 Cloudflare 配置 |
| 改变访问模型前需批准 | 引入 Nginx；`XEXAM_BIND` 的含义；放开 Host / Origin；加认证或 Cloudflare Access JWT 校验 | 模型不变：应用无登录、默认只绑定本机、Host/Origin 守卫保留且可配置（记忆 `xexam-deployment-cloudflare`）；不引入认证；不默认打开 `ALLOW_ANY_HOST`；Nginx 不盲信 `X-Forwarded-*`，也不转发它们（后端从不读，grep 为空）|

---

## 6. 文档集：规格要求 vs 现有

**现有**：`README.md`（812 行：截图、7 张 Mermaid 架构图、部署、使用、Excel 格式、备份、更新日志、CI、不用 Docker、Cloudflare、目录结构、安全说明）、`使用说明.txt`（103 行，面向使用者）、`docs/screenshots/*.png`（12 张）。无 `.env.example`、无 LICENSE、无 CONTRIBUTING / `.editorconfig`（`ls -a`）。

**规格要求（11 个文件）与重叠**
| 文件 | 来源 | 与谁重叠 / 建议 |
|---|---|---|
| `docs/architecture-audit.md` | Phase 0 | 与 `architecture.zh-CN.md` 重叠：前者 = **现状**审计快照（冻结）；后者 = **目标**架构 |
| `docs/migration-plan.md` | Phase 0 | 与 `migration.zh-CN.md` 重叠：前者 = 迁移**计划**（分阶段）；后者 = 迁移**执行记录 + 回滚 + 有意差异清单** |
| `docs/risk-assessment.md` | Phase 0 | 与 `security.zh-CN.md` 部分重叠：前者 = 迁移风险；后者 = 安全控制说明 |
| `docs/test-strategy.md` | Phase 0 | 与 `development.zh-CN.md`（含测试命令）部分重叠 |
| `docs/architecture.zh-CN.md`、`api.zh-CN.md`、`database.zh-CN.md`、`security.zh-CN.md`、`deployment.zh-CN.md`、`migration.zh-CN.md`、`development.zh-CN.md` | Phase 7 | `deployment.zh-CN.md` 与 README 的"快速开始 / Cloudflare / 备份"重叠；`architecture.zh-CN.md` 与 README 的 Mermaid 图重叠；`api.zh-CN.md` 在 README 里没有对应 |
| `docs/openapi.yaml`（规格"API 契约 + OpenAPI"） | Phase 2 | 新 |

建议：README 保持入口（简介、快速开始、更新日志），详细内容迁到 `docs/` 并在 README 链接；`使用说明.txt` 保持面向使用者（不变更名称，`.dockerignore` 里已排除，:9）。`docs/screenshots/` 路径被 README 多处引用，不要移动。

---

## 7. 保持 Python 实现可恢复：具体布局提案

**事实**：没有 tag；只有 `main`；CI 只由 `push main` / `pull_request main` 触发，**不**由 tag 触发（`docker.yml:3-7`，grep `tags:` 仅出现在 build 的 `tags:` 参数里）；`latest` 仅在默认分支标（:124）。v0.2 此刻在工作区、尚未提交（另一位正在收尾）。

**提案（按顺序）**
1. **前置**：等 v0.2 提交并推送、CI 发布成功（镜像 `xexam:v0.2` 与 `latest` 存在）后，再开始动结构。我不碰 v0.2 的收尾。
2. 在 v0.2 提交上打**带注释 tag `python-v0.2`**，并建**分支 `legacy/python`**（同一提交）。tag 防丢，分支用于以后热修。不会与镜像标签 `v0.2` 冲突（CI 不看 git tag）。
3. 现代化在分支 `modernize/rust-ts` 上做。**Phase 1–4 阶段不移动任何现有文件**：新代码放进 `frontend/ backend/ nginx/ tests/ tools/ docs/…`，与根目录的 Python 文件并存（规格允许额外文件）。Phase 1 的基线测试就针对根目录里原样的 Python 应用（`XEXAM_LEGACY_ROOT` 指向它）。
4. **切换提交**（Phase 5 末，一个单独的提交）：`git mv server.py app web Dockerfile docker-compose.build.yml scripts/smoke-test.sh start.bat VERSION → legacy/python/`（`VERSION` 在根目录保留一份新的，指向 v0.3）。`legacy/python/docker-compose.yml` 的 `image:` 仍是 `ghcr.io/yumin-space/xexam:v0.2`（固定版本，不用 `latest`）；`legacy/python/docker-compose.build.yml` 的 `build:` 改成 `.`（相对该目录）；`start.bat` 无需修改（`cd /d "%~dp0"`，:4）；`使用说明.txt` 保留在根并加一行指向 `legacy/python/`。
5. **CI**：`.github/workflows/docker.yml` 重写为 web / backend 两个镜像 + 前后端测试 + 扩展后的版本检查；新增 `.github/workflows/legacy-python.yml`，只在 `legacy/python/**` 变动时对 `legacy/python` 做构建 + 冒烟测试，**不发布 `latest`**（只推 `xexam:v0.2.N` 之类，或干脆不发布）。`legacy/python` 分支上保留原 workflow 不变（它只在 `main` 触发，所以不会误发布）。
6. **回滚**：因为 schema 与 `journal_mode` 一律不变，新后端写过的库旧 Python 仍可读写 → 回滚 = 停新栈，`docker compose -f legacy/python/docker-compose.yml up -d` 指向同一个卷 `xexam_data`（项目名同为 `xexam`，容器名同为 `xexam`，因此新旧不能同时运行）。回滚前提：Rust 后端不得向库里写入任何旧版读不懂的东西（新表、新列、改日志模式、user_version 以外的 PRAGMA 持久变化）。
7. **备份**：任何接触真实库之前的备份步骤沿用 README:607-627 的 `tar` 命令，并增加 `sqlite3 xexam.db "PRAGMA integrity_check"`（见 `docs/migration.zh-CN.md`）。

---

## 8. 开放问题（我无法从代码确定的事）

- **O1** 业主的真实 `xexam.db` 在哪里？（本机 Docker 无 `xexam_data` 卷；仓库根有 `data/`，我按规则没读。）"在副本上测试"的副本从哪来？默认：只用虚构夹具 + 两份工作簿现生成的全新库；若业主以后提供真实库的副本，再做二次对比。
- **O2** `docker.yml` 在远端的实际状态：v0.1 的发布步骤没跑成（README:676），v0.2 尚未推送；`xexam:v0.2` / `latest` 是否已存在，我无法核实（`gh` 可能能查，但属于访问远端，超出本任务范围）。
- **O3** Cloudflare 路径从未用真实 Cloudflare 测过（README:774）；新栈同样只能用本地 HTTPS 代理模拟。

---

## 9. 决策表（业主一次答完；每项有未答时的默认）

| # | 问题 | 选项 | 建议 | **默认（将被采用）** |
|---|---|---|---|---|
| Q1 | 规格里的 `<旧名>.db` 是否理解为 `xexam.db`？ | xexam.db / 旧名 | xexam.db | **xexam.db** |
| Q2 | 生产数据存放 | 命名卷 `xexam_data` / `./data` 绑定 | 命名卷；`./data` 仅开发 | **命名卷不变**，`./data` 仅测试副本 |
| Q3 | 判分与错题本留在浏览器？ | 留 / 加服务端判分 / 改 API | 留，文档标注 | **留** |
| Q4 | 镜像命名 | 新名 `xexam-web`+`xexam-backend` / 沿用 `xexam` | 新名；旧 `xexam` 冻结 v0.2 | **新名**；nginx.conf 烘进 web 镜像 |
| Q5 | 待确认导入的隔离 | 全局单槽 / 按 token 的映射 / Cookie 会话 | 单槽 | **单槽，取消无需 token**；非 ASCII token 修成 409 |
| Q6 | 错误文案 | 全逐字 / 固定化两处任意异常回显 / 全改英文 | 固定化两处 | **固定化两处**，其余中文逐字保留 |
| Q7 | UTF-16 实体声明 | 新版拒绝 / 复刻旧缺陷 | 拒绝 | **拒绝**（登记为有意差异） |
| Q8 | 服务端扩展名检查 | 保持内容检查 / 带文件名时要求 .xlsx | 保持 | **保持**（文档注明） |
| Q9 | 分块请求体 | 流式计数并限 20 MiB / 拒绝分块 | 流式计数 | **流式计数**；超限 413 + 同一中文提示 |
| Q10 | 解析时间上限 | 无 / 协作式 deadline | 协作式 | **30 秒**，`XEXAM_PARSE_TIMEOUT_SECS` 可调 |
| Q11 | 速率限制 | 无 / Nginx 宽松全局限流 | Nginx 宽松限流 | **Nginx**：`/api/` 10 r/s burst 30；上传 1 r/s burst 5 |
| Q12 | CORS | 不发 / 白名单 | 不发 | **不发 CORS 头**（同源） |
| Q13 | 不支持的方法 | 405 / 保持 501 | 405 | **405 + Allow** |
| Q14 | 安全头 | 仅 nosniff / 完整集合 | 完整集合 | **nosniff、X-Frame-Options DENY、Referrer-Policy no-referrer、严格 CSP（`default-src 'self'`，样式不用内联）；不设 HSTS**（TLS 在 Cloudflare 终止）。CSP 对 `setAttribute('style')` 的影响需在 Phase 5 用真实 Chrome 验证 |
| Q15 | 成绩百分比舍入 | 复刻银行家舍入 / 改半入 | 复刻，记录既有不一致 | **复刻 Python 舍入**，不修前端显示 |
| Q16 | 时区 | 镜像带 tzdata，`TZ=Asia/Tokyo` / UTC | 带 tzdata | **带 tzdata**，时间格式不变 |
| Q17 | `/api/shutdown` | 保留(Docker 恒 403) / 删 | 保留 | **保留** |
| Q18 | `--import` 与 `progress.json` 迁移 | 移植 / 删 | 移植 | **移植** |
| Q19 | `start.bat` / 无 Docker 运行 | 随 legacy 保留 + 新开发脚本 / 删 | 保留 | **保留于 `legacy/python/`**；新栈 Docker-only，另附 Windows 开发脚本 |
| Q20 | 端口与访问模型 | 保持 `127.0.0.1:8765`→nginx / 改端口 | 保持 | **`XEXAM_BIND` 默认 `127.0.0.1:8765` → nginx 容器**；Host/Origin 守卫留在后端，Nginx 透传 `$http_host`，不转发 `X-Forwarded-*`；无认证 |
| Q21 | Python 留存方式 | tag+分支+目录 / 仅分支 | tag+分支+目录 | **`python-v0.2` tag、`legacy/python` 分支、切换时 `git mv` 到 `legacy/python/`** |
| Q22 | 版本 | v0.3，根 `VERSION` 唯一来源，CI 比对 `Cargo.toml` / `package.json` | 同左 | **v0.3**（清单里写 `0.3.0`） |
| Q23 | 文档 | README 为入口，11 篇 docs，中文 | 同左 | **同左** |
| Q24 | "上传到 GitHub" | 推分支 + PR / 推 main | 推分支 + PR | **推 `modernize/rust-ts` 并开 PR，不合并、不推 main** |
| Q25 | OpenAPI / 类型生成 | 手写 + openapi-typescript / utoipa / 手写类型 | 手写 + 生成 | **手写 `docs/openapi.yaml` + `openapi-typescript` + 契约测试** |
| Q26 | 夹具与忽略规则 | 虚构夹具入库 + 例外规则 / 运行时生成 | 混合 | **混合**；真实工作簿永不入库 |
| Q27 | 缺少的工具（cargo / rustc / nginx） | rustup(MSVC) / GNU 工具链 / 在 Docker 容器里构建与测试 | 优先 Docker 内构建（本机已有 Docker 28.1.1），减少对宿主机的改动 | **先尝试 Docker 内工具链；必要时再装 rustup**（业主已授权自行安装）|
| Q28 | 基线测试的语言 | stdlib `unittest` + `http.client` / pytest | stdlib | **stdlib `unittest`**，基址参数化，可对旧版与新栈跑同一套 |
| Q29 | 截图 / 验证工具 | 迁入仓库并改成点击驱动 / 仍留在 Temp | 迁入 | **迁入 `tools/`**，点击驱动 |

---

## 10. 我没能验证的事（notVerified）

- 没有启动任何服务或浏览器，因此："分块请求体被当作空体""非 ASCII token 在真实服务里导致连接断开""Nginx 的 413 为 HTML 且前端只显示 HTTP 413""Nginx 默认缓冲到临时目录""CSP 会拦 `setAttribute('style')`"都是按代码 / 文档推断，**不是实测**。
- 没有运行 `host_test.py`（其 55 项检查数来自协调者；我 grep 到 40 行 `ok(`）。
- 没有运行 `scripts/smoke-test.sh`（只做了 `bash -n` 语法检查，退出 0；需要 Docker 构建）。
- 没有读 `data/`、`progress.json`，不知道真实库的内容、大小、是否已迁移过 `progress.json`。
- 没有访问 GitHub（Actions 实际结果、`ghcr.io` 上现有标签）。
- 本机没有 cargo / rustc / nginx，所以 Rust 的 `regex` / `fancy-regex` 行为差异是按文档对比 Python 实测值得出的，没有在 Rust 里跑。
- `ZIP 炸弹` 只测了 60 MiB 一档；更大的真实压缩流没有测。
