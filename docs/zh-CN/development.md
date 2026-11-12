[English](../en/development.md) | [日本語](../ja/development.md) | **简体中文**

# 开发

> 日常改代码、跑测试用的命令，以及东西都放在哪里。架构见 [architecture.md](architecture.md)；最初的测试思路见 [test-strategy.md](history/test-strategy.md)。

## 1. 工具

| 工具 | 版本 | 说明 |
|---|---|---|
| Rust | 1.99.1（由 `backend/rust-toolchain.toml` 和 `ai-service/rust-toolchain.toml` 固定） | `rustup` 会自动安装。`rusqlite` 的 `bundled` 特性（以及 ai-service 里的 `ring`）需要 C 编译器（Linux：`build-essential`；Windows：MinGW gcc 或 MSVC） |
| Node | 24 | 前端；单元测试用 Node 自带的测试运行器加 `--experimental-strip-types` |
| Python | 3.8+ | 只用于 `tests/integration/` 里的集成脚本（带虚构的 AI 服务商 `ai_stub.py`）、`scripts/` 里的两个脚本、`tools/icons/` 里的图标工具（**只用标准库**）和 `tools/screenshots/` 里的截图工具（标准库加 `openpyxl`） |
| Docker | Compose v2 | 整栈检查、镜像、截图工具 |
| Chrome 或 Chromium | 较新的任意版本 | 只用于截图工具和图标工具（第 9.1 节和第 9.2 节） |

## 2. 后端

```bash
cd backend
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 382 个测试
cargo run -- serve --data ../data --no-browser     # 监听 127.0.0.1:8765
```

- 测试对着 `tests/baseline/` 里已提交的**旧版快照**运行；不需要 Python。
- 所有测试都用临时目录里的新数据库，或 `tests/baseline/fixtures/sample.db` 的副本，**永远不碰 `data/`**。除了 `src/` 里的单元测试，测试文件有：`api_basic.rs`、`api_import.rs`、`api_misc.rs` 和 `api_golden.rs`（路由、守卫、旧版的响应），`db_golden.rs`（数据库对照旧版快照），`parser_golden.rs`、`parser_hardening.rs`、`parser_four_columns.rs`、`parser_six_columns.rs` 和 `id_vectors.rs`（解析器），以及每个版本一个文件：`v05_banks_records_settings.rs`（schema v2、题库、记录、设置）、`v06_users_and_sign_in.rs`（schema v3、账户、会话、登录限制）、`v065_bank_access_and_practice.rs`（schema v4、题库权限、可管理题库的用户、所有权、练习）、`v07_bank_access_and_layouts.rs`（勾选的题库和 **包括以后导入的题库**、预发布版 v4 数据库的转换、拒绝旧格式和下架它们的题）、`v07_wal_and_logs.rs`（WAL 模式、检查点、服务器运行时的 `reset-password`、系统日志）、`v07_log_floods.rs`（大量被拒绝的请求和失败的登录被汇总）`v08_ai_link.rs`（schema v5、AI+ 功能授权、内部令牌、认证步骤和内部路由；其中一个测试运行真正的程序）和 `v09_ai_study.rs`（schema v6 及其备份、保存的记录的会话 `key` 和 `startedAt` 以及它的 `recordId`，以及内部链接的三个学习路由：会话绑定、AI+ 检查、范围、上限、答错的题的随机抽样、同一个「未找到」）。
- 测试通过辅助代码登录；没有任何开关能关掉登录，测试也不例外。
- **测试里的旧格式。** 正式运行时会拒绝三列和四列格式的工作簿（`services::check_layout`）。黄金测试套件重放旧版的导入，那些都是三列格式的文件，所以它们建立的 API 状态设置了 `old_layouts: OldLayouts::AllowForTests`（在 `backend/tests/api_common/mod.rs` 里，以及 `v05_banks_records_settings.rs` 和 `v065_bank_access_and_practice.rs` 里，这两个文件通过 API 导入已提交的三列格式夹具 `tests/baseline/fixtures/demo.xlsx`）。它是 API 状态的一个字段，只有测试代码会设置：`AppState::new` 和 `import` 命令始终用 `OldLayouts::Refuse`，没有任何环境变量、参数或设置能改变它。`v07_bank_access_and_layouts.rs` 测试正式运行时的规则。存储层的方法本身接受任何题目，所以数据库的黄金测试原样运行。
- 常用的环境变量：`XEXAM_DATA`、`XEXAM_PORT`、`XEXAM_ALLOWED_HOSTS`、`XEXAM_LOG`（日志级别，例如 `debug`）、`XEXAM_SQLITE_JOURNAL`（`delete` 保留回滚日志，不用 WAL）、`XEXAM_COOKIE_SECURE`、`XEXAM_LINK_DIR` 和 `XEXAM_INTERNAL_TOKEN`（和 ai-service 的连接，第 3 节）。完整的列表见 `xexam-backend --help` 和 [deployment.md](deployment.md)。
- `serve` 在 `<数据目录>/link/internal.token`（或 `XEXAM_LINK_DIR/internal.token`）不存在时创建它；其他命令从不碰它。
- 在空的数据目录上第一次启动时，会创建管理员 `admin` / `admin`；第一次登录时页面会要求设置新密码。忘了的话：`cargo run -- reset-password admin --data ../data`（服务器运行时也能用）。
- 命令行导入六列格式的工作簿（导入到旧题库）：`cargo run -- import ../some.xlsx --data ../data`。三列或四列格式的工作簿会被拒绝。
- 让后端打开一个旧 schema 的数据库，会迁移它，并在旁边写一个 `backups/`。请用副本。v0.9 的学习路由（`api/internal_study.rs`，以及 `database/store/study.rs` 里的投影）是带令牌的路由：后端只对 AI 服务回答它们，什么都不写，也什么都不记录。后端也不为它们在测试代码之外写任何中文字面量（`server-messages.test.ts` 会扫描 `backend/src`）。
- 数据库以 WAL 模式运行：后端运行时，`xexam.db` 旁边有 `xexam.db-wal` 和 `xexam.db-shm`。要么三个一起复制，要么先停止后端（之后所有内容都在 `xexam.db` 里，旁边即使留着 `-wal` 文件，也是空的）。后端运行时 `sqlite3` 也能读这个数据库。

