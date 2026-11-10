# 开发

> 日常改代码、跑测试用的命令。架构见 [architecture.md](architecture.md)，测试思路见 [test-strategy.md](test-strategy.md)。

## 1. 工具

| 工具 | 版本 | 说明 |
|---|---|---|
| Rust | 1.99.1（`backend/rust-toolchain.toml` 固定） | `rustup` 会自动装。`rusqlite` 的 `bundled` 需要 C 编译器（Linux：`build-essential`；Windows：MinGW gcc 或 MSVC） |
| Node | 24 | 前端 |
| Python | 3.8+ | 只用来跑测试：黄金响应对照、解析器对照（「旧版是标准答案」）。**只用标准库** |
| Docker | Compose v2 | 整栈检查、镜像 |

## 2. 后端

```bash
cd backend
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets          # 136 个测试
cargo run -- serve --data ../data --no-browser     # 监听 127.0.0.1:8765
```

- 测试对着 `tests/baseline/` 里已提交的**旧版快照**，不需要 Python（只有 `migrate_check.py` 这类集成脚本用 Python 标准库跑）。
- 所有测试用临时目录里的全新数据库或 `tests/baseline/fixtures/sample.db` 的副本，**不会碰 `data/`**。
- 常用环境变量：`XEXAM_DATA`、`XEXAM_PORT`、`XEXAM_ALLOWED_HOSTS`、`XEXAM_LOG`（日志级别，如 `debug`）。完整列表见 `xexam-backend --help` 和 [deployment.md](deployment.md)。
- 命令行导入：`cargo run -- import ../some.xlsx --data ../data`。

## 3. 前端

```bash
cd frontend
npm ci
npm run typecheck      # tsc（含测试文件）
npm test               # 19 个单元测试（判分、错题规则、sessionStorage）
npm run build          # 类型检查 + vite build → dist/
npm run dev            # http://127.0.0.1:5173，/api 代理到 http://127.0.0.1:8765
```

- 开发时请用 `localhost` 或 `127.0.0.1` 打开页面。Vite 代理的 `changeOrigin` 必须是 `false`：后端会比较 `Host` 与 `Origin`，改写 `Host` 会让写请求被 403。
- 后端不在默认地址时：`XEXAM_BACKEND=http://127.0.0.1:9000 npm run dev`。
- 导入路径用 `.ts` 扩展名（Node 自带的测试运行器要求）。
- TypeScript 固定在 `~5.9.3`（`openapi-typescript` 的 peer 依赖）。

## 4. 整栈

```bash
# 构建并启动两个容器（同一个 xexam_data 卷！自己的数据会被用到，要试验请另用项目名：-p 别的名字）
docker compose -f docker-compose.build.yml up -d --build

# 检查脚本都用临时项目名和临时卷，不碰 xexam_data
docker build -f nginx/Dockerfile -t xexam-frontend:test .
python -I tests/integration/nginx_check.py xexam-frontend:test   # 38 项，用一个假后端
python -I tests/integration/stack_check.py                      # 34 项，起整个栈（会构建两个镜像）
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend         # 10 项：旧 progress.json 的迁移（需要先 cargo build）
```

## 5. 对照旧版（回归）

旧的 Python 实现已经从仓库移除，但它的行为被固定在 `tests/baseline/` 里（都是虚构的合成数据）：

| 快照 | 内容 | 谁用它 |
|---|---|---|
| `golden/NNN_*.json`（172 份） | 旧版每个端点 × 每种状态的真实响应（状态码、稳定的响应头、规范化后的正文） | `backend/tests/api_golden.rs` 按顺序重放到新路由并逐项比较 |
| `fixtures/*.xlsx` + `parser_golden/*.json`（49 个） | 合成工作簿及旧解析器对每个的输出 | `backend/tests/parser_golden.rs` |
| `golden/id_vectors.json`（85 条） | 题目 id 的断言，每条带被哈希的字节 | `backend/tests/id_vectors.rs` |
| `fixtures/sample.db` + `golden/store_sample_outputs.json` | 旧实现写出的示例库及其各接口输出 | `backend/tests/db_golden.rs` |
| `golden/allowed_differences_rust.json`（23 项）、`allowed_probe_differences_rust.json`（51 项） | 有意与旧版不同的地方，每项一句理由 | `api_golden.rs` 里没登记的差异 = 失败 |

- 要**改变**行为（比如新增一条有意的差异）：改代码，再在 `allowed_differences_rust.json` 里登记并写明理由；不要直接改黄金响应。
- 旧的抓取 / 对照 / 探针脚本依赖旧 Python 代码，已随它移除；需要时从 Git 标签 `python-v0.2` 和提交历史里找回。

## 6. 数据安全守则

- **不要**读取、打印、提交或修改 `data/`、`progress.json`、你自己的题库 Excel。
- 试验永远在副本或临时目录上做；不要对真实的 `xexam.db` 跑「看看会怎样」的命令。
- 不要运行 `docker compose down -v`。

## 7. CI

只有一个流水线：[`.github/workflows/ci.yml`](../.github/workflows/ci.yml)。

| 作业 | 内容 |
|---|---|
| `check` | `VERSION` 格式、README 更新日志最上面一条 = `VERSION` |
| `frontend` | 类型检查、19 个单元测试、构建 |
| `backend` | fmt、clippy、136 个测试，以及 `progress.json` 迁移检查 |
| `stack` | 构建 `xexam-frontend` 镜像跑 38 项 Nginx 检查；起整个 compose 栈跑 34 项端到端检查 |
| `arm64` | 两个镜像为 arm64 构建，在 QEMU 下启动，检查 `/api/info` 报 `aarch64` |
| `publish` | 仅 `main`：两个镜像（amd64 + arm64）推到 ghcr.io，核对清单和标签 |
| `release` | 仅 `main`：建（或更新）`v<VERSION>` 的 GitHub Release |

拉取请求和其它分支只跑前五项，**从不发布**。

## 8. 版本号和更新日志

- 版本号的唯一来源是仓库根目录的 `VERSION`。`Cargo.toml` 和 `package.json` 里各有一份，要保持一致。
- 每次更新都要在 README 的「更新日志」最上面加一条，版本号与 `VERSION` 一致（CI 会校验）。
