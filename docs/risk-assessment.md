# 风险评估（Phase 0）

> 与 [architecture-audit.md](architecture-audit.md)（现状）、[migration-plan.md](migration-plan.md)（计划）、[test-strategy.md](test-strategy.md)（怎么验证）配套。
> 证据在 [`docs/audit/`](audit/) 的附录里；"检测"一列指向能发现该风险的具体测试。

## 1. 评级方法

- **可能性**：高 = 几乎肯定会发生；中 = 有路径会碰到；低 = 需要异常输入或环境。
- **影响**：高 = 数据丢失、安全漏洞、核心流程坏；中 = 用户会遇到的错误行为；低 = 体验瑕疵。
- **严重度** = 两者综合（高/中/低）。

## 2. 风险登记表

| # | 类别 | 风险 | 可能性 | 影响 | 严重度 | 缓解 | 检测 | 阶段 |
|---|---|---|---|---|---|---|---|---|
| R1 | 数据 | **题目 id 与旧版不一致**。学习记录和错题本以 id 为键；id 一变，历史就"脱钩"（旧题被下架、新题没有记录）。陷阱：Python 的 `\s` 是 29 个码点（含 U+001C–U+001F），Rust 的 `is_whitespace` 只有 25 个；必须按**码点**而不是字节截断 300/200；不能加 NFKC（业主真实题库大部分含全角 ASCII） | 高 | 高 | **高** | 自写 `is_py_space`；按码点切；85 条测试向量逐条断言；用旧解析器和 Rust 解析器分别解析同一批合成工作簿，id 逐项比较 | `tests/baseline/golden/id_vectors.json`、解析器差分测试 | 2 |
| R2 | 数据 | **解析器对等性**：Python `re` 与 Rust `regex` 的差异（`\d` 含全角数字、不区分大小写的 Unicode 规则、`xlsx_parser.py:216` 的前后断言 Rust 不支持）；`splitlines`/`strip` 的 Unicode 集合；expat 把字面 CRLF 规范化成 LF（业主真实题库依赖它）而 quick-xml 是否同样处理没验证 | 高 | 高 | **高** | 18 个正则里只有 1 个需要手写；其余逐个对照；用 50 个夹具做差分；CRLF 单独做一个夹具 | `tests/baseline/parser_golden/` | 2 |
| R3 | 数据 | **导入的原子性**：确认时必须一个事务；预览绝不写库；令牌一次性；并发确认只能成功一次 | 中 | 高 | **高** | `BEGIN IMMEDIATE`；令牌用 `Mutex<Option<_>>` 一次取出；并发测试（50 个线程抢同一个令牌，只能 1 个赢） | 并发测试；基线 #116–#133 | 2 |
| R4 | 数据 | 新后端写过的库，旧 Python 读不了（回滚失效） | 低 | 高 | **中** | 不改 schema、不改 `journal_mode`、不加表列、不设 `user_version`；每次写完让旧代码读一遍 | 双向读写测试 + `PRAGMA integrity_check` | 2、5 |
| R5 | 数据 | 测试误伤业主真实数据库 | 低 | 高 | **高** | 所有测试只用临时目录里的全新库或 `integrity_check` 过的副本；测试 compose 不挂载 `xexam_data`；**不替业主执行"切到真实卷"** | 流程约束；`git status` | 全程 |
| R6 | 兼容 | **JSON 字节格式**：`smoke-test.sh` 用 `grep '"bankCount": 0'`（冒号后有空格），serde_json 的紧凑输出会让它失败；键顺序也要一致 | 高 | 中 | **中** | 自写与 Python 默认一致的格式化器，字段按旧顺序；或把冒烟测试改成语义比较 | `diff_golden.py` | 2 |
| R7 | 兼容 | **百分比舍入**：库里 `round(100*good/total)` 是银行家舍入（`1/8 → 12`），Rust 的 `f64::round` 给 13；`2600` 个平局里有 `1300` 个会不同 | 高 | 低 | **中** | 用纯整数的"半偶数"公式（已在 200 万个组合上与 Python 对照 0 差异）；前端 v0.2 已改用同一规则 | 单元测试 | 2 |
| R8 | 兼容 | 不支持的方法：旧版给 501（标准库 HTML 页），axum 给 405；`//api/info` 旧版折叠成 200 | 高 | 低 | 低 | 登记为**有意差异**（405 + `Allow`）并进白名单 | `allowed_differences` | 2 |
| R9 | 兼容 | HTTP/1.0 无 keep-alive → HTTP/1.1；Nginx 的 413 页是 HTML，前端只会显示"HTTP 413"，看不到"文件太大" | 高 | 低 | 低 | Nginx `error_page 413` 返回 JSON，文案与后端一致 | 集成测试 + 真实 Chrome | 4、5 |
| R10 | 兼容 | 前端模块化后，业主的截图/验证脚本依赖的全局（`S`、`render`、`openAdmin`…）消失 | 高 | 低 | 低 | 保持 DOM 与 CSS 类名不变；验证工具改成"点击驱动"，放进 `tests/`/`tools/` | 前端 UI 流程测试 | 3 |
| R11 | 安全 | **XML 实体检查可被绕过**（UTF-16 的 DOCTYPE、非 `.xml` 扩展名、大写 `.XML`）；目前只靠 expat 的放大防护兜底 | 中 | 中 | **中** | Rust 里在 XML **事件层**拒绝任何 DOCTYPE（不依赖字节扫描）；拒绝 UTF-16；登记为有意加固 | 恶意夹具 `f16*` `f26*` | 2 |
| R12 | 安全 | **ZIP 炸弹 / 内存**：旧版按声明大小检查，实际可整读 100 MiB 的成员；解析无行数、单元格、深度上限 | 中 | 中 | **中** | `take(limit)` 有界读取；拒绝重复条目名；限条目数、元素深度、单元格数；协作式的解析超时（30 秒） | 恶意 ZIP 集合（S3）；RSS 有界断言 | 2 |
| R13 | 安全 | **代理后的 Host/Origin**：Nginx 默认把 Host 改成 `$proxy_host`（如 `backend:8765`），后端会 403 一切；若用 `X-Forwarded-Host` 做 Host 判断就等于把守卫交给了客户端 | 高 | 高 | **高** | `proxy_set_header Host $host`；清掉 `X-Forwarded-*`；后端**只读真实 `Host` 头**；永远不用 axum-extra 的 `Host` 提取器；Nginx 对未知 Host 返回 444 | `security_probe.py` 的 `hosts` / `anyhost` 配置；隧道模拟 | 4、5 |
| R14 | 安全 | 健康检查被自己的 Host 守卫拒绝 | 中 | 中 | 中 | 后端提供 `healthcheck` 子命令，自己带 `Host: 127.0.0.1` 访问自己；Nginx 的健康检查走 `localhost` | 容器 `healthy` 断言 | 4 |
| R15 | 安全 | Slowloris：旧版没有读超时；`axum::serve` 默认也不给 hyper 装计时器 | 中 | 低 | 低 | Nginx 的 `client_header_timeout`/`client_body_timeout`；后端可选再加 | 慢速客户端脚本（S11） | 4 |
| R16 | 并发 | rusqlite 是同步的，直接放进 async 处理器会阻塞执行器线程 | 高 | 中 | **中** | `Arc<Mutex<Connection>>` + `spawn_blocking`，锁不跨 `.await`；与旧版"一个连接 + RLock"一一对应 | 并发测试 | 2 |
| R17 | 并发 | `with_graceful_shutdown` 没有超时；进行中的导入被 SIGKILL 会留下热日志（SQLite 下次打开会回滚，是原子的） | 中 | 中 | 中 | 8 秒超时包装；导入提交作为独立任务，客户端断开也跑完；`stop_grace_period ≥ 10 s` | `docker stop` 计时（S7） | 4、5 |
| R18 | 构建 | **arm64 的 Rust 构建在 QEMU 下极慢**（CI 现在用 QEMU 跑 arm64） | 高 | 中 | **中** | 优先交叉编译（`cargo-zigbuild` / musl 目标）；rusqlite 的 `bundled` 需要 C 编译器，交叉时要验证 | CI 耗时实测（S9） | 4 |
| R19 | 构建 | 私有仓库 + 免费套餐能否用 GitHub 托管的 arm64 runner 没有确认 | 中 | 中 | 中 | 查官方文档；不行就用交叉编译 | 文档核实 | 4 |
| R20 | 构建 | **GHCR 写权限**：包存在之后 `GITHUB_TOKEN` 被拒（v0.2.2 的 `publish` 实测失败，v0.2、v0.2.1 成功） | 已发生 | 中 | **中** | 在包设置里给仓库 Actions **Write**（业主操作）；新镜像用新名字也会遇到同样的第一次创建流程 | 流水线 | 4、5 |
| R21 | 工具 | 本机没有 Rust、Nginx；Docker 守护进程曾经未运行 | — | — | 已缓解 | 已用 scoop 装好 Rust 1.99.1 + gcc 16.2；Docker Desktop 已启动；Nginx 用官方镜像跑 | 工具链冒烟（已做） | 0 |
| R22 | 时间 | 镜像里没有 tzdata 时，`TZ=Asia/Tokyo` 静默失效，时间戳变 UTC | 中 | 低 | 低 | 用 `jiff`（内置时区库）或带 tzdata 的基础镜像；三种基础镜像各输出一次比对 | S8 | 2、4 |
| R23 | 流程 | 推到 `main` 会触发 `publish`，移动 `latest` | 高 | 中 | **中** | 重构在分支 `modernize/rust-ts` 上做；新镜像用新名字（见 [migration-plan.md](migration-plan.md)） | 流水线条件 `ref == main` | 全程 |
| R24 | 流程 | `VERSION` 是 CI 的唯一版本来源；`Cargo.toml`、`package.json` 又各有版本 | 中 | 低 | 低 | `VERSION` 仍是唯一来源，CI 额外校验两个清单与它一致 | `check` 作业 | 5 |
| R25 | 安全/易用 | 旧版数据库文件名变更（v0.1 → v0.2）：旧文件被忽略，看起来像数据丢了 | 中 | 中 | 中 | v0.2 已加启动提醒与升级说明；Rust 版沿用同样的提醒 | `v02_fix_checks.py` | 已做 |