## 3. AI 服务（v0.8；学习功能 v0.9）

AI 服务是 `ai-service/` 里一个单独的 crate（包和可执行程序叫 `xexam-ai-service`，库叫 `xexam_ai_service`，没有 workspace，禁止 `unsafe`），有自己的 `Cargo.lock` 和 `rust-toolchain.toml`（Rust 1.99.1，和后端一样）。

```bash
cd ai-service
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 383 个测试（以及 1 个文档测试，`cargo test --doc`：提示的提示词不能接受解答）
cargo tree -e normal --target all | grep -E 'aws-lc|openssl-sys|native-tls'   # 必须什么都不打印（CI 的 TLS 检查）
```

- **结构**（完整的列表见 [architecture.md](architecture.md) 第 2 节）：`src/api/`（分发、用户路由和管理员路由）、`src/chat.rs`（生成回答的任务、每个用户和全局的名额、取消）、`src/app.rs`（共享状态和发给后端的心跳）、`src/provider/`（OpenAI 兼容和 Anthropic 适配器、事件流解析器、错误映射）、`src/store/`（AI 数据库）、`src/identity.rs` 和 `src/link.rs`（身份声明和对后端的调用）、`src/secret.rs` 和 `src/settings.rs`（密钥加密；服务商、URL 规则、指纹）、`src/limits.rs`、`src/prompt.rs`、`src/retention.rs`、`src/logs.rs`、`src/codes.rs`（服务的每一段中文文字）、`src/config.rs`。从 v0.9 起还有 `src/api/study.rs`（四个学习路由）、`src/study/`（提示词 `hint.rs`、`explain.rs`、`summary.rs` 及其渲染，`guard.rs` 和 `guard/{lists,norm,rules}.rs` 里的提示守卫，`title.rs`，`wire.rs` 里后端数据路由的严格类型，`cache.rs` 里的记录身份缓存），以及 `src/store/` 里 `data.rs` 的学习对话函数和 `stats.rs` 的按用途统计的用量。
- **测试**（`ai-service/tests/`，针对库运行，带进程内的假服务商和假后端（它还会记录心跳）；没有一个访问网络）：`streaming.rs`（事件、停止、重试、时间限制、脱离、调用进行中的闸门和上限）、`settings_and_keys.rs`（验证、重新输入密钥、请求头冲突、加密、密钥文件和 `XEXAM_AI_SECRET_KEY`）、`identity_access.rs`（声明、访问级别、范围、令牌规则）、`admin_data.rs`（管理员的查看和清除、审计日志和发件箱、用量、对账）、`logs_no_secrets.rs`（任何日志行里都没有密钥、令牌、提示词或回答）和 `proxy_env.rs`（忽略环境里的 `HTTP_PROXY`）；v0.8 第二轮的有 `r2_core_heartbeat.rs`（报告、它的时间和 60 秒规则）、`r2_core_schema.rs`（schema 2、schema 1 的备份和升级、给缺了几列的 schema 2 文件补上的列）、`r2_core_suspend.rs`（暂停 / 恢复、审计日志、停止正在生成的回答）、`r2_chat_lang.rs`（回复语言）、`r2_chat_free.rs`（自由模式是管理员的状态：`POST /api/ai/free`、`freeMode`、没有按对话的标志）、`r2_free_history.rs`（两种模式下都发送整个历史、每种模式的系统提示词、切换之后的模式说明、每条消息上的 `free`）、`r2_cache.rs`（提示词缓存、用量行里缓存命中的 token、更早的历史消息被缩短）、`r2_chat_thinking.rs`（`thinking` 事件；推理文字不能出现在流、数据库或日志里）和 `r2_chat_suspend.rs`（状态和重新检查）；v0.9 的有 `v09_store.rs`（schema 3：新文件、从 schema 1 和 2 升级及其备份、写不出来的备份、两条路径得到相同的 `sqlite_master`、重新打开什么都不写、已保存回答的查询的 `EXPLAIN QUERY PLAN`、学习对话和提问事务、按用途的用量、分批保留）、`study_guard.rs`（提示守卫：字母、序数、热点题的陈述、答案文字、词表、`<think>` 标签、对流的任何分块都得到同样的结论、上限、持久化规则；模块在 `tests/study_guard/` 里）、`study_prompts.rs`（提示词、`compile_fail` 文档测试）和 `study_api.rs`（对着假后端端到端测试学习路由：流式输出、对话的身份、已保存回答的查询、边界、总结的拒绝、访问、管理员的查看、不含内容的日志、重新生成的备注和 Cloudflare AI Gateway 的缓存绕过（AI 对话的重试也算）；模块在 `tests/study_api/` 里）；再加上 `src/` 里的单元测试。`study_api` 的一些测试依赖短暂的等待（200 毫秒和 120 毫秒），它们是稳定的，但取决于时间。`TestApp` 辅助程序在被丢弃时会关闭服务并释放数据库文件，所以测试不会在临时文件夹里留下任何东西（在 Windows 上，打开着的文件会让它的目录留下来）。
- **在本地和后端一起运行**。两者必须共用内部令牌：要么用后端写进它的链接目录的令牌文件（默认是它的数据目录里的 `link`），要么在两个 shell 里设置同一个 `XEXAM_INTERNAL_TOKEN`。

  ```bash
  # 1. 后端（写入 ../data/link/internal.token）
  cd backend && cargo run -- serve --data ../data --no-browser
  # 2. ai-service：它的数据在 ai-service/data-ai（Git 忽略它），令牌来自后端的链接目录
  cd ai-service && XEXAM_LINK_DIR=../data/link cargo run
  # 3. 可选，一个虚构的服务商：在 AI 设置里用地址 http://127.0.0.1:9000/v1、类似 sk-demo-1234 的密钥和模型 demo-chat-small
  python -I tests/integration/ai_stub.py --port 9000
  # 4. 前端：/api/ai/ 先由 vite 插件问过后端（和 Nginx 一样），再转到 ai-service
  cd frontend && npm run dev
  ```

  ai-service 监听 `127.0.0.1:8766`（`XEXAM_AI_HOST`、`XEXAM_AI_PORT`），在 `http://127.0.0.1:8765`（`XEXAM_AI_BACKEND_URL`，只能是普通的 http）调用后端。`xexam-ai-service --help` 列出所有变量。它一拿到令牌就会向后端报告（心跳，每 25 秒一次；[architecture.md](architecture.md) 第 5 节）；后端还没有看到报告时，用户管理里没有 AI+ 功能开关，AI 设置会说 ai-service 在等它的第一次报告，所以刚启动的几秒钟里两者都属于正常现象。没有令牌时，它对每个 AI 路由回答 503 `ai.backendDown`，并记录 `AI link: waiting for the internal token at …`，直到文件出现。`ai_stub.py`（只用标准库）还提供一个 Anthropic 路由（`/anthropic`）、网关路由（`/gw/openai`、`/gw/anthropic`，用类似 `gw-demo-…` 的网关密钥）和用于错误路径的特殊模型、思考状态（`demo-reasoning`，它会先发一段虚构的推理）以及非对话的标记（`demo-embedding`，它会列出来但拒绝对话）；它会用系统提示词里「Always reply in …」那一行指定的语言回答。它的文档字符串列出了它们。
