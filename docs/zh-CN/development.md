[English](../en/development.md) | [日本語](../ja/development.md) | **简体中文**

# 开发

> 日常改代码、跑测试用的命令，以及各部分代码放在哪里。架构见 [architecture.md](architecture.md)；最初的测试思路见 [test-strategy.md](history/test-strategy.md)。

## 1. 工具

| 工具 | 版本 | 说明 |
|---|---|---|
| Rust | 1.99.1（由 `backend/rust-toolchain.toml` 固定） | `rustup` 会自动安装。`rusqlite` 的 `bundled` 特性需要 C 编译器（Linux：`build-essential`；Windows：MinGW gcc 或 MSVC） |
| Node | 24 | 前端；单元测试用 Node 自带的测试运行器，加 `--experimental-strip-types` |
| Python | 3.8+ | 只用于 `tests/integration/` 里的集成脚本。**只用标准库** |
| Docker | Compose v2 | 整栈检查、镜像 |

## 2. 后端

```bash
cd backend
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 181 个测试
cargo run -- serve --data ../data --no-browser     # 监听 127.0.0.1:8765
```

- 测试对照的是 `tests/baseline/` 里已提交的**旧版快照**，不需要 Python。
- 所有测试都用临时目录里的全新数据库或 `tests/baseline/fixtures/sample.db` 的副本，**不会碰 `data/`**。v0.5 的行为（schema 迁移和备份、题库、考试记录、设置、新路由上的守卫）在 `backend/tests/v05_banks_records_settings.rs` 里。
- 常用环境变量：`XEXAM_DATA`、`XEXAM_PORT`、`XEXAM_ALLOWED_HOSTS`、`XEXAM_LOG`（日志级别，如 `debug`）。完整列表见 `xexam-backend --help` 和 [deployment.md](deployment.md)。
- 命令行导入（导入到 legacy 题库）：`cargo run -- import ../some.xlsx --data ../data`。
- 让后端打开 schema 1 的数据库（v0.4 或更早）时，会迁移它，并在它旁边写出 `backups/`。请用副本。

## 3. 前端

```bash
cd frontend
npm ci
npm run typecheck      # 先对应用跑 tsc，再对测试文件跑（tsconfig.test.json）
npm test               # 44 个单元测试：判分和抽题、语言规则、保存的考试、界面文案表、后端消息翻译、更新日志解析
npm run build          # 类型检查 + vite build → dist/
npm run dev            # http://127.0.0.1:5173，/api 代理到 http://127.0.0.1:8765
```

- 开发时请用 `localhost` 或 `127.0.0.1` 打开页面。Vite 代理的 `changeOrigin` 必须是 `false`：后端会比较 `Host` 与 `Origin`，改写 `Host` 会让写请求被 403。
- 后端不在默认地址时：`XEXAM_BACKEND=http://127.0.0.1:9000 npm run dev`。
- `npm run build` 和 `npm run dev` 会读取仓库根目录的 `README.md` 以及 `README/README.zh-CN.md` 和 `README/README.ja.md`：`vite.config.ts` 里的插件 `xexam-changelog` 把它们的更新日志变成模块 `virtual:changelog`，给「关于」页（`#/about`）的「更新内容」用。哪一份 README 里没有 `### vX.Y` 条目，构建就失败。开发服务器会监视这三个文件的改动。
- 只跑一个测试文件：`node --experimental-strip-types --test src/i18n/server-messages.test.ts`。
- 导入路径用 `.ts` 扩展名（Node 自带的测试运行器要求）。
- TypeScript 固定在 `~5.9.3`。

### 3.1 界面文字

用户看到的每一段文字都来自文案表（catalog），页面里不直接写文字。