## 3. 必须保留的安全机制（回归测试的最低清单）

| 机制 | 旧版位置 | 验证 |
|---|---|---|
| 默认只接受本机 Host；白名单可配置 | `server.py _host_ok` | 基线 #040–#053；`host_origin_test.py`（55 项） |
| 写请求的 Origin 只比主机名；`XEXAM_ALLOWED_ORIGINS` | `server.py _origin_ok` | 基线 #054–#069；隧道模拟两种场景 |
| `/api/shutdown` 默认关闭 | `server.py` | 基线 #095、#170–#172 |
| 上传魔数 / ZIP / 解压大小 / 20 MB | `check_xlsx`、`_body` | 基线 #096–#111；夹具 `f21*` |
| 令牌 128 位、常量时间比较、一次性、30 分钟 | `Pending` | 基线 #116–#133；进程内过期测试 |
| 导入在一个事务里 | `Store.tx` | 回滚夹具；并发测试 |
| SQL 全参数化 | `db.py` | 代码检索：除 `IN (?,?,…)` 的占位符拼接外无字符串格式化 |
| `nosniff`、`no-store` | `_send` | 基线的响应头比较 |
| 非 root 运行 | Dockerfile | 冒烟测试 `id -u` |
| 学习记录不因下架而丢 | `apply_import` | 基线 #159–#164 |

