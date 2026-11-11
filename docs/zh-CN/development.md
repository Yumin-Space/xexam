[English](../en/development.md) | [日本語](../ja/development.md) | **简体中文**

# 开发

> 日常改代码、跑测试用的命令，以及东西都放在哪里。架构见 [architecture.md](architecture.md)；最初的测试思路见 [test-strategy.md](history/test-strategy.md)。

## 1. 工具

| 工具 | 版本 | 说明 |
|---|---|---|
| Rust | 1.99.1（由 `backend/rust-toolchain.toml` 固定） | `rustup` 会自动安装。`rusqlite` 的 `bundled` 特性需要 C 编译器（Linux：`build-essential`；Windows：MinGW gcc 或 MSVC） |
| Node | 24 | 前端；单元测试用 Node 自带的测试运行器加 `--experimental-strip-types` |
| Python | 3.8+ | 只用于 `tests/integration/` 里的集成脚本和 `scripts/` 里的两个脚本。**只用标准库** |
| Docker | Compose v2 | 整栈检查、镜像 |

## 2. 后端

```bash
cd backend
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 302 个测试
cargo run -- serve --data ../data --no-browser     # 监听 127.0.0.1:8765
```

- 测试对着 `tests/baseline/` 里已提交的**旧版快照**运行；不需要 Python。
- 所有测试都用临时目录里的新数据库，或 `tests/baseline/fixtures/sample.db` 的副本，**永远不碰 `data/`**。除了 `src/` 里的单元测试，测试文件有：`api_basic.rs`、`api_import.rs`、`api_misc.rs` 和 `api_golden.rs`（路由、守卫、旧版的响应），`db_golden.rs`（数据库对照旧版快照），`parser_golden.rs`、`parser_hardening.rs`、`parser_four_columns.rs`、`parser_six_columns.rs` 和 `id_vectors.rs`（解析器），以及每个版本一个文件：`v05_banks_records_settings.rs`（schema v2、题库、记录、设置）、`v06_users_and_sign_in.rs`（schema v3、账户、会话、登录限制）、`v065_bank_access_and_practice.rs`（schema v4、题库权限、可管理题库的用户、所有权、练习）、`v07_wal_and_logs.rs`（WAL 模式、检查点、服务器运行时的 `reset-password`、系统日志）和 `v07_log_floods.rs`（大量被拒绝的请求和失败的登录被汇总）。
- 测试通过辅助代码登录；没有任何开关能关掉登录，测试也不例外。
- 常用的环境变量：`XEXAM_DATA`、`XEXAM_PORT`、`XEXAM_ALLOWED_HOSTS`、`XEXAM_LOG`（日志级别，例如 `debug`）、`XEXAM_SQLITE_JOURNAL`（`delete` 保留回滚日志，不用 WAL）、`XEXAM_COOKIE_SECURE`。完整的列表见 `xexam-backend --help` 和 [deployment.md](deployment.md)。
- 在空的数据目录上第一次启动时，会创建管理员 `admin` / `admin`；第一次登录时页面会要求设置新密码。忘了的话：`cargo run -- reset-password admin --data ../data`（服务器运行时也能用）。
- 命令行导入（导入到旧题库）：`cargo run -- import ../some.xlsx --data ../data`。
- 让后端打开一个旧 schema 的数据库，会迁移它，并在旁边写一个 `backups/`。请用副本。
- 数据库以 WAL 模式运行：后端运行时，`xexam.db` 旁边有 `xexam.db-wal` 和 `xexam.db-shm`。要么三个一起复制，要么先停止后端（之后所有内容都在 `xexam.db` 里，旁边即使留着 `-wal` 文件，也是空的）。后端运行时 `sqlite3` 也能读这个数据库。

## 3. 前端

```bash
cd frontend
npm ci
npm run typecheck      # 先对程序跑 tsc，再对测试文件跑（tsconfig.test.json）
npm test               # 119 个单元测试：判分和随机抽题、练习的核对、保存的成绩、语言规则、记录筛选、
                       # 锁定说明和等待时间、导入提醒、系统日志缓冲、题库权限、暂存的考试、设置缓存、
                       # 界面文字目录、服务器提示的翻译、更新日志解析
npm run build          # 类型检查 + vite build → dist/
npm run dev            # http://127.0.0.1:5173，/api 代理到 http://127.0.0.1:8765
```