- **位置**：`src/i18n/catalog/<area>.ts`，每个区域一个文件：`common`、`home`、`banks`、`records`、`settings`、`quiz`、`result`、`about`。每个文件都是 `export default defineCatalog({ en: {…}, zh: {…}, ja: {…} })`。
- **键名**以区域开头：`banks.delete.title`、`records.deleteAll.ok`。`common.ts` 放共用的前缀（`app.`、`nav.`、`common.`、`err.`、`kind.`、`mode.`、`act.` 等）。
- **永远是三种语言。** `zh` 或 `ja` 块缺了某个英文键，TypeScript 会报错。以 `_one` 结尾的键是英文的单数形式：数量为 1 时 `tn()` 会用它；中文和日文里可以不写。
- **占位符**写作 `{name}`：`t('records.deleteOne.body', { at, pct })`。数量用 `tn(key, n)`，它会填入 `{n}`。
- **检查**：`src/i18n/catalog.test.ts` 确认三种语言的键和占位符完全相同、没有空文本、每个键都带自己区域的前缀并且只出现在一个文案表里，以及每个文案表文件都已在 `src/i18n/index.ts` 里登记。
- **新增文案表文件**：在 `src/i18n/index.ts` 里 import 它，并把它加进 `ALL` 和 `MsgKey` 类型。
- 先写英文，再写中文和日文。用界面已经在用的说法（例如中文的「题库」「下架」「错题本」，英文的 "question bank"、"retired"、"wrong-answer notebook"）。

### 3.2 界面里的后端消息

后端的错误消息是中文，而且固定不变（它们是经过测试的 API 的一部分）。前端负责翻译：

- `src/i18n/server-messages.ts`：`EXACT` 对应整条消息，`PATTERNS` 对应后端的格式字符串（其中的 `{}` 部分会被捕获成值）。中文界面原样显示消息；不认识的文本也原样显示。少数底层协议错误是英文单词（`not found`、`bad json` 等）；它们在每种语言里都有一句译文，中文也不例外。
- `src/i18n/errors.ts`：`errorText(e)` 把任何错误变成当前语言的文字。页面显示的每个错误都要经过它。页面自己的错误是一个 `LocalError(key, params)`，所以界面换语言时它也跟着换。
- **漂移测试**：`src/i18n/server-messages.test.ts` 用一个小词法分析器读取 `backend/src` 里的 Rust 源码。每个含中文或日文字符的字符串字面量（注释、测试代码、断言、日志和控制台输出除外）都必须有英文和日文译文；译文要保留原文里的占位符，并且不能含中文。如果某条译文对应的后端文本已经不存在，测试也会失败。
- **新增或修改后端消息时**：在同一次改动里往 `server-messages.ts` 加上译文。如果这个字符串不会显示给用户（比如解析器要查找的某个词），就把它加进测试里的 `NOT_MESSAGES`，并写明原因。

### 3.3 新增页面或路由

1. **路由**：在 `src/router.ts` 里给 `Route` 加一个变体，给 `parseHash` 加一个 case；如果它的 hash 不是 `#/<name>`，也给 `routeHash` 加一个。在 `sectionOf` 里写明它在菜单里高亮哪一项（`null` = 不高亮）。
2. **页面**：新建 `src/pages/<name>.ts`，里面有一个返回 `HTMLElement` 的 `render<Name>()` 函数。用 `src/utils/dom.ts` 里的 `h()` 构建 DOM。样式放在 `src/styles/<name>.css`，在页面文件顶部 import 它，并使用 `main.css` 里的颜色变量，这样深色主题才能正常显示。
3. **接入**：在 `src/main.ts` 的 `page()` 里加一个 case，写上浏览器标签页用的标题（顶栏只显示 Xexam 标志，不显示页面标题）。
4. **菜单条目**（只有主栏目才需要）：加到 `src/router.ts` 的 `NavSection`，以及 `src/components/shell.ts` 的 `NAV`、`HREF`，图标加到 `ICONS`。下拉菜单里分隔线下面的「Xexam vX.Y」（打开「关于」页）不在 `NAV` 里，是在 `shell()` 里单独写的。
5. **文字**：新建文案表 `src/i18n/catalog/<name>.ts`（见 3.1 节），三种语言都要写。
6. **状态**：页面之间从不互相 import。共享状态放在 `src/state/store.ts`；修改要通过 `src/state/actions.ts` 里的函数（`navigate(route)`、`render()` 等）。只有本页面用到的状态留在页面模块的变量里。`onRouteChange()` 用来在用户离开或进入某个页面时执行代码。
7. **API**：新端点要在 `src/api/` 里有一个带类型的封装，类型放在 `src/types/api.ts`，并写进 [api.md](api.md) 和 [openapi.yaml](../en/openapi.yaml)。