- **学习功能**（v0.9）：测试和 `tests/integration/ai_stub.py` 的虚构服务商按系统提示词回答：`asked for a HINT` 给出一个干净的自编提示（用户回合里带 `[leak]` 时，以一句点明答案的话结尾，供守卫测试用），`asked for an EXPLANATION` 给出 `**Answer: <字母>**` 和文字，`answered wrong` 给出四个标题和 `Items: <n>`。要改变守卫扣下什么，请改 `guard/lists.rs` 里的词表，并把那句话加到 `tests/study_guard/` 里；提示的提示词不接收 `Solution`，必须保持这样（文档测试固定了这一点）。后端学习路由的新数据字段，需要在 `backend/src/api/internal_study.rs`、`src/study/wire.rs`（它拒绝未知的键）和两边的契约测试里做同样的修改。
- **规则**：服务的每一段中文文字都在 `src/codes.rs` 里（前端按错误码翻译，所以新的错误码要在 `frontend/src/i18n/catalog/ai.ts` 里加一个 `ai.err.*` 键）。日志行用英语，用用户名的 `{:?}` 指代用户，从不包含密钥、内部令牌、身份声明、请求头的值、API 地址、提示词、回答、对话标题或记忆笔记；`logs_no_secrets.rs` 检查这一点。保存密钥或令牌的类型没有派生的 `Debug`。请求体手动验证（未知的键会被拒绝）；JSON 的键请保持文档里写的顺序（`serde_json` 带 `preserve_order`）。
- 连接的**后端一侧** 是 `backend/src/api/internal.rs`（令牌、身份声明、`/internal/ai/*`，包括心跳路由和最后一次报告的内存记录）、`backend/src/api/internal_study.rs`（v0.9：`question`、`record` 和 `record-info`）和 `api::auth::introspect`（认证步骤）；它的测试在 `backend/tests/v08_ai_link.rs` 和 `v09_ai_study.rs` 里。

## 4. 前端

```bash
cd frontend
npm ci
npm run typecheck      # 先对程序跑 tsc，再对测试文件跑（tsconfig.test.json）
npm test               # 391 个单元测试：API 客户端（Nginx 的 429 和重试、时间限制、AI 的流和错误码）、
                       # 判分和随机抽题、练习的核对、保存的成绩、语言规则、记录筛选、
                       # 锁定说明和等待时间、导入提醒、系统日志缓冲、题库状态、题库权限、暂存的考试、
                       # 设置缓存、事件流解析、AI 对话的状态、回答的 Markdown、记忆的请求、
                       # AI 设置的规则（包括连接原因和模型种类）、AI 状态、徽章状态、用户管理的 AI+ 功能规则、路由（`#/ai/<id>`、`#/ai/new`）、界面文字目录（所有者的措辞决定里的确切用词）、服务器提示的翻译、更新日志解析，以及 v0.9 的 AI+ 学习功能（谁看到哪个控件、一个回答的状态、带假定时器和假传输层的引擎、弹出框的规则、请求的构造、焦点目标、会话 key）