- 开发时请通过 `localhost` 或 `127.0.0.1` 打开页面。Vite 代理的 `changeOrigin` 必须是 `false`：后端会比较 `Host` 和 `Origin`，改写 `Host` 会让写请求得到 403。
- 后端不在默认地址时：`XEXAM_BACKEND=http://127.0.0.1:9000 npm run dev`。
- 关于页面显示 `README.md`、`README/README.ja.md` 和 `README/README.zh-CN.md` 的更新日志。`vite.config.ts` 里的 `xexam-changelog` 插件在构建前端时（`npm run dev` 时是第一次打开关于页面时）从 `frontend/` 的上一级文件夹（仓库根目录）读取它们，所以它们必须在那里。其中某一份没有更新日志条目（`### vX.Y — 日期`）时，构建会报错停下。
- 只跑一个测试文件：`node --experimental-strip-types --test src/i18n/server-messages.test.ts`。
- import 路径要写 `.ts` 扩展名（Node 自带的测试运行器要求这样）。
- TypeScript 固定为 `~5.9.3`。

### 3.1 界面文字

用户看到的每一段文字都来自文字目录（catalog）。页面里不直接写任何文字。

- **在哪里**：`src/i18n/catalog/<区域>.ts`，每个区域一个文件：`common`、`home`、`banks`、`records`、`settings`、`quiz`、`result`、`about`、`auth`、`users`、`logs`。每个文件都是 `export default defineCatalog({ en: {…}, zh: {…}, ja: {…} })`。
- **键**以区域开头：`banks.delete.title`、`records.deleteAll.ok`。`common.ts` 放共用的前缀（`app.`、`nav.`、`common.`、`err.`、`kind.`、`mode.`、`act.`、`role.`、`forbidden.` 等）。
- **永远是三种语言。** `zh` 或 `ja` 块缺了一个英语的键，TypeScript 就会拒绝。以 `_one` 结尾的键是英语的单数形式；数量为 1 时 `tn()` 用它，中文和日语里可以不写。
- **占位符**写成 `{name}`：`t('records.deleteOne.body', { at, pct })`。计数用 `tn(key, n)`，它会填进 `{n}`。
- **检查**：`src/i18n/catalog.test.ts` 确保三种语言的键和占位符完全相同，没有空的文字，每个键都带着自己区域的前缀、只在一个文字目录里出现，每个文字目录文件都在 `src/i18n/index.ts` 里注册了。
- **新的文字目录文件**：在 `src/i18n/index.ts` 里 import 它，加进 `ALL` 和 `MsgKey` 类型。
- 先写英语，再写中文和日语。用界面里已经在用的说法（例如「题库」「下架」「错题本」「可管理题库」）。
- 题目 / 答案的语言切换按钮是例外：不管界面是什么语言，它的三个按钮总是写着 **English**、**中文** 和 **日本語**（`src/components/lang-switch.ts`）；只有它的无障碍名称和悬停提示来自文字目录。

### 3.2 界面里的后端提示

后端的错误提示是中文，而且是固定的（它们是经过测试的 API 的一部分）。前端翻译它们：

- `src/i18n/server-messages.ts`：`EXACT` 对应整句提示，`PATTERNS` 对应后端的格式字符串（其中的 `{}` 部分变成捕获的值）。中文界面原样显示提示；不认识的文字也原样显示。少数底层的协议错误是英语单词（`not found`、`bad json` 等）；它们在每种语言里都有一句话，中文也不例外。
- `src/i18n/errors.ts`：`errorText(e)` 把任何错误变成当前语言的文字。页面显示的每个错误都用它。页面自己的错误是 `LocalError(key, params)`，所以它会随界面切换语言。
- **漂移测试**：`src/i18n/server-messages.test.ts` 用一个小的词法分析器读取 `backend/src` 里的 Rust 源码。每一个带中文或日文字符的字符串字面量（注释、测试代码、断言、日志和控制台输出除外）都必须有英语和日语的翻译，保留它的占位符的值，而且译文里不能有中文。后端里已经不存在的文字还留着翻译，测试也会失败。
- **新增或修改后端提示时**：在同一次修改里把翻译加进 `server-messages.ts`。如果这个字符串不会显示给用户（例如解析器要找的一个词），就把它加进测试里的 `NOT_MESSAGES`，并写明原因。
- **日志行保持英语**，绝不能包含密码、会话令牌、Cookie 的值或题目文字（`backend/tests/v07_wal_and_logs.rs` 在一整个会话之后检查日志）。任何人不用登录就能引起的日志行要经过 `BurstLog`（`backend/src/logs.rs`），而不是直接交给 `tracing`。

### 3.3 新增页面或路由