## 4. 现在就存在的安全缺口（不是回归）

规格要求"评审"这些项；旧版**没有**它们。默认做法是**列入文档，并按下表决定是否新增**，不假装它们存在。

| 项 | 旧版 | 默认 |
|---|---|---|
| CORS | 无（同源，无 OPTIONS 处理） | 不发 CORS 头 |
| 速率限制 | 无 | Nginx 宽松限流：`/api/` 10 r/s（突发 30），上传 1 r/s（突发 5） |
| 认证 | 无（靠 Cloudflare Access） | 不引入；模型不变 |
| 解析时间上限 | 无 | 30 秒（`XEXAM_PARSE_TIMEOUT_SECS`） |
| 访问日志 | 无 | Nginx 记；后端用 `tracing`，**不记令牌和文档内容** |
| 临时文件清理 | 不适用（全在内存） | 保持全内存 |
| 按用户隔离待确认导入 | 无用户概念，全局一个槽位 | 保持单槽位；非 ASCII 令牌改成 409 |
| 服务端 `.xlsx` 扩展名检查 | 无（只有前端检查；服务端按内容判断） | 保持按内容判断，文档注明 |
| 安全头 | 只有 `nosniff` / `no-store` | Nginx 加 `X-Frame-Options: DENY`、`Referrer-Policy: no-referrer`、严格 CSP；不设 HSTS（TLS 在 Cloudflare 终止） |

