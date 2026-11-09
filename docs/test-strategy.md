# 测试策略（Phase 0）

> 与 [architecture-audit.md](architecture-audit.md)、[migration-plan.md](migration-plan.md)、[risk-assessment.md](risk-assessment.md) 配套。
> 原则：**旧的 Python 版本就是标准答案**。任何没法解释的新旧差异都按缺陷处理；没执行过的测试不算通过。

## 1. 原则

1. **旧版是 oracle**。Phase 1 已经把旧版的行为固定在 `tests/baseline/`：172 份真实响应、50 个 XLSX 夹具及解析输出、85 条题目 id 向量。新实现要和它们逐项对照。
2. **差异必须有名字**。有意的差异写进 `tests/baseline/golden/allowed_differences_*.json`，每一项一句理由；没写的差异 = 失败。
3. **只在副本上测**。永远不碰业主的真实数据库；测试库是临时目录里的全新库，或经 `PRAGMA integrity_check` 的副本（见 §8）。
4. **真实工作簿不入库**。业主的两份题库只在本机做"只读的结构统计"，不进仓库；仓库里的夹具全是虚构的。
5. **同一套测试跑两个后端**。工具都参数化了基址（`--base-url`），可对旧 Python 和新 Rust 各跑一遍再 `diff_golden.py` 比较。

## 2. 已有的基线（Phase 1 的成果）

| 资产 | 位置 | 说明 |
|---|---|---|
| 黄金响应（172） | `tests/baseline/golden/NNN_*.json` + `index.json` | 每个端点 × 每种状态：状态码、稳定的响应头、规范化后的正文。易变值（`token`、`at`、`mtime`、`version`、`arch`）替换成 `<VOLATILE>` |
| 允许的差异 | `tests/baseline/golden/allowed_differences_v02.json` | 24 项，对应 v0.2 的审计修复 |
| 解析器夹具（50） | `tests/baseline/fixtures/*.xlsx` | 全是虚构的合成文件，覆盖行内字符串 / 共享字符串（含富文本、注音）/ 工作表部件名不是 `sheet1.xml` / 隐藏表 / 缺失单元格 / CRLF、制表符、全角空格 / 14 种选项标签 / 9–10 个选项 / 热点题 20 种变体 / 40 种答案写法 / 16 种翻译情形 / DOCTYPE 与实体 / 损坏包 / UTF-8 BOM / UTF-16 / 3000 行 |
| 解析器黄金输出 | `tests/baseline/parser_golden/*.json` | 旧解析器对每个夹具的输出（含"服务器会怎么回应"） |
| 题目 id 向量 | `tests/baseline/golden/id_vectors.json` | 54 条直接向量 + 12 个流水线用例（31 个 id），每条带被哈希的字节 |
| 示例数据库 | `tests/baseline/fixtures/sample.db` | 由真实代码从虚构工作簿生成：7 题（6 上架 1 下架）、5 行统计、3 场成绩、2 次导入 |
| 示例库的各接口输出 | `tests/baseline/golden/store_sample_outputs.json` | `bank()`、`progress()`、`import_history()`、`plan_import()` 的真实输出 |
| 过期的进程内测试 | `tests/baseline/tools/expiry_inprocess.py` | 不用等 30 分钟：过期、双取、错误令牌不消耗、50 线程抢同一令牌只有 1 个赢 |
| 安全探针 | `tests/baseline/tools/security_probe.py` | 340 项黑盒探测（Host、Origin、方法、Content-Type、请求体、上传、实体、待确认、尝试、静态、关闭、DoS、响应头），`--golden` 与旧结果比较 |
| Host/Origin 检查 | `tests/baseline/tools/host_origin_test.py` | 55 项，纯函数 + 真实服务端 |
| 隧道模拟 | `tests/baseline/tools/tunnel_sim.py` | 真实 Chrome 经 HTTPS 代理：保留 Host / 改写 Host 两种场景，上传→预览→确认→交卷 |
| v0.2 修复检查 | `tests/baseline/tools/v02_fix_checks.py` | 26 项 |
| 比较工具 | `tests/baseline/tools/diff_golden.py` | 比两个抓取目录，支持白名单 |
| 路径 | `tests/baseline/tools/paths.py` | `XEXAM_LEGACY_ROOT` 指定被测代码；不写死本机路径 |

怎么重新抓取并比较：

