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
cargo test --all-targets          # 约 140 个测试
cargo run -- serve --data ../data --no-browser     # 监听 127.0.0.1:8765
```

- 测试里有一部分要调用旧的 Python 代码当 oracle（`tests/common/py_oracle.py`）。找不到 Python 时这些测试**会跳过**；设 `XEXAM_REQUIRE_PYTHON=1` 则改成失败（CI 就是这样）。
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
# 构建并启动两个容器（同一个 xexam_data 卷！自己的数据会被用到，要试验请先改 .env 或另用项目名）
docker compose -f docker-compose.modern.yml up -d --build

# 检查脚本都用临时项目名和临时卷，不碰 xexam_data
docker build -f nginx/Dockerfile -t xexam-web:test .
python -I tests/integration/nginx_check.py xexam-web:test       # 38 项，用一个假后端
python -I tests/integration/stack_check.py                      # 34 项，起整个栈（会构建两个镜像）
python -I tests/integration/rollback_check.py backend/target/release/xexam-backend   # 19 项
python -I tests/integration/migrate_check.py backend/target/debug/xexam-backend         # 10 项：旧 progress.json 的迁移（需要先 cargo build）
```

## 5. 对照旧版（回归）

旧版是标准答案。`tests/baseline/` 里固定了它的行为：

| 目的 | 命令 |
|---|---|
| 用新后端抓一遍，和旧版的 172 份黄金响应比较 | `python -I tests/baseline/tools/run_rust_capture.py backend/target/release/xexam-backend` |
| 安全探针（3 种配置） | 先 `python -I tests/baseline/tools/security_probe.py --spawn-python <旧版代码目录> --no-dos --out python.json` 生成旧版基准，再 `python -I tests/baseline/tools/probe_rust.py backend/target/release/xexam-backend python.json` |
| 解析器黄金输出（旧解析器） | `python -I tests/baseline/tools/run_all.py` |
| 旧版 v0.2 的修复检查 | `python -I tests/baseline/tools/v02_fix_checks.py` |

- 有意的差异登记在 `tests/baseline/golden/allowed_differences_rust.json`（23 项）和 `allowed_probe_differences_rust.json`（51 项），每项一句理由。**没登记的差异 = 缺陷。**
- 旧版代码取自 `tests/baseline/tools/paths.py` 里的 `LEGACY_ROOT`（现在是仓库根目录的 `server.py` / `app/`；切换到 `legacy/python/` 之后这里要同步改）。
- `tests/baseline/fixtures/` 里的 `.xlsx` 和 `sample.db` 全是**虚构**的，可以入库；真实题库永远不入库。

## 6. 数据安全守则

- **不要**读取、打印、提交或修改 `data/`、`progress.json`、你自己的题库 Excel。
- 试验永远在副本或临时目录上做；不要对真实的 `xexam.db` 跑「看看会怎样」的命令。
- 不要运行 `docker compose down -v`。

## 7. CI

| 工作流 | 触发 | 内容 |
|---|---|---|
| `modern.yml` | 分支 `modernize/**`、对 `main` 的 PR | `frontend`（类型检查、单元测试、构建）、`backend`（fmt、clippy、测试，Python oracle 必须真的运行）、`stack`（构建 nginx 镜像 → `nginx_check.py` → 整栈 `stack_check.py`）。**从不发布** |
| `docker.yml` | 推到 `main`、标签 | 旧版的检查、测试、发布、自动 Release（推 `main` 会移动 `latest`） |

## 8. 版本号和更新日志

- 版本号的唯一来源是仓库根目录的 `VERSION`。`Cargo.toml` 和 `package.json` 里各有一份，要保持一致。
- 每次更新都要在 README 的「更新日志」最上面加一条，版本号与 `VERSION` 一致（CI 会校验）。