npm run build          # 类型检查 + vite build → dist/
npm run dev            # http://127.0.0.1:5173，/api 代理到 http://127.0.0.1:8765，/api/ai/ 代理到 http://127.0.0.1:8766
```

- 开发时请通过 `localhost` 或 `127.0.0.1` 打开页面。Vite 代理的 `changeOrigin` 必须是 `false`：后端会比较 `Host` 和 `Origin`，改写 `Host` 会让写请求得到 403。
- 后端不在默认地址时：`XEXAM_BACKEND=http://127.0.0.1:9000 npm run dev`；ai-service：`XEXAM_AI_BACKEND=http://127.0.0.1:9001`。
- **开发时的 AI 请求**（v0.8）：`vite.config.ts` 里的插件 `xexamAiAuth()` 在 `npm run dev` 和 `npm run preview` 里做 Nginx 在正式环境里做的事。对每个路径以 `/api/ai/` 开头的请求，它先问后端（`GET /internal/ai/auth`，带 Cookie 和 `X-Original-Host`、`X-Original-Origin`、`X-Original-Method`），然后去掉 `Cookie`、`Authorization` 和 `Cf-Access-Jwt-Assertion`，设置后端返回的身份声明（浏览器发来的这个请求头总会被替换）。后端的 401 / 403 原样转发；其他情况（后端停了）变成 503 `ai.backendDown`。不带斜杠的 `/api/ai` 和 Nginx 一样转到后端。
- 关于页面显示 `docs/en/changelog.md`、`docs/ja/changelog.md` 和 `docs/zh-CN/changelog.md` 这三份更新日志。`vite.config.ts` 里的 `xexam-changelog` 插件在构建前端时（`npm run dev` 时是第一次打开关于页面时）从仓库根目录（`frontend/` 的上一级文件夹）读取它们，所以它们必须在那里。其中某一份没有更新日志条目（`### vX.Y — 日期`）时，构建会报错停下。
- 只跑一个测试文件：`node --experimental-strip-types --test src/i18n/server-messages.test.ts`。
- import 路径要写 `.ts` 扩展名（Node 自带的测试运行器要求这样）。
- TypeScript 固定为 `~5.9.3`。

### 4.1 界面文字

用户看到的每一段文字都来自文字目录（catalog）。页面里不直接写任何文字。

- **在哪里**：`src/i18n/catalog/<区域>.ts`，每个区域一个文件：`common`、`home`、`banks`、`records`、`settings`、`quiz`、`result`、`about`、`auth`、`users`、`logs`、`ai`、`aiplus`（v0.9，学习功能）。每个文件都是 `export default defineCatalog({ en: {…}, zh: {…}, ja: {…} })`。
- **键**以区域开头：`banks.delete.title`、`records.deleteAll.ok`。`common.ts` 放共用的前缀（`app.`、`nav.`、`common.`、`err.`、`kind.`、`mode.`、`act.`、`role.`、`forbidden.` 等）。
- **永远是三种语言。** `zh` 或 `ja` 块缺了一个英语的键，TypeScript 就会拒绝。以 `_one` 结尾的键是英语的单数形式；数量为 1 时 `tn()` 用它，中文和日语里可以不写。
- **占位符**写成 `{name}`：`t('records.deleteOne.body', { at, pct })`。计数用 `tn(key, n)`，它会填进 `{n}`。
- **检查**：`src/i18n/catalog.test.ts` 确保三种语言的键和占位符完全相同，没有空的文字，每个键都带着自己区域的前缀、只在一个文字目录里出现，每个文字目录文件都在 `src/i18n/index.ts` 里注册了。
- **新的文字目录文件**：在 `src/i18n/index.ts` 里 import 它，加进 `ALL` 和 `MsgKey` 类型。
- 先写英语，再写中文和日语。用界面里已经在用的说法（例如「题库」「下架」「错题本」「可管理题库」）。
- 题目 / 答案的语言切换按钮是例外：不管界面是什么语言，它的三个按钮总是写着 **English**、**中文** 和 **日本語**（`src/components/lang-switch.ts`）；只有它的无障碍名称和悬停提示来自文字目录。

### 4.2 界面里的后端提示

后端的错误提示是中文，而且是固定的（它们是经过测试的 API 的一部分）。前端翻译它们：

- `src/i18n/server-messages.ts`：`EXACT` 对应整句提示，`PATTERNS` 对应后端的格式字符串（其中的 `{}` 部分变成捕获的值）。中文界面原样显示提示；不认识的文字也原样显示。少数底层的协议错误是英语单词（`not found`、`bad json` 等）；它们在每种语言里都有一句话，中文也不例外。
- `src/i18n/errors.ts`：`errorText(e)` 把任何错误变成当前语言的文字。页面显示的每个错误都用它。页面自己的错误是 `LocalError(key, params)`，所以它会随界面切换语言。
- **漂移测试**：`src/i18n/server-messages.test.ts` 用一个小的词法分析器读取 `backend/src` 里的 Rust 源码。每一个带中文或日文字符的字符串字面量（注释、测试代码、断言、日志和控制台输出除外）都必须有英语和日语的翻译，保留它的占位符的值，而且译文里不能有中文。后端里已经不存在的文字还留着翻译，测试也会失败。
- **新增或修改后端提示时**：在同一次修改里把翻译加进 `server-messages.ts`。如果这个字符串不会显示给用户（例如解析器要找的一个词），就把它加进测试里的 `NOT_MESSAGES`，并写明原因。
- **日志行保持英语**，绝不能包含密码、会话令牌、Cookie 的值或题目文字（`backend/tests/v07_wal_and_logs.rs` 在一整个会话之后检查日志）。任何人不用登录就能引起的日志行要经过 `BurstLog`（`backend/src/logs.rs`），而不是直接交给 `tracing`。
- **AI 的错误** 不按文字翻译：AI 服务给每个错误一个固定的 `code`，`errorText()` 使用 `src/i18n/catalog/ai.ts` 里的键 `ai.err.<去掉 "ai." 的错误码>`，并用错误里的 `limit` 填入 `{n}`。AI 服务新增的错误码需要在三种语言里都加上这个键。
- **AI+ 学习控件**（v0.9）由一个组件 `src/components/ai-plus-controls.ts` 构建；页面向它要一个控件，自己从不构建 AI 元素，所以规则（没有 AI+ 功能时什么都没有，AI 用不了时变灰并带标签和原因，考试页上没有控件）都集中在一处和 `src/domain/ai-plus.ts` 里。弹出框（`ai-pop.ts`）和总结窗口（`ai-summary.ts`）由 `main.ts` 安装一次，同时安装每次页面重绘之后的 `reanchorAiPop()`，以及让它们忘掉自己记忆的退出登录钩子。判断都是纯函数，有单元测试；一个回答的逻辑是 `ai-study.ts` 里的 reducer，`ai-run.ts` 是注入了传输层和定时器的引擎。回答是 Markdown，用和 AI 对话相同的安全渲染器（`ai-render.ts`，只用文本节点）；学习功能的请求错误文字是 `src/i18n/errors.ts` 里的 `studyErrorText`。