```bash
# 对旧版 Python（在仓库根目录；它会自己起服务器、用临时库、并预先写好迁移标记，所以不会读 progress.json）
python -I -B tests/baseline/tools/capture.py --out /tmp/golden_py
python -I tests/baseline/tools/diff_golden.py tests/baseline/golden /tmp/golden_py --allow tests/baseline/golden/allowed_differences_v02.json

# 对新的后端（必须是全新空库、默认配置、未开 shutdown）
python -I -B tests/baseline/tools/capture.py --base-url http://127.0.0.1:9000 --shutdown-url http://127.0.0.1:9001 --out /tmp/golden_rs
python -I tests/baseline/tools/diff_golden.py tests/baseline/golden /tmp/golden_rs --allow tests/baseline/golden/allowed_differences_rust.json
```

## 3. Phase 1：对旧版的基线测试清单

规格要求覆盖的每一项，对应的资产：

| 规格要求 | 覆盖 |
|---|---|
| 每个端点、每种方法 | 黄金 #001–#172；方法 PUT/DELETE/OPTIONS/PATCH/TRACE/CONNECT = #033–#039 |
| 有效与无效请求、错误响应 | #070–#095（Content-Type、JSON、过大、过深）、#096–#111（上传）、#090–#094、#116–#119 |
| XLSX 解析：单选 / 多选 / 热点 | 夹具 `demo`、`f10`–`f14` |
| 日语内容、中文翻译 | `demo`（5 题全有翻译）、`f15`（匹配 / 不匹配） |
| 导入预览、确认、取消 | #114–#133 |
| 待确认过期、一次性确认 | `expiry_inprocess.py`；#116、#121、#131 |
| 数据库写入、重置 | #120–#125、#135–#158、#166–#169 |
| 并发确认 | `expiry_inprocess.py`（50 线程）；Rust 阶段加 HTTP 级并发测试 |
| 安全校验 | `security_probe.py`、`host_origin_test.py`、`tunnel_sim.py` |

## 4. 新旧对比（差分测试）

- **HTTP 级**：`capture.py` 对两个后端抓同一批请求，`diff_golden.py` 比较。差异只能来自白名单。
- **解析器级**：把 50 个夹具各用旧解析器和 Rust 解析器解析，比较 JSON（题目、跳过原因、`zh`）和 id。Python 库产生的错误文案（expat、`KeyError`、`zipfile`）无法复刻，只比较**失败类别 + HTTP 400 + 中文前缀**。
- **数据库级**：旧代码生成的库 → Rust 读 `bank()`/`progress()`/`import_history()` 与 `store_sample_outputs.json` 比较 → Rust 写 → 旧代码再读 → `PRAGMA integrity_check`。

**预计的有意差异**（逐项登记并配测试）：

| 差异 | 原因 |
|---|---|
| 不支持的方法 501 → 405 + `Allow` | 标准行为；旧值是标准库的偶然产物 |
| 非 ASCII 确认码：断开连接 → 409 | 缺陷修复（v0.2 的 Python 已修，所以不是差异） |
| UTF-16 / 非 `.xml` 扩展名里的 DOCTYPE：被展开 → 拒绝 | 安全加固 |
| `HINT_RE` 灾难性回溯 → 线性时间 | `regex` crate 的保证 |
| 错误文案里不再带 Python 异常名和 SQLite 文本 | 不泄露内部细节 |
| `Server` 响应头不再带 Python 版本 | 不泄露版本 |
| HTTP/1.0 → 1.1 | Nginx 在前面 |

## 5. Rust 后端的测试

| 层 | 内容 | 工具 |
|---|---|---|
| 单元 | `is_py_space`（29 个码点）、`canon`、`make_id`（85 条向量）、半偶数取整（与 Python 在 200 万个组合上对照）、`Host`/`Origin` 解析、令牌 | `cargo test` |
| 解析器 | 50 个夹具的差分；恶意 ZIP（声明大小撒谎、重复条目名、zip64、加密、未知压缩方法）→ 全部在上限内被拒绝且 RSS 有界；DOCTYPE 全家桶 | `cargo test` + Python 生成器 |
| 数据库 | 临时文件；schema 与旧库逐字相同；双向读写；`BEGIN IMMEDIATE` 与忙等待；回滚（注入坏行） | `cargo test` |
| 路由 | `tower::ServiceExt::oneshot`，不开端口；守卫顺序；Host/Origin 的全部变体 | `cargo test` |
| 并发 | 50 个任务抢同一个令牌只有 1 个成功；导入与读取并发；`spawn_blocking` 不饿死执行器 | `cargo test` |
| 流式限制 | 20 MiB 与 +1、分块上传、缺 `Content-Length`、`Content-Length` 撒谎 | `cargo test` |
| 优雅停机 | 慢请求在途时 SIGTERM；导入提交完整 | 容器内 `docker stop` 计时 |
| 质量 | `cargo fmt --check`、`cargo clippy -- -D warnings`、`cargo audit` / `cargo deny` | 本机 + CI |

## 6. 前端的测试