## 5. 数据丢失情景与对策

| 情景 | 对策 |
|---|---|
| 新后端把数据库写坏 | 不改 schema/journal；写测试只在副本上；每次写完 `integrity_check`；回滚 = 停新栈，启动旧 compose 指向同一个卷 |
| 题目 id 变了，历史脱钩 | R1 的 85 条向量；切换前对业主**库副本**里全部现有题目重算 id 并比对 |
| 重构误删旧实现 | 打标签 `python-v0.2`（已在 GitHub 上）、建分支 `legacy/python`；之后只 `git mv` 到 `legacy/python/`，不 `rm` |
| 旧文件名数据库被忽略 | 启动提醒；升级说明 |
| `docker compose down -v` 把卷删了 | README 与使用说明里的醒目警告（已有） |

## 6. 无法在本机验证的事，以及怎么办

| 事 | 原因 | 办法 |
|---|---|---|
| arm64 镜像的真实运行 | 没有 ARM 硬件 | CI 里 QEMU 冒烟测试（v0.2.x 已在跑） |
| Cloudflare 隧道 + Access | 没有真实 Cloudflare | 本地 HTTPS 代理 + 真实 Chrome 模拟（已有两种场景）；文档明说"没有用真实 Cloudflare 测过" |
| 真实 iPhone / Safari | 没有设备 | 用 Chrome 的移动模拟 + 代码审阅；文档注明最低 iOS 15.4 |
| 业主的真实数据库 | 不在这台机器 | 用合成库；真实库的副本到手后再做一次差分，**不在未验证前切换** |

## 7. 开始实现与切换的门槛

**开始 Phase 2（Rust 后端）前**：基线已提交（✅ v0.2.2）；Rust 工具链已验证（✅）；本文和 [migration-plan.md](migration-plan.md) 已写。

**把新栈指向业主的真实数据之前（这一步不由我执行）**：
1. 业主停掉旧容器，按 README 的"备份和恢复"备份卷，并在备份上跑 `PRAGMA integrity_check`。
2. 在备份的**副本**上跑新后端，对 `/api/bank`、`/api/progress`、`/api/imports` 与旧版逐项比较，**零未解释差异**。
3. 全部回归测试（[test-strategy.md](test-strategy.md) §10 的"完成的定义"）通过。
4. 业主确认切换；回滚命令写在 [migration-plan.md](migration-plan.md) §8。