### 4.3 新增页面或路由

1. **路由**：在 `src/router.ts` 里，给 `Route` 加一个变体，给 `parseHash` 加一个分支；如果它的 hash 不是 `#/<名字>`，也给 `routeHash` 加一个。在 `sectionOf` 里说明它高亮哪个菜单项（`null` = 不高亮）。只有管理员能打开的页面，要么放进 `adminOnly`（其他人被送回主页），要么由 `main.ts` 显示 **没有权限** 页面。
2. **页面**：新建 `src/pages/<名字>.ts`，写一个返回 `HTMLElement` 的 `render<Name>()` 函数。用 `src/utils/dom.ts` 里的 `h()` 构建 DOM。样式放进 `src/styles/<名字>.css`，在页面开头 import 它，并使用 `main.css` 里的颜色变量，这样深色主题才能用。
3. **接上**：在 `src/main.ts` 的 `page()` 里加一个分支，带浏览器标签页的标题。
4. **菜单项**（只用于主要的板块）：加进 `src/router.ts` 的 `NavSection`，以及 `src/components/shell.ts` 的 `NAV`（只有部分用户能看到时带上 `need`）、`HREF`，图标加进 `ICONS`。`NAV` 里的项列在菜单分隔线上面；分隔线下面的关于那一项是单独构建的。
5. **文字**：新的文字目录 `src/i18n/catalog/<名字>.ts`（第 4.1 节），三种语言都要写。
6. **状态**：页面之间从不互相 import。共享状态放在 `src/state/store.ts` 里；修改都通过 `src/state/actions.ts` 里的函数（`navigate(route)`、`render()` 等）。只有这个页面需要的状态放在页面模块的变量里。用户离开或进入一个页面时，`onRouteChange()` 会运行代码。
7. **API**：新的接口要在 `backend/src/api/auth.rs` 的路由表 `ROUTES` 里加一行（谁能用它），在 `src/api/` 里加一个带类型的封装，在 `src/types/api.ts` 里加它的类型，并写进 [api.md](api.md) 和 [openapi.yaml](../en/openapi.yaml)。AI 路由则加进 `ai-service/src/api/mod.rs` 的 `ROUTES`，带它的访问级别。

## 5. 整栈

```bash
# 构建并启动三个容器（同样的 xexam_* 卷！会用到你自己的数据；想做实验请换一个项目名：-p some-other-name）
docker compose -f docker/docker-compose.build.yml up -d --build

# 检查脚本都用临时的项目名和临时卷，从不碰 xexam_* 卷
docker build -f nginx/Dockerfile -t xexam-frontend:test .
python -I tests/integration/nginx_check.py xexam-frontend:test 18080   # 141 项检查，用一个假的后端和一个假的 ai-service
                                                                      # （限流、认证步骤、去掉请求头、流式输出）
python -I tests/integration/stack_check.py 18090                      # 314 项检查：整栈（会构建三个镜像），带虚构的
                                                                      # AI 服务商：登录、导入、交卷、AI 设置、
                                                                      # 流式回答、AI+ 功能及其暂停、
                                                                      # 心跳和 docker stop、回复语言、自由模式（按用户）、
                                                                      # AI+ 学习功能（提示、解释、总结、对话
                                                                      # 和它们的标题、已保存的回答、应用忙时的流程、什么都不记录）、
                                                                      # 持久化、WAL、系统日志、
                                                                      # reset-password、docker stop、每个卷里有什么
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend   # 78 项检查（Linux 上 80 项）：progress.json 迁移、WAL 和 DELETE
                                                                      # 选项、schema 3 → 6、4 → 6 和 5 → 6、内部令牌（先 cargo build）
```

每个脚本都会打印它的检查项，最后输出 `All N checks passed`。`nginx_check.py` 和 `stack_check.py` 的最后一个参数是它们用的宿主机端口；请选一个你机器上空闲的端口。`stack_check.py` 读取一个可选的 `XEXAM_STACK_COMPOSE`：另一个结构相同的 compose 文件（例如 `docker/docker-compose.build.yml` 的一份副本，换了别的镜像标签和容器名），这样它就能在一个用着通常名字的安装旁边运行；它通过 compose 服务找到容器。`nginx_check.py` 大约要两分半钟，因为有一项检查要为 AI 设置里一次很慢的 **测试** 等 95 秒。`stack_check.py` 会给它的学习提问限速（每个会话 Cookie 大约 3.2 秒一次），因为 Nginx 在突发 10 个之后，每 3 秒才让一个会话开始一个 AI 回答（区域 `aisend`，每分钟 20 个）；这大约多花一分半钟。这些脚本用它们自己写出来的虚构题库；没有一个会碰你的数据。

compose 文件和 `.env` 在 `docker/` 里；`docker/docker-compose.build.yml` 以仓库根目录（`..`）为构建上下文。前端镜像从仓库根目录构建。`nginx/Dockerfile` 把三份更新日志复制进它的 Node 构建阶段（`COPY docs/en/changelog.md /docs/en/`，`ja` 和 `zh-CN` 也一样），因为关于页面是从它们构建出来的；出于同样的原因，`.dockerignore` 把 `docs/` 和 README 排除在构建上下文之外，只把这三个文件放回来。最终镜像里只有构建出的静态文件。

## 6. 和旧版对照（回归）

旧的 Python 实现已经从仓库里删掉了，但它的行为固定在 `tests/baseline/` 里（全部是虚构的合成数据）：