1. **路由**：在 `src/router.ts` 里，给 `Route` 加一个变体，给 `parseHash` 加一个分支；如果它的 hash 不是 `#/<名字>`，也给 `routeHash` 加一个。在 `sectionOf` 里说明它高亮哪个菜单项（`null` = 不高亮）。只有管理员能打开的页面，要么放进 `adminOnly`（其他人被送回主页），要么由 `main.ts` 显示 **没有权限** 页面。
2. **页面**：新建 `src/pages/<名字>.ts`，写一个返回 `HTMLElement` 的 `render<Name>()` 函数。用 `src/utils/dom.ts` 里的 `h()` 构建 DOM。样式放进 `src/styles/<名字>.css`，在页面开头 import 它，并使用 `main.css` 里的颜色变量，这样深色主题才能用。
3. **接上**：在 `src/main.ts` 的 `page()` 里加一个分支，带浏览器标签页的标题。
4. **菜单项**（只用于主要的板块）：加进 `src/router.ts` 的 `NavSection`，以及 `src/components/shell.ts` 的 `NAV`（只有部分用户能看到时带上 `need`）、`HREF`，图标加进 `ICONS`。`NAV` 里的项列在菜单分隔线上面；分隔线下面的关于那一项是单独构建的。
5. **文字**：新的文字目录 `src/i18n/catalog/<名字>.ts`（第 3.1 节），三种语言都要写。
6. **状态**：页面之间从不互相 import。共享状态放在 `src/state/store.ts` 里；修改都通过 `src/state/actions.ts` 里的函数（`navigate(route)`、`render()` 等）。只有这个页面需要的状态放在页面模块的变量里。用户离开或进入一个页面时，`onRouteChange()` 会运行代码。
7. **API**：新的接口要在 `backend/src/api/auth.rs` 的路由表 `ROUTES` 里加一行（谁能用它），在 `src/api/` 里加一个带类型的封装，在 `src/types/api.ts` 里加它的类型，并写进 [api.md](api.md) 和 [openapi.yaml](../en/openapi.yaml)。

## 4. 整栈

```bash
# 构建并启动两个容器（同一个 xexam_data 卷！会用到你自己的数据；想做实验请换一个项目名：-p some-other-name）
docker compose -f docker/docker-compose.build.yml up -d --build

# 检查脚本都用临时的项目名和临时卷，从不碰 xexam_data
docker build -f nginx/Dockerfile -t xexam-frontend:test .
python -I tests/integration/nginx_check.py xexam-frontend:test 18080   # 38 项检查，用一个假的后端
python -I tests/integration/stack_check.py 18090                      # 启动整栈（会构建两个镜像）：登录、导入、
                                                                      # 交卷、持久化、WAL、系统日志、reset-password、docker stop
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend   # progress.json 迁移、WAL 和 DELETE 选项（先 cargo build）
```

每个脚本都会打印它的检查项，最后输出 `All N checks passed`。`nginx_check.py` 和 `stack_check.py` 的最后一个参数是它们用的宿主机端口；请选一个你机器上空闲的端口。

compose 文件和 `.env` 在 `docker/` 里；`docker/docker-compose.build.yml` 以仓库根目录（`..`）为构建上下文。前端镜像从仓库根目录构建。`nginx/Dockerfile` 把三份 README 复制进它的 Node 构建阶段（`COPY README.md /` 和 `COPY README/README.ja.md README/README.zh-CN.md /README/`），因为关于页面的更新日志是从它们构建出来的；出于同样的原因，`.dockerignore` 把 `README.md` 和 `README/` 留在构建上下文里。最终镜像里只有构建出的静态文件。

## 5. 和旧版对照（回归）

旧的 Python 实现已经从仓库里删掉了，但它的行为固定在 `tests/baseline/` 里（全部是虚构的合成数据）：

| 快照 | 内容 | 谁在用 |
|---|---|---|
| `golden/NNN_*.json`（172 个文件） | 旧版在每个接口 × 每种状态下的真实响应（状态码、稳定的响应头、规范化后的正文） | `backend/tests/api_golden.rs` 按顺序对新的路由重放它们，并逐项比较 |
| `fixtures/*.xlsx` + `parser_golden/*.json`（49 个） | 合成的工作簿，以及旧版解析器对每一份的输出 | `backend/tests/parser_golden.rs`（三列和四列格式必须逐字节保持不变） |
| `golden/id_vectors.json`（85 条断言） | 关于题目 id 的断言；每一条都带着被哈希的字节 | `backend/tests/id_vectors.rs` |
| `fixtures/sample.db` + `golden/store_sample_outputs.json` | 旧实现写的示例数据库，以及它每个接口的输出 | `backend/tests/db_golden.rs`（也检查它迁移到当前 schema） |
| `golden/allowed_differences_rust.json`（23 条）、`allowed_probe_differences_rust.json`（51 条） | 新实现有意和旧版不同的地方，每一处都有一句理由 | `api_golden.rs` 里没有登记的差异 = 失败 |