## 4. 整栈

```bash
# 构建并启动两个容器（同一个 xexam_data 卷！自己的数据会被用到；要试验请换一个项目名，例如 -p some-other-name）
docker compose -f docker/docker-compose.build.yml up -d --build

# 检查脚本都用临时项目名和临时卷，不碰 xexam_data
docker build -f nginx/Dockerfile -t xexam-frontend:test .
python -I tests/integration/nginx_check.py xexam-frontend:test   # 38 项，用一个假后端
python -I tests/integration/stack_check.py                      # 34 项，起整个栈（会构建两个镜像）
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend         # 10 项：旧 progress.json 的迁移（需要先 cargo build）
```

- compose 文件和 `.env` 在 `docker/` 里。镜像的构建上下文是仓库根目录（`docker/docker-compose.build.yml` 里写作 `..`）。`nginx/Dockerfile` 先复制 `frontend/`，再执行 `COPY README.md /` 和 `COPY README/README.ja.md README/README.zh-CN.md /README/`，因为 `vite.config.ts` 从前端目录的上一级读取这三份 README（给「关于」页的更新日志）。所以 `.dockerignore` 排除数据、构建产物、`docs`、`tests` 等，但不排除 `README.md` 和 `README/`。最终镜像里只有 `nginx.conf` 和 `dist/` 里的静态文件。

## 5. 对照旧版（回归）

旧的 Python 实现已经从仓库移除，但它的行为被固定在 `tests/baseline/` 里（都是虚构的合成数据）：

| 快照 | 内容 | 谁用它 |
|---|---|---|
| `golden/NNN_*.json`（172 份） | 旧版每个端点 × 每种状态的真实响应（状态码、稳定的响应头、规范化后的正文） | `backend/tests/api_golden.rs` 按顺序重放到新路由并逐项比较 |
| `fixtures/*.xlsx` + `parser_golden/*.json`（49 个） | 合成工作簿及旧解析器对每个的输出 | `backend/tests/parser_golden.rs` |
| `golden/id_vectors.json`（85 条断言） | 题目 id 的断言，每条带被哈希的字节 | `backend/tests/id_vectors.rs` |
| `fixtures/sample.db` + `golden/store_sample_outputs.json` | 旧实现写出的示例库及其各接口输出 | `backend/tests/db_golden.rs`（也检查它迁移到 schema v2） |
| `golden/allowed_differences_rust.json`（23 项）、`allowed_probe_differences_rust.json`（51 项） | 新实现有意与旧版不同的地方，每项一句理由 | `api_golden.rs` 里没登记的差异 = 失败 |

- 要**改变**行为（比如新增一条有意的差异）：改代码，再在 `allowed_differences_rust.json` 里登记并写明理由；不要直接改黄金响应。
- 旧的抓取 / 对照 / 探针脚本依赖旧 Python 代码，已随它移除；需要时从 Git 标签 `python-v0.2` 和提交历史里找回。

## 6. 数据安全守则

- **不要**读取、打印、提交或修改 `data/`、`progress.json`、你自己的题库 Excel 文件。
- 试验永远在副本或临时目录上做；不要对真实的 `xexam.db` 跑「看看会怎样」的命令。别忘了：用 v0.5 打开更旧的数据库会迁移它。
- 不要运行 `docker compose down -v`。
- 代码、测试和文档里的示例只用编造的内容，绝不用真实的考试题目。

## 7. CI