| 快照 | 内容 | 谁在用 |
|---|---|---|
| `golden/NNN_*.json`（172 个文件） | 旧版在每个接口 × 每种状态下的真实响应（状态码、稳定的响应头、规范化后的正文） | `backend/tests/api_golden.rs` 按顺序对新的路由重放它们，并逐项比较 |
| `fixtures/*.xlsx` + `parser_golden/*.json`（49 个） | 合成的工作簿（三列和四列格式，也就是旧版的格式），以及旧版解析器对每一份的输出 | `backend/tests/parser_golden.rs`（解析器读出的结果仍必须逐字节相同；导入就是靠这一点认出并拒绝它们） |
| `golden/id_vectors.json`（85 条断言） | 关于题目 id 的断言；每一条都带着被哈希的字节 | `backend/tests/id_vectors.rs` |
| `fixtures/sample.db` + `golden/store_sample_outputs.json` | 旧实现写的示例数据库，以及它每个接口的输出 | `backend/tests/db_golden.rs`（也检查它迁移到当前 schema） |
| `golden/allowed_differences_rust.json`（23 条）、`allowed_probe_differences_rust.json`（51 条） | Rust 实现取代旧版时有意和旧版不同的地方，每一处都有一句理由（用中文写的）；只是一份记录，没有测试读取它们 | [api.md](api.md) 第 7 节 |

- `api_golden.rs` 把每一份抓取归入五种结果之一，用 `cargo test --test api_golden -- --nocapture` 打印出这张表。现在是：84 份一致，58 份在用抓取内容喂给引擎之后一致，16 份有意的差异（7 个不支持的方法现在得到 405，以及 9 份 `GET /api/info` 的抓取，从 v0.7.5 起比较时不看 `bankCount`），14 份跳过（静态文件由 Nginx 提供，还有一份抓取没有响应），0 份不同。只要有一份不同，测试就失败。
- 要**改变**某份抓取覆盖的行为：先改代码，再在 `api_golden.rs` 里写明这处有意的差异和理由（就像 `bankCount` 那样）；不要直接改黄金响应。
- 旧的抓取 / 对照 / 探测脚本依赖旧的 Python 代码，已经随它一起删掉了；需要的话，从 Git 标签 `python-v0.2` 和提交历史里找回来。

## 7. 数据安全规则

- **不要**读取、打印、提交或修改 `data/`、`data-ai/`、`progress.json` 或你自己的题库 Excel 文件。
- 永远不要把真正的 AI 密钥放进代码、测试、文档或截图：测试用明显是假的密钥（`sk-demo-…`、`gw-demo-…`），示例用 `example.com` 的地址，例如网关用 `https://ai-gateway.example.com/<route>`。
- 永远在副本或临时目录里做实验；不要对真正的 `xexam.db` 运行「试试看会怎样」的命令。记住，用较新的版本打开较旧的数据库会迁移它（并把它切换到 WAL）。
- 不要运行 `docker compose down -v`。
- 代码、测试和文档里的示例只用虚构的内容，永远不用真实的考试题目（`tools/screenshots/demo_banks.py` 里的演示题库「General knowledge」和「Kitchen basics」，或者 `frontend/src/pages/banks.ts` 里虚构的六列格式示例）。

## 8. CI