- 要**改变**行为（例如新增一处有意的差异）：先改代码，再把差异和理由登记进 `allowed_differences_rust.json`；不要直接改黄金响应。
- 旧的抓取 / 对照 / 探测脚本依赖旧的 Python 代码，已经随它一起删掉了；需要的话，从 Git 标签 `python-v0.2` 和提交历史里找回来。

## 6. 数据安全规则

- **不要**读取、打印、提交或修改 `data/`、`progress.json` 或你自己的题库 Excel 文件。
- 永远在副本或临时目录里做实验；不要对真正的 `xexam.db` 运行「试试看会怎样」的命令。记住，用较新的版本打开较旧的数据库会迁移它（并把它切换到 WAL）。
- 不要运行 `docker compose down -v`。
- 代码、测试和文档里的示例只用虚构的内容，永远不用真实的考试题目（演示题库「General knowledge」和「Kitchen basics」，或者 `frontend/src/pages/banks.ts` 里虚构的六列格式示例）。

## 7. CI

只有一条流水线：[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。推送到 `main` 和 `dev`、向 `main` 提拉取请求，以及手动启动时，它都会运行。

| 任务 | 做什么 |
|---|---|
| `check` | `VERSION` 里**正好有一行** `CODE_VERSION=` 和**一行** `RELEASE_VERSION=`，两者都形如 `0.7` 或 `0.6.5`，而且 `RELEASE_VERSION` 不比 `CODE_VERSION` 新；三份 README 各自最新的更新日志条目 = `CODE_VERSION`；每份 README 顶部的发布徽章 = `RELEASE_VERSION`；`backend/Cargo.toml` 和 `frontend/package.json` 写的是 `CODE_VERSION`；文件布局（根目录只有 `README.md`，另外两份 README 在 `README/` 里，compose 文件和 `.env.example` 在 `docker/` 里）和文档检查（第 8 节）。它输出的 `version`（`CODE_VERSION`）供后面的任务使用 |
| `frontend` | 类型检查、119 个单元测试、构建 |
| `backend` | fmt、clippy、302 个测试，以及 `progress.json` 迁移检查 |
| `stack` | 构建 `xexam-frontend` 镜像，跑 38 项 Nginx 检查；启动完整的 compose 栈，跑端到端检查 |
| `arm64` | 为 arm64 构建两个镜像，在 QEMU 下启动它们，检查 `/api/info` 报告的是 `aarch64` 和 `CODE_VERSION` |
| `publish` | 只有 `main`：把两个镜像（amd64 + arm64）以 `latest`、`v<CODE_VERSION>` 和 `sha-<提交>` 标签推送到 Docker Hub（`docker.io/yuminspace/`，公开，`docker/docker-compose.yml` 拉取的就是它）和 ghcr.io（私有镜像副本），并在两边都核对清单和标签。需要仓库机密 `DOCKERHUB_USERNAME` 和 `DOCKERHUB_TOKEN`（变量 `DOCKERHUB_NAMESPACE` 可选）；没有它们时，会在推送任何东西之前停下 |
| `release` | 只有 `main`：建立（或更新）GitHub Release `v<CODE_VERSION>`，说明取自 `README.md` 里的更新日志（`scripts/release-notes.py`）。比 `main` 上已经记录的发布更旧的版本的运行，不会把它的 Release 标成最新版。它的最后一步记录这次发布（第 7.1 节） |

推送到 `dev` 和向 `main` 提拉取请求只跑前五个任务，**从不发布**；只有推送到 `main` 才会发布。手动启动的运行，和推送到它所在的分支效果一样。

### 7.1 发布之后：机器人的提交

只有镜像已经发布、GitHub Release 也已经存在时，`release` 任务的最后一步才会改动 `RELEASE_VERSION`：

1. 它在最新的 `origin/main` 上运行 `scripts/set-release-version.py <版本>`，设置 `VERSION` 里的 `RELEASE_VERSION`，以及三份 README 顶部的发布徽章（它的替代文字、URL 和宽度）。
2. 它把这些提交为 **`Record release v<版本>: RELEASE_VERSION and the README badge [skip ci]`**，并推送到 `main`。用工作流自己的令牌推送不会启动新的运行，提交信息里也写着 `[skip ci]`。没有任何变化时（重新运行），它什么都不提交。
3. 如果 `dev` 没有自己的提交，就把 `dev` 快进到同一个提交。如果 `dev` 已经往前走了（或者在发布期间往前走了），它就不动 `dev`，并在运行里显示一条**提示**：`merge main into dev (git pull origin main on dev) before the next merge to main`（意思是：下一次合并到 main 之前，先把 main 合并进 dev）。不管哪种情况，发布都已经完成。

`set-release-version.py` 从不把 `RELEASE_VERSION` 往回调：重新运行一次较早的运行的发布任务，什么都不改，以 0 退出。它拒绝比 `CODE_VERSION` 更新的版本。

所以每次发布之后，`origin/main`（通常还有 `origin/dev`）比你的克隆多一个提交：

- **下一次推送之前**，先更新这两个分支：`git checkout main && git pull`，然后 `git checkout dev && git pull`。
- **运行里显示了那条提示时**，在下一次合并到 main 之前把 main 合并进 dev：在 `dev` 上 `git pull origin main`，然后推送 `dev`。否则下一次合并到 `main` 时会漏掉机器人的提交，或者在 `VERSION` 和徽章上发生冲突。

## 8. 文档

- 每种语言一个文件夹：`docs/en`（英语，原文）、`docs/ja`（日语）和 `docs/zh-CN`（简体中文）。每个文件夹里都有同样的七份文档：`api`、`architecture`、`database`、`deployment`、`development`、`migration`、`security`。
- `docs/zh-CN/history/` 里是切换到 Rust 时的旧文档（审计、计划、风险评估）。它们只有中文，不再更新。
- `docs/en/openapi.yaml` 由所有语言共用；日语和中文的文档链接到 `../en/openapi.yaml`。
- 每份文档开头都有一条语言栏，链接到另外两个文件夹里的同一份文档。链接相对于文件自己所在的文件夹：仓库根目录是 `../../`。
- 先改英语的文档，再在同一次修改里改日语和中文的。界面上的标签用粗体，写法和那种语言的文字目录完全一样；题目语言的切换按钮总是写成 **English** / **中文** / **日本語**。
- 七份文档中有一份在某个语言文件夹里缺失，`docs/` 里有 `en`、`ja` 和 `zh-CN` 以外的东西，或者 README 或文档里的相对链接或图片路径指向不存在的文件时，CI 的 `check` 任务会失败。指向 GitHub 页面的链接（`../../releases` 之类）也会检查：它们必须正好回到仓库页面，所以在 `README/` 里要以 `../../../` 开头。

## 9. 版本、更新日志和许可证

- 仓库根目录的 **`VERSION`** 里有两行 `KEY=VALUE`（以 `#` 开头的行是注释），每行正好出现一次：
  - `CODE_VERSION`（现在是 `0.7`）：这个分支上代码的版本。手动调高它，同时在三份 README 里写更新日志条目，并改 `backend/Cargo.toml` 和 `frontend/package.json`（`0.7.0`；还有它们的锁文件）。镜像用它来构建和打标签，`/api/info` 报告的也是它（后端从镜像里的 `/VERSION` 读取它）。
  - `RELEASE_VERSION`（v0.7 的发布任务运行之前是 `0.6`）：最后一个真正发布出去的版本。**永远不要手动修改它**：由发布任务写入，同时更新 README 顶部的发布徽章（第 7.1 节）。单纯的推送或合并从不改变它。
- 每次更新都要在三份 README（`README.md`、`README/README.ja.md`、`README/README.zh-CN.md`）的更新日志最上面加一条，版本号等于 `CODE_VERSION`。CI 会检查这一点。GitHub Release 的说明取自英语的那一份。
- 程序的关于页面显示这些更新日志，它们是构建进前端的（第 3 节）。条目请只用简单的 Markdown：段落、`- ` 列表、缩进的 `  - ` 子列表、`**粗体**` 和 `` `代码` ``；指向仓库里文件的链接在那里显示成普通文字。修改了的更新日志，要重新构建前端之后才会出现在程序里。
- 在 `dev` 上开发；`main` 接收合并，推送到 `main` 会发布镜像和 Release。每次发布之后都要拉取 `main` 和 `dev`（第 7.1 节）。
- 本项目采用 MIT 许可证（仓库根目录的 `LICENSE`）。