只有一个流水线：[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。推送到 `main` 和 `dev`、向 `main` 提拉取请求，以及手动启动时，它都会运行。

| 作业 | 内容 |
|---|---|
| `check` | `VERSION` 格式；三个 README 各自最新的一条更新日志 = `VERSION`；`backend/Cargo.toml` 和 `frontend/package.json` 的版本号相同；目录布局（根目录只有 `README.md`，另外两份 README 在 `README/`，compose 文件和 `.env.example` 在 `docker/`）和文档检查（第 8 节） |
| `frontend` | 类型检查、44 个单元测试、构建 |
| `backend` | fmt、clippy、181 个测试，以及 `progress.json` 迁移检查 |
| `stack` | 构建 `xexam-frontend` 镜像跑 38 项 Nginx 检查；起整个 compose 栈跑 34 项端到端检查 |
| `arm64` | 为 arm64 构建两个镜像，在 QEMU 下启动，检查 `/api/info` 报的是 `aarch64` 和正确的版本号 |
| `publish` | 仅 `main`：两个镜像（amd64 + arm64）用相同的标签推到 Docker Hub（`docker.io/yuminspace/`，公开，`docker/docker-compose.yml` 拉取的就是它）和 ghcr.io（私有镜像副本），并在两边核对清单和标签。需要仓库机密 `DOCKERHUB_USERNAME` 和 `DOCKERHUB_TOKEN`（可选：变量 `DOCKERHUB_NAMESPACE`）；没有它们，会在推送任何东西之前停下 |
| `release` | 仅 `main`：建（或更新）`v<VERSION>` 的 GitHub Release，说明取自 `README.md` 的更新日志 |

推送到 `dev` 和向 `main` 提拉取请求只跑前五项，**从不发布**；只有推送到 `main` 才会发布。手动启动的运行，效果和推送到它所在的分支一样。

## 8. 文档

- 每种语言一个文件夹：`docs/en`（英文，原文）、`docs/ja`（日文）和 `docs/zh-CN`（简体中文）。每个文件夹里都有同样的七份文档：`api`、`architecture`、`database`、`deployment`、`development`、`migration`、`security`。
- `docs/zh-CN/history/` 里是改用 Rust 时期的旧文档（审计、计划、风险评估）。它们只有中文，不再更新。
- `docs/en/openapi.yaml` 由所有语言共用；日文和中文文档链接到 `../en/openapi.yaml`。
- 每份文档开头都有一个语言栏，链接到另外两个文件夹里的同一份文档。链接都相对于文件自己所在的文件夹：仓库根目录是 `../../`。
- 先改英文文档，再在同一次改动里改日文和中文文档。
- 以下情况 CI 的 `check` 作业会失败：某个语言文件夹里缺了七份文档中的一份；`docs/` 里除了 `en`、`ja`、`zh-CN` 三个文件夹还有别的东西；README 或文档里的相对链接或图片路径指向不存在的文件。指向 GitHub 页面的链接（`../../releases` 等）也会检查：必须正好回到仓库页面，所以在 `README/` 里要以 `../../../` 开头。

## 9. 版本号、更新日志和许可证

- 版本号的唯一来源是仓库根目录的 `VERSION`（`0.6`）。`backend/Cargo.toml` 和 `frontend/package.json` 里各有一份（`0.5.1`）；CI 会检查它们是否一致。
- 每次更新都要在三个 README（`README.md`、`README/README.ja.md`、`README/README.zh-CN.md`）的更新日志最上面加一条，版本号与 `VERSION` 一致。CI 会检查这一点：它取每份 README 里第一行以 `### v` 加数字开头的标题，所以更新日志前面不能有别的以 `### v` 开头的标题。GitHub Release 的说明取自英文那一份。
- 「关于」页的「更新内容」也来自这些更新日志（见第 3 节），每种界面语言显示对应语言的 README。页面只认简单的 Markdown：段落、以 `- ` 开头的列表项、多缩进两个空格的子列表项、`**粗体**`，以及用反引号括起来的代码；指向仓库里文件的链接显示成普通文字。所以更新日志里只用这些写法。
- 在 `dev` 上开发；`main` 接收合并，推送到 `main` 会发布镜像和 Release。
- 本项目使用 MIT 许可证（仓库根目录的 `LICENSE`）。