只有一条流水线：[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。推送到 `main` 和 `dev`、向 `main` 提拉取请求，以及手动启动时，它都会运行。

| 任务 | 做什么 |
|---|---|
| `check` | `VERSION` 里**正好有一行** `CODE_VERSION=` 和**一行** `RELEASE_VERSION=`，两者都形如 `0.7` 或 `0.6.5`，而且 `RELEASE_VERSION` 不比 `CODE_VERSION` 新；三份更新日志（`docs/en/changelog.md`、`docs/ja/changelog.md`、`docs/zh-CN/changelog.md`）各自最新的条目 = `CODE_VERSION`，三份的 `### vX.Y — 日期` 标题相同、顺序相同；README 里不再有更新日志条目，每份 README 都链接到自己语言的更新日志；每份 README 顶部的发布徽章 = `RELEASE_VERSION`；`backend/Cargo.toml`、`ai-service/Cargo.toml` 和 `frontend/package.json` 写的是 `CODE_VERSION`；文件布局（根目录只有 `README.md`，另外两份 README 在 `README/` 里，compose 文件和 `.env.example` 在 `docker/` 里）、`docker/docker-compose.yml` 从 Docker Hub 拉取三个镜像，以及文档检查（第 9 节）。它输出的 `version`（`CODE_VERSION`）供后面的任务使用 |
| `frontend` | 类型检查、391 个单元测试、构建 |
| `backend` | fmt、clippy、382 个测试，以及 `progress.json` 迁移检查 |
| `ai-service` | fmt、clippy、ai-service 的 383 个测试，以及 TLS 检查：`cargo tree` 里不能有 `aws-lc`、`openssl-sys` 或 `native-tls` |
| `stack` | 在 `frontend`、`backend` 和 `ai-service` 之后：构建 `xexam-frontend` 镜像，跑 141 项 Nginx 检查；启动完整的 compose 栈（三个镜像、虚构的 AI 服务商），跑 217 项端到端检查（最多 45 分钟） |
| `arm64` | 为 arm64 构建三个镜像，在 QEMU 下启动它们，检查 `/api/info` 报告的是 `aarch64` 和 `CODE_VERSION`，并检查单独运行的 ai-service 通过它的健康检查（`xexam-ai-service healthcheck`）、在 `/healthz` 里报告同样的内容（最多 60 分钟） |
| `publish` | 只有 `main`：把三个镜像（amd64 + arm64）以 `latest`、`v<CODE_VERSION>` 和 `sha-<提交>` 标签推送到 Docker Hub（`docker.io/yuminspace/`，公开，`docker/docker-compose.yml` 拉取的就是它）和 ghcr.io（私有镜像副本），并在两边都核对清单和标签。需要仓库机密 `DOCKERHUB_USERNAME` 和 `DOCKERHUB_TOKEN`（变量 `DOCKERHUB_NAMESPACE` 可选）；没有它们时，会在推送任何东西之前停下 |
| `release` | 只有 `main`：建立（或更新）GitHub Release `v<CODE_VERSION>`，说明取自 `docs/en/changelog.md`（`scripts/release-notes.py`）。比 `main` 上已经记录的发布更旧的版本的运行，不会把它的 Release 标成最新版。它的最后一步记录这次发布（第 8.1 节） |

推送到 `dev` 和向 `main` 提拉取请求只跑检查任务（`check` 到 `arm64`），**从不发布**；只有推送到 `main` 才会发布。手动启动的运行，和推送到它所在的分支效果一样。

### 8.1 发布之后：机器人的提交

只有镜像已经发布、GitHub Release 也已经存在时，`release` 任务的最后一步才会改动 `RELEASE_VERSION`：

1. 它在最新的 `origin/main` 上运行 `scripts/set-release-version.py <版本>`，设置 `VERSION` 里的 `RELEASE_VERSION`，以及三份 README 顶部的发布徽章（它的替代文字、URL 和宽度）。
2. 它把这些作为 **`Record release v<版本>: RELEASE_VERSION and the README badge [skip ci]`** 提交到 `main`，用的是 GitHub API（GraphQL 的 `createCommitOnBranch`），而不是 `git push`，所以 GitHub 会为这个提交签名，显示为 **Verified**。请求里写明了出发时的 `main` 提交，如果这期间 `main` 前进了，API 会拒绝。用工作流自己的令牌生成的提交不会启动新的运行，提交信息里也写着 `[skip ci]`。没有任何变化时（重新运行），它什么都不提交。
3. 如果 `dev` 没有自己的提交，就把 `dev` 快进到同一个提交。如果 `dev` 已经往前走了（或者在发布期间往前走了），它就不动 `dev`，并在运行里显示一条**提示**：`merge main into dev (git pull origin main on dev) before the next merge to main`（意思是：下一次合并到 main 之前，先把 main 合并进 dev）。不管哪种情况，发布都已经完成。

`set-release-version.py` 从不把 `RELEASE_VERSION` 往回调：重新运行一次较早的运行的发布任务，什么都不改，以 0 退出。它拒绝比 `CODE_VERSION` 更新的版本。

所以每次发布之后，`origin/main`（通常还有 `origin/dev`）比你的克隆多一个提交：

- **下一次推送之前**，先更新这两个分支：`git checkout main && git pull`，然后 `git checkout dev && git pull`。
- **运行里显示了那条提示时**，在下一次合并到 main 之前把 main 合并进 dev：在 `dev` 上 `git pull origin main`，然后推送 `dev`。否则下一次合并到 `main` 时会漏掉机器人的提交，或者在 `VERSION` 和徽章上发生冲突。

## 9. 文档

- 每种语言一个文件夹：`docs/en`（英语，原文）、`docs/ja`（日语）和 `docs/zh-CN`（简体中文）。每个文件夹里都有同样的九份文档：`api`、`architecture`、`changelog`（从 v0.9.1 起；以前在 README 里）、`database`、`deployment`、`development`、`migration`、`security`，以及从 v0.7.5 起的 `usage`（怎样使用程序，从 README 移过来的），每个文件夹都有自己的 `screenshots/`。
- `docs/zh-CN/history/` 里是切换到 Rust 时的旧文档（审计、计划、风险评估）。它们只有中文，不再更新。
- `docs/en/openapi.yaml` 由所有语言共用；日语和中文的文档链接到 `../en/openapi.yaml`。
- README 很短：Xexam 能做什么、快速开始、配置、升级、文档链接，以及指向更新日志（同一语言的 `changelog.md`）的链接。怎样使用程序、Excel 格式和导入规则在 `usage.md` 里；部署细节、备份、安全、架构和开发各有自己的文档。一个细节只写在一个地方，其他地方链接过去。
- 每份文档开头都有一条语言栏，链接到另外两个文件夹里的同一份文档。链接相对于文件自己所在的文件夹：仓库根目录是 `../../`。
- 先改英语的文档，再在同一次修改里改日语和中文的。界面上的标签用粗体，写法和那种语言的文字目录完全一样；题目语言的切换按钮总是写成 **English** / **中文** / **日本語**。
- `api` … `usage` 这九份文档中有一份在某个语言文件夹里缺失，`docs/` 里有 `en`、`ja` 和 `zh-CN` 以外的东西，或者 README 或文档里的相对链接或图片路径指向不存在的文件时，CI 的 `check` 任务会失败。指向 GitHub 页面的链接（`../../releases` 之类）也会检查：它们必须正好回到仓库页面，所以在 `README/` 里要以 `../../../` 开头。

### 9.1 截图

README 和文档里的图片（`docs/<lang>/screenshots`，每种语言 19 张电脑截图和 5 张 iPhone 截图，一共 24 张）由在仓库根目录运行的 `python tools/screenshots/retake.py` 生成。它用 `backend/Dockerfile` 和 `ai-service/Dockerfile` 以唯一的标签（`xexam-shots-backend:<十六进制>`、`xexam-shots-ai-service:<十六进制>`）构建后端和 ai-service 的镜像，在一个新的 Docker 网络上用新卷（各自的 `/data`，以及一个共用的链接卷，对 ai-service 是只读的）和 `TZ=UTC` 运行它们，旁边运行虚构的 AI 服务商 `tests/integration/ai_stub.py`（网络名 `ai-stub`）。它用 Vite 构建前端，在一个空闲端口上用 `vite preview` 提供它（认证插件和 Nginx 一样，用后端检查每个 AI 请求），然后通过 API 往这个实例里填入虚构的演示题库、用户和记录，并用虚构的服务商和虚构的密钥设置好 AI，这都是通过 API 完成的；之后才给 sam 和 alex 开启 AI+ 功能（AI+ 功能不可用时后端会拒绝这一步），并暂停 alex 的 AI+ 功能，所以图片里有亮色、暗色和没有徽章的三种，也有一行 **使用中** 和一行 **已暂停**。重启之后，它会等到 `GET /api/users` 回答 `aiAvailable: true`。它会重启后端，让系统日志里只有短短一段登录和用户修改，然后为每种语言在无头 Chrome 里操作真正的界面：AI 对话的场景先打开管理员的自由模式（这样会显示绿色的 **自由模式** 标签），开始一个对话，通过页面发送第二个问题（用这次运行的语言，所以回答也是这种语言），之后删除这个对话并关闭自由模式，v0.9 的学习场景则打开一道练习题的 AI 提示窗口、一张考试成绩卡片的 AI 解析窗口和一条学习记录的 AI 总结窗口（`desktop-17-ai-hint`、`desktop-18-ai-result`、`desktop-19-ai-summary`、`iphone-5-ai-hint`），里面是虚构服务商的文字；同一次运行还会检查布局（检查失败就会停下）：练习的操作栏在 390 px 宽时最多两行，学习记录在 800 px 宽时（英语和日语）用卡片布局，没有横向滚动；AI 设置的场景按下 **测试**，并用两张图片显示这个页面（设置部分和带 AI+ 功能一列的 AI+ 用户；然后是用量、对话保留和安全上限），所以各种语言的用量数字会略有不同。图片在不会切断文字或卡片的地方裁开，并去掉 PNG 元数据。容器、卷、网络、镜像标签、临时文件夹和进程在最后都会删掉，出错时也一样。

选项：`--lang en|ja|zh-CN|all`、`--out DIR`（写到 `DIR/<lang>/`，这样可以在动 `docs/` 之前先比较）、`--only desktop-13-logs.png,...`、`--chrome PATH`（或 `XEXAM_CHROME` / `CHROME_PATH`），以及没有 Docker 时用的 `--backend cargo`（这样运行会跳过系统日志的图片）。它需要带 `openpyxl` 的 Python 3.8+、Node.js、Docker 以及 Chrome 或 Chromium；见 `tools/screenshots/README.md`。用 `--backend cargo` 时，它用 cargo 构建 `backend/` 和 `ai-service/`，并把虚构的服务商作为本地的 Python 进程运行。有两个页面在页面本身里被缩短，好让卡片在图片里结束：题库管理显示的格式说明打开后只有前两部分，关于页面只显示最新版本的前几点。可以同时运行两三次（被别的运行先占用的端口会换一个新的重试，`npm ci` 在一个锁文件后面运行）。请用 `python tools/screenshots/retake.py` 启动它，不要用 `python -I`：`-I` 会隐藏通常装着 `openpyxl` 的用户 site-packages。改了界面或版本之后重新截图，先看一遍，然后才写进 `docs/`。

### 9.2 图标

程序的图标（页头的标志、浏览器标签页、手机主屏幕、README）放在 `frontend/public/`：`icon.svg`、`icon-32.png`、`icon-192.png`、`icon-512.png` 和 `apple-touch-icon.png`（180 × 180，没有透明的圆角，因为 iOS 会自己把它们变圆）。`python3 -I tools/icons/make_icons.py <源.svg>` 从一个源 SVG 写出全部五个文件：它用无头 Chrome（`tools/screenshots/browser.py` 的 DevTools 客户端）画出 PNG，再用纯 Python 重新编码，只保留 IHDR、IDAT 和 IEND 块，所以它们不带元数据，输出只取决于像素。它打印每个文件的 SHA-256，重复运行结果不变。

## 10. 版本、更新日志和许可证

- 仓库根目录的 **`VERSION`** 里有两行 `KEY=VALUE`（以 `#` 开头的行是注释），每行正好出现一次：
  - `CODE_VERSION`（现在是 `0.9.1`）：这个分支上代码的版本。手动调高它，同时在三份更新日志（`docs/<lang>/changelog.md`）里写条目，并改 `backend/Cargo.toml`、`ai-service/Cargo.toml` 和 `frontend/package.json`（像 `0.9` 这样两段的版本号在那里写成 `0.9.0`；还有它们的锁文件）以及 `docs/en/openapi.yaml` 里的 `info.version`。镜像用它来构建和打标签，`/api/info` 报告的也是它（后端从镜像里的 `/VERSION` 读取它）。
  - `RELEASE_VERSION`（v0.9.1 的发布任务运行之前是 `0.9`）：最后一个真正发布出去的版本。**永远不要手动修改它**：由发布任务写入，同时更新 README 顶部的发布徽章（第 8.1 节）。单纯的推送或合并从不改变它。
- 每次更新都要在三种语言的更新日志（`docs/en/changelog.md`、`docs/ja/changelog.md`、`docs/zh-CN/changelog.md`）最上面加一条，版本号等于 `CODE_VERSION`；三份的条目始终保持一致。CI 会检查这一点。README 里不写条目，只放一个指向更新日志的链接。GitHub Release 的说明取自英语的那一份。
- 程序的关于页面显示这些更新日志，它们是构建进前端的（第 4 节）。条目请只用简单的 Markdown：段落、`- ` 列表、缩进的 `  - ` 子列表、`**粗体**` 和 `` `代码` ``；指向仓库里文件的链接在那里显示成普通文字，链接要写成相对于更新日志所在文件夹（`docs/<lang>/`）的路径。修改了的更新日志，要重新构建前端之后才会出现在程序里。
- 在 `dev` 上开发；`main` 接收合并，推送到 `main` 会发布镜像和 Release。每次发布之后都要拉取 `main` 和 `dev`（第 8.1 节）。
- 本项目采用 MIT 许可证（仓库根目录的 `LICENSE`）。