- `tsc --noEmit`（`strict`）、`vite build`。
- **UI 流程**（取代业主的临时脚本），用真实 Chrome 点击驱动，不调页面全局：
  空题库 → 上传 → 预览 → 确认 / 取消 → 下架确认弹窗 → 选工作表 / 范围 / 题数 → 单选 / 多选 / 热点作答 → 日语/中文切换（含置灰）→ 暂存退出 → 继续作答 → 交卷（含未答确认）→ 成绩页与错题解析 → 错题本循环 → 清空记录 → 登录过期提示 → 版本页脚 → 窄屏（320 / 375 / 390 / 430）与横屏。
- 判分规则（`grade()`、`stateOf()`、`inNotebook()`）抽成纯函数，写**金标准单元测试**：单选 / 多选 / 热点的所有边界。
- 隧道模拟两种场景在新栈上重跑（经 Nginx）。

## 7. 集成与 Docker 测试

在 `docker compose` 的真实栈上：

1. 干净构建；启动；健康检查变 `healthy`；依赖顺序（nginx 等后端健康）。
2. `/api/info`、静态资源、SPA 回退（若需要）、安全头。
3. 后端**不能从宿主机直接访问**（没有发布端口）；只有 Nginx 暴露。
4. 上传 20 MB 以内成功；超限在 **Nginx 层**和**后端层**都被拒绝，前端看到的是 JSON 文案而不是 HTML 413。
5. 删除容器后用同一个卷重建，数据还在；重启后仍健康。
6. `docker stop` 在 9 秒内结束。
7. Host/Origin：`localhost`、`127.0.0.1`、局域网 IP、公网域名（模拟）、改写 Host 的代理；Nginx 对未知 Host 返回 444。
8. 非 root（uid 10001）；卷的所有权（具名卷首次创建时继承镜像目录的 `app:app`）。
9. arm64：在 CI 里用 QEMU 跑同一份冒烟测试。

## 8. 数据库安全流程（任何接触数据库的测试之前）

```bash
# 备份（停止程序后）
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data.tgz -C /data .

# 在副本上验证
mkdir work && tar xzf xexam-data.tgz -C work
sqlite3 work/xexam.db "PRAGMA integrity_check"      # 必须输出 ok
sha256sum work/xexam.db > work/before.sha256

# ……对 work/ 里的副本跑任何测试……

# 证明原件没动
sha256sum -c work/before.sha256
```

规则：永远对**副本**跑；原件的哈希在测试前后比较；测试 compose 不挂载 `xexam_data`。

## 9. 本机不能跑的测试，以及怎么办

| 测试 | 原因 | 办法 |
|---|---|---|
| Nginx 行为 | 本机没有原生 Nginx | 用 `nginx` 官方镜像在 Docker 里跑（Docker Desktop 已可用） |
| arm64 运行 | 没有 ARM 硬件 | CI 的 QEMU 冒烟测试 |
| Cloudflare 真实路径 | 没有真实 Cloudflare | 本地 HTTPS 代理模拟；文档明说没有真测过 |
| 真实 iPhone | 没有设备 | Chrome 移动模拟；文档注明最低 iOS 15.4 |

## 10. 每个阶段"完成"的定义

| 阶段 | 完成 = |
|---|---|
| 1 基线 | 黄金响应、夹具、id 向量提交；同一套工具对当前旧版重跑，**0 个未解释的差异** |
| 2 Rust 后端 | `cargo fmt --check`、`cargo clippy -D warnings`、`cargo test` 通过；对黄金响应的差分只剩白名单；50 个夹具的解析差分一致；85 条 id 向量全过；并发与流式限制测试通过；在**副本**上双向读写 + `integrity_check` |
| 3 前端 | `tsc` 与 `vite build` 通过；UI 流程测试全过；DOM 与 CSS 类名与旧版一致 |
| 4 Nginx + Docker | `docker compose up` 起来，第 7 节的 9 项全过；后端不可直连 |
| 5 回归 | 新栈对黄金响应的差分只剩白名单；安全探针与旧版同一结果（有意加固除外）；隧道模拟两种场景通过；文档齐全 |

## 11. CI

- `check`：`VERSION` 与 README 更新日志一致，并校验 `Cargo.toml`、`package.json` 的版本与之相等。
- `backend`：fmt、clippy、test、audit；缓存 `~/.cargo` 与 `target`。
- `frontend`：`npm ci`、`tsc`、`vite build`、UI 流程测试。
- `compose-test`：起栈，跑集成与 Docker 测试。
- `images`：两个镜像（web、backend）× 两种架构；arm64 优先交叉编译。
- `publish` 与 `release`：只在 `main`；同现在的流程。
