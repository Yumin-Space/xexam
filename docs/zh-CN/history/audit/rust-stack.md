# Xexam Phase 0 草稿：Rust 技术栈核查与设计约束（维度 rust）

> 日期 2026-11-10。只读审计，未创建任何 Rust 项目，未改动仓库、`data/`、`progress.json`。
> 图例：**[事实]** = 读了代码 / 跑了命令 / 查了注册表；**[规格]** = 业主规格的要求；**[建议]** = 我的建议；**[未证实]** = 没有验证，需要 spike。
> 命名：本文不出现旧名称。规格里数据库文件写作 `<旧名>.db`，**实际文件是 `xexam.db`（server.py:380）——这是规格与现实的差异，已按现实处理**。

## 0. 依据文件与 md5（读取时）

| 文件 | md5 |
|---|---|
| server.py（477 行） | 48306e1f716d5dadeeedb63e805cebae |
| app/db.py（308 行） | 02846cbc21b21da42bb6f680f8638792 |
| app/xlsx_parser.py（410 行） | 239b594ef8cbba5206e9a6f116178afe |
| scripts/smoke-test.sh | c242b5deecc79fc23b728e4cf31ac2f1 |
| Dockerfile / docker-compose.yml | b031d35fad7d2bae6c2777be520e0399 / 160023431bbe53035860a022124df9ae |
| web/app.js | 0dd0e1139626fe020f483ff03a02de07 |
| 外部 host_test.py / tunnel_sim.py / mm_check.py | f51d5384961c83263aad9934c5a7b347 / 938d73e00ccd592a3af9910548728009 / ecd42d63c1048a1b480001d1cb8a0d9c |
| xexam_shots/files/demo.xlsx（虚构演示题库） | 0771c043b0433eee89438445513e88f3 |

收尾前重新计算过一次 server.py / db.py / xlsx_parser.py / smoke-test.sh / Dockerfile / docker-compose.yml / app.js 的 md5，与上表一致。VERSION 为 `0.2`。

我写出的产物（全部在 `xexam_phase0/rust/` 内）：
- `work/legacy_probe.py` + `legacy_probe.out`：不开端口，用假 socket 把真实 `Handler` 在进程内跑了 60 余个请求（只导入仓库代码，设了 `dont_write_bytecode`，没写 .pyc）。
- `work/semantics_probe.py`、`work/edge_probe.py`：Python 语义实验（空白集合、`\d`、`make_id`、`canon`、`round`、JSON 宽松度、ET 行为）。
- `work/flow_golden.py` → `golden/`：用**虚构的 demo.xlsx / demo_v2.xlsx** 在旧实现上跑完整流程得到的黄金文件（parse_*.json、preview_*.json、confirm_result_*.json、attempt_result.json、imports.json、progress_after_reset.json、info.json），另有 `percent_tie_cases.csv`（2600 个舍入平局）和 `nd_run_starts_unicode_13.0.0.txt`（Unicode 13 的 65 段十进制数字起点）。黄金文件含时间戳，比较时必须规范化 `at` / `mtime` 字段。
- `work/src/`：拉取下来的 axum / tower-http / hyper / quick-xml / roxmltree / zip 等源码与 changelog 片段（用作行号证据）。

---

## 1. 依赖核查表

数据来源：crates.io API（2026-11-10 抓取）、rustsec/advisory-db `main` 分支（GitHub contents API）、npm registry、static.rust-lang.org。**没有运行 `cargo audit`/`cargo deny`**（未安装，且规则不允许安装）。"advisory 目录不存在"只表示 RustSec 库里没有该 crate 的条目，不是安全保证。

| 组件 | 当前版本（发布日） | 许可证 | MSRV | 活跃度 | RustSec（本版本是否受影响） | 用于安全关键解析？ |
|---|---|---|---|---|---|---|
| **Rust stable** | 1.99.1（2026-10-15 点版本；channel manifest 日期 2026-10-15；cargo 0.100.0） | MIT/Apache | — | 6 周节奏 | 1.99.1 修复 x86_64 上 FFI `bool` 返回值误编译（RELEASES.md，issue 163911）→ **建议 ≥1.99.1**。edition 2024 可用（quick-xml/rusqlite 已用 2024） | — |
| axum | 0.8.9（2026-04-14） | MIT | 1.80 | 近 7 个月无新版，仍是 tokio-rs 主线 | 库中无条目 | 是 HTTP 层，不是解析器 |
| tokio | **1.54.1**（2026-10-21） | MIT | 1.85 | 极活跃 | **1.54.0 已被 yank**，RUSTSEC-2026-0384（`Interval` 数据竞争，≥1.54.1 修复）；RUSTSEC-2026-0386（`AsyncFd::new` 不健全，≥1.53.3 已修）。**必须写 `>=1.54.1`** | — |
| tower | 0.5.3（2026-01-12） | MIT | 1.64 | 稳定、低频 | 无条目 | — |
| tower-http | 0.7.1（2026-08-31） | MIT | 1.65 | 活跃（0.7.0/0.7.1 近期） | 仅 2022 年旧条目（`ServeDir`，已修）；不用 `fs` | — |
| hyper / hyper-util | 1.12.0（2026-10-06）/ 0.1.21（2026-09-24） | MIT | 1.63 / 1.85 | 活跃 | hyper 条目全是 0.x 旧版本 | HTTP 解析（Nginx 在前面） |
| serde / serde_json | 1.0.229 / 1.0.154 | MIT OR Apache-2.0 | 1.56 / 1.71 | 极成熟 | 无条目 | 是，成熟 |
| rusqlite | 0.41.1（2026-10-28） | MIT | **1.89** | 活跃 | 旧条目均已修（≥0.26.2） | 是（SQL 参数化） |
| libsqlite3-sys | 0.39.1（2026-10-25；内置 SQLite 3.54.1） | MIT | 1.88 | 活跃 | RUSTSEC-2026-0459（SQLite<3.54 的 WAL 页大小不一致泄露未初始化内存，≥0.39.0 已修）；本方案不用 WAL 也已规避 | — |
| tracing / tracing-subscriber | 0.1.44 / 0.3.23 | MIT | 1.65 | 稳定（tracing 上次发布 2025-12） | tracing-subscriber RUSTSEC-2025-0055（日志里 ANSI 转义注入，≥0.3.20 修复）→ **关闭 `ansi` 特性**并 ≥0.3.20 | — |
| thiserror / anyhow | 2.0.21 / 1.0.104 | MIT OR Apache-2.0 | 1.77 / 1.68 | 成熟 | anyhow RUSTSEC-2026-0190（`downcast_mut` 不健全，≥1.0.103 修复）→ 用 ≥1.0.103 | — |
| **zip** | **9.5.0（2026-11-09）** | MIT | 1.88 | **非常活跃，但变动极快**：9.0.0（2026-10-08）到 9.5.0 一个月 13 个版本 | 2026 年 5 条：RUSTSEC-2026-0367/0368/0369（`extract` 的 SUID/符号链接）、-0429（`extract` 遇到声明超大的符号链接条目会 `with_capacity` 后 abort）、2025-0168；**全部 ≥9.0.0 已修，且都与 `extract()` 有关，本方案不调用 `extract()`**。changelog 还显示 9.0.0 修了"按声明大小预分配"（#984/#991/#937），9.0.1 才加了"读取超过声明大小则报错"（#1022） | **勉强够用**：读路径被广泛使用，但版本变动快、2026 年连出 5 个公告。必须锁 `Cargo.lock`、CI 跑 cargo-audit，且**不信任 `file.size()` 做预分配** |
| **quick-xml** | 0.42.0（2026-08-22） | MIT | 1.86 | 活跃 | RUSTSEC-2026-0194（重复属性检查二次复杂度）、-0195（`NsReader` 命名空间声明无限分配），**均 ≥0.41.0 已修**；0.42.0 还把命名空间绑定总数默认上限设为 128、嵌套深度 u16 溢出改为报错（changelog #977/#980） | 流式、内存可控，但 API 在 0.38/0.42 两次大改（见 §2.10）；DOCTYPE 不会自动展开 |
| roxmltree | 0.21.2（2026-10-24） | MIT OR Apache-2.0 | 1.60 | 活跃 | 无条目 | **DOM 一次性载入**（内存是文件的数倍）；默认 `allow_dtd=false`（有 DTD 直接 `DtdDetected`，空 DTD 块除外）、内置 billion-laughs 防护（实体深度 10）、可设 `nodes_limit`；0 依赖、无 unsafe；仅 UTF-8 |
| xml-rs | 1.0.0（2026-08-06） | MIT | 1.70 | 低频 | 2022 年"unmaintained"条目已撤回（2023-05） | **未读其源码**，对 DOCTYPE/实体的处理未核实，不推荐 |
| regex | 1.13.1 | MIT OR Apache-2.0 | 1.65 | 极成熟 | 2022 旧条目已修 | 线性时间保证（无灾难回溯） |
| fancy-regex | 0.20.0 | MIT | 1.66 | 活跃 | 无条目 | 有回溯，**不建议**用在不可信输入；本项目不需要它（见 §3） |
| unicode-normalization | 0.1.26 | (MIT OR Apache-2.0) AND Unicode-3.0 | 1.36 | 稳定 | 无条目 | **本项目不需要**（旧实现从不做 NFC/NFD；已实测 NFC≠NFD 的 id 不同，见 §3） |
| sha1 | 0.11.0（2026-07-10） | MIT OR Apache-2.0 | — | RustRypto | 无条目 | 仅用于旧 id 指纹（非安全用途） |
| sha2 / md5 | 0.11.1 / 0.8.1 | — | 1.88 / — | — | sha2 仅 2021 旧条目 | **本项目均不需要** |
| getrandom | 0.4.3（2026-06-17） | MIT OR Apache-2.0 | 1.85 | 极成熟 | 无条目 | 直接取 OS CSPRNG：`getrandom::fill(&mut [u8])`（源码 lib.rs:87） |
| rand | 0.10.3 | MIT OR Apache-2.0 | 1.85 | 活跃 | RUSTSEC-2026-0097（自定义 logger 里调用 `rand::rng()` 不健全，0.10.3 已修） | **不需要**（令牌只要 `getrandom`） |
| subtle | 2.6.1（2024-06-24） | BSD-3-Clause | — | 稳定、几乎不再变 | 无条目 | 常量时间比较；备选 `constant_time_eq` 0.6.1（2026-10-01，CC0/MIT-0/Apache，更小）。令牌长度是公开的，两者都只在长度不同时提前返回，与 Python `compare_digest` 一致 |
| base64 | 0.23.1（2026-08-04） | MIT OR Apache-2.0 | 1.71 | 成熟 | 仅 2017 旧条目 | 令牌 base64url 编码；**0.23 的 API 我没看**（axum 的可选依赖仍是 ^0.22）。16 字节编码只要约 10 行，也可手写省掉依赖 |
| time / chrono / jiff | 0.3.55 / 0.4.45 / 0.2.41（2026-11-08） | MIT OR Apache-2.0 / 同 / Unlicense OR MIT | 1.88 / 1.62 / 1.70 | 均活跃 | time：RUSTSEC-2026-0009（≥0.3.47）；chrono：2020 旧条目 | 见 §2.8 时间戳 |
| utoipa / utoipa-axum | 6.0.0 / 0.3.0（均 2026-09-22） | MIT OR Apache-2.0 | 1.88 | 活跃 | 无条目 | utoipa-axum 0.3.0 依赖 `axum ^0.8.4`（default-features=false）、`utoipa ^6.0.0`，与 axum 0.8.9 兼容（crates.io 依赖接口已核对） |
| utoipa-swagger-ui | 10.0.2 | MIT OR Apache-2.0 | 1.88 | 活跃 | — | 构建期会下载 swagger-ui 资源，**不建议**纳入 |
| openapi-typescript（npm） | 7.13.0（2026-02-11；元数据修改 2026-06-15） | MIT | 无 engines | 活跃度中等 | — | **peerDependencies: typescript ^5.x**（npm registry）。前端如果选 TS 6+ 会冲突，装之前先核对 |
| tokio-util / bytes / http / mime 等传递依赖 | — | — | — | — | bytes RUSTSEC-2026-0007（≥1.11.1，当前 1.12.1 已修）；time（zip 默认特性会带）RUSTSEC-2026-0009 已修 | — |

### 1.1 兼容性约束（来自 crates.io 依赖接口）

- axum 0.8.9 → `axum-core ^0.5.5`、`hyper ^1.1`、`hyper-util ^0.1.3`（特性 tokio/server/service）、`tower ^0.5.2`、`tower-http ^0.6.8`（**仅可选**）、`matchit =0.8.4`、`tokio ^1.44`。**我们直接依赖 tower-http 0.7.1，它和 axum 内部可选的 0.6.8 不冲突**（axum 的 tower-http 是可选特性，默认不启用；但 `Router::layer` 接受任何 `tower::Layer`，类型上 tower-http 0.7 与 axum 0.8 都基于 `http 1.x`/`http-body 1.x`/`tower-layer 0.3`，可以混用；具体编译结果以协调者的 spike 为准——spike 已确认 axum 0.8.9 + tower-http 0.7.1 + tokio 1.54.1 同时可编译）。
- tokio 特性：`rt-multi-thread`、`macros`、`signal`、`net`、`time`、`sync`（`default = []`，所有特性必须显式声明；tokio 1.54.1 的特性表已核对）。优雅停机的 SIGTERM 需要 `signal`（仅 unix 有 `SignalKind::terminate`，Windows 上只有 `ctrl_c`）。
- MSRV 取最大值：**rusqlite 0.41.1 要 1.89**（libsqlite3-sys/zip/utoipa/sha2 要 1.88）。→ `rust-version = "1.89"`，`rust-toolchain.toml` 固定 `1.99.1`。
- zip 9.5.0 默认特性包含 aes-crypto/bzip2/lzma/ppmd/zstd/xz/time 等一长串（features 表已核对）：**必须 `default-features = false`**，只留解压 deflate。可选名 `deflate-flate2-zlib-rs`（纯 Rust）；协调者 spike 用的是 `deflate`，它会带上 `zopfli`（压缩器，读取用不到）。
- rusqlite 0.41.1 默认特性 `cache` + `ffi-sqlite-wasm-rs`：原生构建 `default-features = false, features = ["bundled"]`。
- tracing-subscriber 默认带 `ansi`：关掉（RUSTSEC-2025-0055）。

---

## 2. 必需行为 → Rust 方案 → 陷阱

### 2.1 请求体大小限制（独立于 Content-Length，流式）

**旧实现事实**
- 常量：`MAX_JSON = 1_000_000`（十进制）、`MAX_UPLOAD = 20*1024*1024`（20 MiB，server.py:48-49）。Nginx 的 `20m` 也是 20 MiB（1024×1024），两者一致；判断是 `>`，恰好 20 MiB 通过、多 1 字节 413（server.py:281；实测见下）。
- `_body()`（server.py:272-290）：只读 `Content-Length`；解析失败按 0；负数返回 None（→ 413）；`n > limit` 时先**排空** `min(n, limit*4)` 字节再关连接并 413。**不理解 `Transfer-Encoding: chunked`**：实测（进程内探针）分块上传 demo.xlsx → 旧服务把正文当空 → 400"文件是空的，或者没有上传完整"。也就是说旧服务从不接受分块上传。
- 超限响应：JSON 正文 413 `{"error":"too large"}`；上传是 413 `{"error":"文件太大了（最大 20 MB）。…"}`（server.py:321）。
- 实测（探针）：`CL=1000001` JSON → 413；`CL=-5` → 413；`CL=abc` → 当 0，`{}` 有效；上传 `CL=20 MiB+1` → 413；`CL=20 MiB` 但正文很短 → 400"文件已损坏"。
- 前端对 413 的处理：`!r.ok` 时 `d.error || '上传失败（HTTP '+r.status+'）。'`，且只在 `Content-Type` 含 json 时解析（web/app.js:448-457）。

**Rust 方案与陷阱（验证过的源码事实）**
- `DefaultBodyLimit`（axum-core default_body_limit.rs:4-12）：默认 2 MB，**只对 `Bytes/String/Json/Form` 这类缓冲提取器生效**；自己用 `poll_frame` 读 body 时**不生效**。→ 单靠它不够。
- `RequestBodyLimitLayer`（tower-http 0.7.1 limit/service.rs:36-48）：先看 `Content-Length`，`> limit` 立即返回固定的 413（正文是 crate 内置常量，**不是我们的 JSON**）；否则把 body 包成 `Limited(min(limit, len))`，分块/超长时在读取过程中报错。axum 把 `LengthLimitError` 映射成 413 "Failed to buffer the request body"（axum-core rejection.rs:41-48），无法识别的错误映射成 400。→ 想要旧版的 JSON 413 和中文提示，必须自己控制响应。
- **[建议]** 在 `security::body` 里写一个 `read_limited(body, limit) -> Result<Bytes, BodyError>`：先用 Content-Length 快速拒绝，再用 `http_body_util::BodyExt::frame()` 循环累加、超限即停；上传限 `20*1024*1024`，JSON 限 `1_000_000`；`>` 而不是 `>=`。不依赖任何层的默认 413。可再叠一个全局 `RequestBodyLimitLayer`（比如 21 MiB）作为兜底。
- **排空行为**：hyper 在响应发出后若 body 没读完，只尝试读取"已在缓冲里"的部分，仍在 `Reading::Body` 就 `close_read()`（hyper `proto/h1/conn.rs:875-891`）。所以后端直连时，客户端可能在读到 413 前看到连接被重置——这正是旧版排空逻辑存在的原因（server.py:273-275 的注释）。**[建议]** 在 413 分支里照旧版排空最多 `4×limit` 再回复；在 Nginx 之后则由 Nginx 的 `client_max_body_size` + lingering close 负责。
- **Nginx 的 413 是 HTML**（不是 JSON）→ 前端会退化成"上传失败（HTTP 413）"，丢掉中文提示（app.js:453-455）。**[建议]** Nginx 里 `error_page 413 = @too_large;` 返回 `application/json` 的同一句中文。
- 分块请求：Nginx 默认 `proxy_request_buffering on`，会读完整个请求再带 `Content-Length` 发给上游，所以后端通常看不到分块体；但后端必须自己防（规格要求）。Rust/hyper 原生接受分块体，**行为比旧版宽松**，需要在文档里记为"有意差异：受限接受"。
- 声明长度大于实际：hyper 报 body 提前结束 → 处理器应返回 400（旧版是 400"已损坏"），措辞不必逐字一致。
- 上传并发：旧版 `ThreadingHTTPServer` 每连接一个线程，内存无上限。**[建议]** `/api/upload` 用 `tokio::sync::Semaphore`（2 个许可）限并发，避免 N×20 MiB。
- 请求头：hyper 默认最多 100 个头（role.rs:31）、读缓冲上限约 8192+4096×100 字节（io.rs:23）；Python 请求行上限 65536。无需调整。
- **Slowloris**：hyper 的 `header_read_timeout` 默认 30 s，但**需要显式设置 Timer 才生效**（hyper http1.rs:360-372、time.rs `check`：没有 timer 时只 `warn!` 并忽略）；而 `axum::serve`（axum serve/mod.rs，全文无 `timer`）**没有设置 timer**。→ 直接用 `axum::serve` 时没有请求头读取超时。**[建议]** 要么靠 Nginx（`client_header_timeout` 等），要么自己用 `hyper_util::server::conn::auto::Builder` + `TokioTimer` 写 accept 循环（hyper-util `auto/mod.rs:829,874` 有 `header_read_timeout`/`timer`）。tower-http 0.7.0 新增 `RequestBodyDeadlineLayer`（不重置的总体 body 期限）和 `TimeoutLayer::with_status_code`（`new()` 自 0.6.7 起已弃用）。

### 2.2 方法、HEAD、404/405、路由形态

**旧实现事实（进程内探针，`legacy_probe.out`）**

| 请求 | 旧响应 |
|---|---|
| `PUT/DELETE/PATCH/OPTIONS/TRACE/FOO /api/bank` | **501**，`text/html`，标准库自带的 HTML 错误页，`Connection: close`。**没有 Host/Origin 检查**（没有 `do_PUT`，直接走 `send_error`） |
| `GET /api/nope`、`GET /api/info/`、`GET /API/INFO`、`GET /api/upload` | **404 `text/plain` "not found"**（GET 未知路径会先当静态文件找，找不到 404） |
| `POST /api/nope` + JSON | 404 `{"error": "not found"}` |
| `POST /api/nope` + `text/plain` | **415** `{"error": "bad content type"}`（内容类型检查在路由之前） |
| `POST /api/bank`（GET-only 路由） | 404 JSON（不是 405） |
| `GET /api/info?x=1` | 200（查询串被忽略）；路径区分大小写、末尾斜杠 404 |
| `HEAD /api/info` | 200，头与 GET 完全一样（含 `Content-Length: 73`），正文为空（server.py:236-242）；`HEAD` 走的也是 GET 的全部检查 |
| 无 `Host` 头 / 空 `Host` | 403（`host_name('')` 不在白名单） |
| 响应协议 | **`HTTP/1.0`**，每次一个连接；`Server: Xexam/0.2 Python/3.10.11`（泄露语言和版本） |
| 响应头 | 仅 `Content-Type`、`Content-Length`、`Cache-Control: no-store`、`X-Content-Type-Options: nosniff`（server.py:200-208） |
| 无 CORS | 没有任何 CORS 头，也不处理 OPTIONS |

**Rust 对应与陷阱**
- axum 默认：已知路径用错方法 → **405 + `Allow`**（method_routing.rs:754 `Ok(StatusCode::METHOD_NOT_ALLOWED)`）；未知路径 → 404 空正文；`get()` 路由自动应答 HEAD 并去掉正文（method_routing.rs:55、1157-1158）。
- 想逐项保持旧形态：`Router::fallback()`（GET/HEAD → 404 `text/plain` "not found"，POST → 404 JSON）+ `Router::method_not_allowed_fallback()`（axum routing/mod.rs:374）返回旧的 404。HEAD 的 `Content-Length` 是否保留 axum 去正文后的值：**[未证实]**，spike S6 验证。
- 其他方法（501）：**[建议]** 保持 501 或明确改成 405，写进兼容性差异表；前端不会发这些方法。Nginx 可用 `limit_except GET HEAD POST { deny all; }` 在更前面拦掉。
- **不要使用 axum 的 `Json` 提取器读请求**：它对缺少 Content-Type 返回 415、语法错误返回 400/422，文本和旧版不同；旧版的规则是"`'application/json' in ctype`"的**子串匹配**（server.py:334），已实测 `text/plain; x=application/json` 也被接受。**[建议]** 为了行为保持先照抄子串规则，并把它记入"技术债/安全加固候选"。
- **不要使用 axum-extra 的 `Host` 提取器**：据我对其文档的记忆，它会优先读 `Forwarded`/`X-Forwarded-Host`——与"不盲信 X-Forwarded-*"相冲突，旧版只读 `Host` 头（server.py:219）。**[未证实]**（没有查证源码）。直接 `headers.get(HOST)`。
- Nginx 的 `proxy_set_header Host`：默认值是 `$proxy_host`（上游名，如 `backend:8765`），会让旧的 Host 白名单对**所有请求返回 403**（含健康检查）。必须 `proxy_set_header Host $host;` 并把公网域名写入 `XEXAM_ALLOWED_HOSTS`；`Origin` 头默认原样透传。不要转发/信任 `X-Forwarded-*`，旧版本来也不读它们。
- 旧版 `urlparse(self.path).path` 对 `//x/api/info` 这类以 `//` 开头的路径会把 `x` 当 netloc、取 `/api/info`（`urllib` 文档行为，**未实际运行**）。Rust 会把它当 `//x/api/info` → 404。无害，记入差异表。

### 2.3 JSON 输出差异

- 旧版：`json.dumps(obj, ensure_ascii=False)`（server.py:211）。**实测输出带空格**：`{"version": "0.2", "arch": "AMD64", "canShutdown": false, "bankCount": 0}`（分隔符是 `", "` 和 `": "`）。`serde_json` 默认紧凑输出 `{"version":"0.2",...}`。
  - **关键依赖**：`scripts/smoke-test.sh:49-51` 用 `grep -q '"bankCount": 0'` 和 `grep -q "\"version\": \"$want\""`——紧凑输出会让冒烟测试失败。
  - **[建议]** 写一个约 20 行的 `serde_json::ser::Formatter`（`PyCompactFormatter`），在对象键值之间输出 `": "`，元素之间输出 `", "`——这样字节级兼容，黄金文件可以逐字节比较，冒烟测试不用改。备选：改冒烟测试用 `jq`/语义比较。
- 键顺序：Python `dict` 按插入顺序。`serde_json::Map` 默认 BTreeMap（按字母序），`json!` 宏会因此**改变键顺序**（例如 `/api/info` 会变成 arch, bankCount, canShutdown, version）。**[建议]** 响应一律用 `#[derive(Serialize)]` 结构体并**按旧顺序声明字段**；同时开启 `serde_json` 的 `preserve_order`（IndexMap；其 `PartialEq` 与顺序无关，不影响"相等比较"）。
- `/api/bank` 的题目键顺序：`data` 在入库时写成 `kind, tag, stem, options, items, answer, translation, zh, explanation, mnemonic, note`，读取时再**追加** `id, sheet, row`（db.py:143-147）。结构体按这个顺序声明；`zh` 为 `null` 时必须输出 `null`（不是省略）。
- 转义：Python `ensure_ascii=False` 只转义 `"`、`\`、控制字符（<0x20，用 `\n \r \t \b \f` 或小写 `\u00xx`），**不转义** DEL(0x7f)、U+2028/2029。`serde_json` 的 ESCAPE 表（ser.rs:2147-2165）完全一致，十六进制也是小写（ser.rs:1786）。
- 数字：只有整数（题号、计数、`percent`），无浮点，格式无差异。
- 请求端 JSON 宽松度（Python `json.loads` 实测，`semantics_probe.out`）：接受 `NaN/Infinity`、超大整数、重复键（后者覆盖）、**UTF-8 BOM、UTF-16 编码**、顶层标量（之后被 `isinstance(dict)` 拦成 400）、空正文当 `{}`；拒绝尾逗号、原始控制字符。`serde_json` 拒绝 NaN/Infinity/BOM/UTF-16/孤立代理项。→ **[建议]** 先把正文解析成 `serde_json::Value`，再手工移植 `clean_attempt`（宽松转换），不要用带 `deny_unknown_fields` 的结构体直接反序列化。深层嵌套 `[[[[…`：Python 抛 `RecursionError`（**不是** ValueError，不会被 `except ValueError` 捕获）；`serde_json` 默认递归深度 128 → 干净报错。

### 2.4 错误形态

- 旧版错误体分三种：JSON `{"error": "…"}`（POST 路径）、`text/plain`（GET 的 403/404）、标准库 HTML（501）。状态码全集：200/400/403/404/409/413/415/500/501。
- **旧版会崩的路径（实测）**：`/api/attempt` 中 `seconds=1e999` → `OverflowError`；`results` 不是列表（如 `{"x":1}`、`5`）→ `TypeError: unhashable type: 'slice'`；`/api/import/confirm` 的 `token` 含非 ASCII → `secrets.compare_digest` 抛 `TypeError`。进程内探针里处理器直接抛异常，**0 字节写出**；在真实服务器里 `socketserver.handle_error` 会向 stderr 打印回溯并断开连接（标准库行为，**未做实机验证**）。→ Rust 应返回 400（或把非法值当 0），记为"有意差异：旧版会断连"。
- 信息泄露（旧版）：解析失败时 `'读取这个 Excel 时出错：%s: %s' % (type(e).__name__, e)`（server.py:329）把 Python 异常类名和消息给了用户；`apply_import` 失败时把 sqlite 异常文本拼进 500（server.py:355）。→ Rust：保留中文前缀，细节只进日志。
- 所有中文提示文案是 API 的一部分（前端直接显示 `d.error`）；移植时**逐字复制**。例外：XML 解析错误里的 expat 位置信息无法复现，测试只比较状态码 + `error` 存在。
- 实现建议：`AppError` 枚举 + `IntoResponse`，区分"GET 的 403 是 text/plain、POST 的 403 是 JSON"（server.py:245-248 vs 308-311）。`tower_http::catch_panic::CatchPanicLayer` 把 panic 变成通用 500 JSON。

### 2.5 优雅停机（SIGTERM/SIGINT，进行中的导入）

**旧实现**：`signal.signal` 收到 SIGTERM/SIGINT 后在线程里 `httpd.shutdown()`，`finally` 里 `server_close(); store.close()`（server.py:456-473）。`ThreadingHTTPServer.daemon_threads = True`（实测），所以**不会等待处理中的请求线程**；但 `store.close()` 要拿 `self.lock`（db.py:124-126），因此**正在进行的事务会先完成**再关闭连接。`docker stop` 在冒烟测试里要求 <9 s（smoke-test.sh 末尾）。

**Rust 方案**
- `axum::serve(listener, app).with_graceful_shutdown(signal)`：收到信号后停止 accept，对每个连接调用 `graceful_shutdown()`，然后 `close_tx.closed().await` 等所有连接任务结束（axum serve/mod.rs:257-306、380-415）。**没有内置超时**——一个挂着的请求会让关停无限等待，Docker 10 秒后 SIGKILL。→ 外层包 `tokio::time::timeout(Duration::from_secs(8), serve)`（上限小于 compose 的 `stop_grace_period` 默认 10 s）。官方例子也加了 `TimeoutLayer`（axum examples/graceful-shutdown）。
- 信号：`tokio::signal::ctrl_c()` + unix 下 `signal(SignalKind::terminate())`；Windows 下 `terminate` 用 `pending()`。
- 进行中的导入：`spawn_blocking` 的任务**无法取消**（tokio 文档行为；超时层只是丢弃 future，任务继续跑）。这对事务是好事（要么提交要么回滚，不会半途中断），但 `Pending.take` 已经消耗令牌，若客户端掉线则前端看不到结果，重试会得到 409（旧版同样）。**[建议]** 把"取令牌 + 应用导入"放进 `tokio::spawn` 的独立任务里再 `await` 其 `JoinHandle`，这样请求 future 被丢弃时提交仍会完成。
- 停机顺序：服务停止 → 对 `Mutex<Connection>` 拿锁（等待进行中的事务）→ `conn.close()`（rusqlite `close()`，lib.rs:820）。这与旧版"`store.close()` 先拿锁"同构。
- **[未证实]** `#[tokio::main]` 的运行时在 drop 时是否无限期等待阻塞任务：我没有查文档，上述"显式拿锁再关"的做法不依赖这个行为。
- 保持 `POST /api/shutdown`（源码里有，规格接口清单里没有）：未加 `--allow-shutdown` 时返回 403 `{"error":"在 Docker 里请用 docker compose stop 停止容器。"}`（server.py:367-369）。

### 2.6 阻塞 SQLite 的策略

**旧实现保证（读代码）**：单个 `sqlite3.Connection`（`check_same_thread=False, isolation_level=None`，db.py:89）+ 一把 `threading.RLock`。每个公开方法都拿锁；`tx()` 在**整个事务期间**持有锁，使用 `BEGIN IMMEDIATE`，异常时 `ROLLBACK`（db.py:100-110）。因此：(a) 同一时刻只有一个 DB 操作；(b) 写事务原子；(c) 读不会看到半个事务；(d) 注意 `bank()` 里 `SELECT` 和 `last_import()` 是**两次**拿锁（db.py:136, 150），之间可被其它写入插队——极小的不一致窗口，Rust 可以改成一次锁内完成（无可见差异，只更严格）。`apply_import` 先 `plan_import`（一次 SELECT）再开事务，两者之间可被插队，因此 Rust 应在**同一事务内**先算计划再写，返回值更准确。

**三种方案对比**

| 方案 | 与旧保证的对应 | 优点 | 缺点 / 陷阱 |
|---|---|---|---|
| A. `Arc<std::sync::Mutex<Connection>>` + `spawn_blocking` | **一一对应**（锁 = RLock） | 最简单；`Connection: Send` 所以 `Mutex<Connection>: Sync`；事务期间天然持锁 | 锁**绝不能跨 `.await`**；`spawn_blocking` 线程池默认上限 512，单人使用不会耗尽；panic 会毒化 Mutex → 用 `lock().unwrap_or_else(PoisonError::into_inner)` 并在事务回滚后继续 |
| B. 专用 DB 线程 + `mpsc` 命令通道 | 等价，且显式串行化 | 停机时 `join` 线程就能确定性关闭；不占用 tokio 阻塞池 | 每个操作要写命令枚举/回执通道，样板多 |
| C. 小连接池（r2d2 等） | **不对应**：journal_mode=DELETE 下写者会阻塞读者，池没有收益，还要管 `SQLITE_BUSY` | — | 增加依赖与并发语义；不要用 |

**[建议] A**，把所有 DB 调用封装为 `Db::run(|conn| …)`（闭包在 `spawn_blocking` 内拿锁，返回 `Result`）；事务用 `conn.transaction_with_behavior(TransactionBehavior::Immediate)`（rusqlite transaction.rs:9-15、123 映射到 `BEGIN IMMEDIATE`），`Transaction` 默认 drop 即回滚。忙等待：rusqlite 打开连接时默认设置 `sqlite3_busy_timeout(db, 5000)`（inner_connection.rs:90），与 Python 默认 `timeout=5.0` 一致（实测旧库 `PRAGMA busy_timeout = 5000`）。只有一个进程写同一个库时 BUSY 不会出现；若备份工具同时读库才可能。

### 2.7 PRAGMA 处理

- 旧版（db.py:93-95），**实测**打开后的状态：`journal_mode=delete`、`synchronous=2(FULL)`、`foreign_keys=1`、`page_size=4096`、`user_version=0`、`application_id=0`、`encoding=UTF-8`、`locking_mode=normal`、`busy_timeout=5000`、`auto_vacuum=0`。不用 WAL 的原因见 db.py:90-92（Docker Desktop 挂载 Windows 目录时共享内存不可靠）。
- 打开顺序：`journal_mode=DELETE` → `foreign_keys=ON` → `synchronous=FULL` → `executescript(SCHEMA)`（全部 `IF NOT EXISTS`）→ `meta('schema','1')` 仅在缺失时插入。Rust 必须**保持同样的"只补不改"**，绝不静默重建。`user_version` 目前为 0（旧版没用它）——若以后要做迁移管理才引入 `user_version`，那是 schema 变更，需要单独论证。
- **陷阱**：`PRAGMA journal_mode=…` 会**返回一行结果**，用 `execute` 会报"Execute returned results"；rusqlite 提供 `pragma_update_and_check`（pragma.rs:291）和 `pragma_query_value`（pragma.rs:190）。`synchronous`/`foreign_keys` 是**每连接**设置，必须在每次打开连接后设置。内置 SQLite 编译选项 `SQLITE_DEFAULT_FOREIGN_KEYS=1`（libsqlite3-sys build.rs:171）已默认打开外键，但仍应显式 `PRAGMA`。当前 schema 里实际没有外键。
- 内置 SQLite 的编译标志（build.rs:166-185）：`THREADSAFE=1`、`ENABLE_LOAD_EXTENSION=1`（rusqlite 默认运行时不启用扩展加载，除非开 `load_extension` 特性——**不要开**）、FTS3/4/5、JSON1、RTREE、STAT4、SOUNDEX、`USE_URI`、`ENABLE_API_ARMOR`。这里 `USE_URI` 意味着数据库路径以 `file:` 开头会被当作 URI——路径来自受信任的配置，且应该用 `OpenFlags` 明确控制（建议 `SQLITE_OPEN_NO_MUTEX`+`READ_WRITE|CREATE`，不使用 URI）。
- 文件格式兼容：本机 Python 的 SQLite 是 3.40.1，rusqlite 内置 3.54.1。SQLite 3.x 的数据库文件格式向前向后兼容（SQLite 文档保证，**未实机验证**）；旧 schema 没用 STRICT 表、生成列等新特性。spike S2 用旧版创建的临时库做双向读写验证。

### 2.8 时间戳（无 chrono 还是用 chrono）

- 旧版：`datetime.now().strftime('%Y-%m-%d %H:%M')`（db.py:74-75），**本地时间、无时区、精确到分钟**；compose 里设了 `TZ: <所在时区>`（docker-compose.yml），所以库里存的是本地时间字符串。这些字符串直接显示在界面上（`更新于 {mtime}`、`${x.at}`，app.js:356, 396-399, 569），只用于展示和 `ORDER BY id`，不做时间比较。
- 标准库只有 UTC（`SystemTime`）。要得到本地时间必须有时区库：
  - `chrono` 0.4.45（`clock` 特性）：成熟；读 `TZ`/`/etc/localtime`；
  - `jiff` 0.2.41（2026-11-08）：时区一等公民，可内置 tzdb；
  - `time` 0.3.55 的 `local-offset`：在多线程 Unix 进程里有已知限制（拿不到偏移），**不建议**。
- **陷阱**：`FROM scratch` 或没有 `/usr/share/zoneinfo` 的镜像会让 `TZ=<所在时区>` 失效，时间悄悄变成 UTC，历史记录里的时间就会和本地时间错位。→ **[建议]** 用 `jiff` 并启用内置 tzdb（特性名我凭记忆写作 `tzdb-bundle-always`/`tz-system`，**未核对**），或在镜像里带 tzdata；并在 spike S8 里在最终镜像中验证"设置 `TZ=<所在时区>` 后输出的是该时区的本地时间"。
- 过期计时用 `std::time::Instant`（单调时钟），比旧版的 `time.time()` 更稳，无需 chrono。

### 2.9 SQLite bundled 编译对 C 编译器的要求

- `bundled` 特性由 `cc` crate 编译 `sqlite3.c`（build.rs:140-185），所以构建机需要 C 编译器：
  - Windows MSVC：需要 VS Build Tools（`cl.exe`/`link.exe`）；Windows GNU：需要 `gcc`；
  - Linux glibc：`gcc` 即可（`rust:1-bookworm` 镜像自带）；
  - musl 静态：原生 musl 环境（`rust:alpine`，装 `musl-dev`）最省事；从 glibc 主机交叉到 musl 需要 `musl-gcc`/`x86_64-linux-musl-gcc`；
  - arm64：交叉需要对应的 `aarch64-linux-*-gcc`，或 `cargo-zigbuild`。
- 现有 CI 用 QEMU 模拟 arm64（`.github/workflows/docker.yml` 的 matrix 含 `linux/arm64` 和 `setup-qemu-action`）。**Rust 编译在 QEMU 下会极慢**（我没有测）。→ 见 S9：用 `--platform=$BUILDPLATFORM` 交叉编译，或原生 arm64 runner。
- 取舍：`bundled` 保证 SQLite 版本可复现、与系统库无关；缺点是每次依赖更新都要自己跟进 SQLite 安全修复（RUSTSEC-2026-0459 就是一例）。

### 2.10 XLSX 读取策略（zip + XML）

**旧实现事实**
- `check_xlsx`（server.py:63-85）：长度 <100 → "文件是空的…"；开头不是 `PK`：若是 OLE（`D0CF11E0A1B11AE1`）→ 旧版 .xls/加密提示，否则 → 不是 .xlsx；`ZipFile` 失败 → 已损坏；必须有 `xl/workbook.xml`；遍历 `infolist()`，**按声明大小**累加，单条 >100 MiB 或总和 >300 MiB 拒绝；对每个 `.xml`/`.rels` 成员**整个读进内存**，字节搜索 `<!ENTITY` / `<!DOCTYPE`。
- 随后 `ET.fromstring(z.read(member))`（xlsx_parser.py:49、91-94）：整个成员解压、整树解析。
- **旧版缺陷（已实测）**：UTF-16 编码的 XML 带 `<!DOCTYPE … <!ENTITY e "BOOM">` 时，字节搜索 `b'<!DOCTYPE'` 为 False（逃过检查），但 `ET.fromstring` 能解析并展开成 `BOOM`（`edge_probe.out`）。→ 旧版的实体防御可被绕过；Rust 用事件级拒绝可以堵上。
- 解析语义（必须移植）：`_si_text` 只取 `<si>` 的直接子 `<t>` 和 `<r>/<t>`，忽略 `<rPh>`（xlsx_parser.py:19-29）；`inlineStr` 读 `<is>`；`t="s"` 用 `int(v.text)` 查共享串，**越界跳过**，负数按 Python 负索引取到最后一个（实测 `shared[-1]='two'`——旧版缺陷，Rust 建议当作无效并记为差异）；`t="b"` → `TRUE/FALSE`；其它类型原样取 `<v>` 文本；`val.strip()` 为空则丢弃；行号 `int(row.get('r'))`（接受 `+5`、`' 5 '`、`1_0`、阿拉伯-印度数字，实测），失败跳过；列号取 `r` 的字母前缀；`root.iter('row')` 是**任意深度**，`findall('c')` 是直接子级；隐藏表（`hidden/veryHidden`）跳过；工作表关系通过命名空间 URI 的 `r:id`（按 **URI 而非前缀**匹配）；`Target` 以 `/` 开头则去掉前导 `/`，否则 `posixpath.normpath('xl/'+target)`；不在 zip 名单里则跳过；**不假设 `sheet1.xml`**。
- 重复的行 / 重复的单元格：行会作为多个条目保留，同列单元格后者覆盖。

**Rust 方案**
- ZIP：`zip::ZipArchive::new(Cursor<Bytes>)`（零拷贝，body 已在内存里）。必须：
  1. 只用中央目录元数据做预检：条目数上限、声明大小之和上限、单条上限、压缩比上限（阈值需要实测业主题库和 demo 的比值后定，**未测**，S3 内完成）；
  2. 读取一律 `entry.take(limit).read_to_end(&mut vec)`，**绝不 `Vec::with_capacity(file.size())`**（zip 9.0.0 修复的正是这类"按声明大小预分配"问题；9.0.1 才加"超出声明大小报错"）；
  3. **重复条目名直接拒绝**（Python `NameToInfo` 取最后一个同名条目，`infolist()` 却遍历全部——检查与读取可能看到不同条目，是经典的解析差异攻击面）。zip crate 对重复名的行为**未验证**；
  4. 拒绝加密条目、zip64 以外的不明扩展、Deflate/Stored 以外的压缩方法（旧版理论上接受 bzip2/lzma，Excel 实际不会用）；
  5. 拒绝路径含 `..`/绝对路径的条目名（我们不解压到磁盘，只按名读取，但仍应拒绝）；
  6. 重叠条目（"zip quine"式炸弹）：靠"声明大小之和"上限 + 每次读取 `take` 限制来封顶；
  7. 保持旧版的上限作为**最宽的上限**（单条 100 MiB、总和 300 MiB）并**建议收紧**（正常题库 1–2 MB，server.py:49 注释）。收紧到多少是业主决定，见 openQuestions。
- 解压后内存：一份 20 MiB 的上传在内存中为 `Bytes`；最大成员 100 MiB 的 `Vec` 是最坏情况。**流式**解析（不要先 `read_to_end` 再解析）可以把峰值降到缓冲区大小，代价是代码更复杂；建议先用 `take(limit)` 读入 `Vec`（有上限），把流式作为后续优化。
- XML：用 `quick-xml::NsReader`（命名空间感知，按 URI 匹配 `r:id`）：
  - **DOCTYPE 拒绝**：遇到 `Event::DocType` 立即报错（quick-xml 会作为独立事件报告，events/mod.rs:1800）。0.39.3 changelog 明确说"目前不校验 DTD 的有效性"，所以**必须在事件层拒绝**，不能指望它报错。保留旧版"任一 `.xml/.rels` 含 `<!DOCTYPE`/`<!ENTITY` 就整体拒绝"的**预扫描**（对全部成员做带重叠的流式字节搜索，且同时拒绝 UTF-16 BOM 开头的 XML），保持旧版"拒绝整份文件"的语义。
  - **实体**：自 0.38.0 起实体引用作为 `Event::GeneralRef` 单独报告，**不会自动展开**（changelog 431-438、467）；预定义实体（`&amp;` 等）和字符引用（`&#x41;`）需要我们自己解析并**拼回文本**。漏掉 `GeneralRef` 会丢字符。旧版（ET）会合并文本与 CDATA、跳过注释（实测 `a<!--c-->b` → `ab`，`a<![CDATA[<x>]]>b` → `a<x>b`），所以 `<t>` 内要把 `Text`/`CData`/`GeneralRef` 都累加。
  - **换行规范化**：expat 会把 `\r\n`、`\r` 规范化成 `\n`（实测）；quick-xml 0.42 的 `xml10_content()` 等方法返回规范化后的内容（changelog 所述），**是否默认规范化 / 是否要显式调用：[未证实]**，spike S4。幸好 `parse_question` 对 A/B/C 列又做了 `_norm_nl`（xlsx_parser.py:250-251），影响很小。
  - `Reader` 配置：默认 `check_end_names=true`、`allow_unmatched_ends=false`、`allow_dangling_amp=false`、`trim_text=false`（reader/mod.rs:52-274 已核对），与 ET 对空白的处理一致（保留）。
  - **输入必须是 UTF-8**：0.42.0 起非 UTF-8 输入直接报 `Error::Encoding`（changelog #963），所以 UTF-16 的 XML 会被拒绝——这是对的（比旧版严格，旧版会接受）。BOM 处理 **[未证实]**。
  - **资源上限（旧版没有，规格要求）**：最大元素深度（OOXML 正常 <32，限 64）、最大单元格数、最大共享串数量与总字符数、最大行数、工作表数、单个文本节点长度；命名空间绑定总数 quick-xml 0.42 默认 128，可保留。
- 备选 roxmltree：好处是 API 更接近 ET（树 + `text()`），`allow_dtd=false` 默认拒绝 DTD 且有 billion-laughs 防护，`nodes_limit` 可限节点数；坏处是整树载入，100 MiB 的成员会占数百 MiB。**只有当 quick-xml 的语义移植被证明太麻烦时才考虑**，并且要把成员上限收紧到几 MiB。
- 解析时限：对 `spawn_blocking` 外面套 `tokio::time::timeout` 只会放弃等待，**不会停止线程里的循环**。→ 解析循环里每处理 N 个事件检查一次 `Instant` 截止时间（协作式取消），超时返回错误。quick-xml 0.41 修的 RUSTSEC-2026-0194 的性质（纯 CPU，I/O 超时打断不了）正好说明需要协作式检查。
- 内存：`Bytes` 引用上传缓冲；解析产物（`Vec<Question>`）在 Pending 中保留，需要总题数/总字节上限（旧版没有）。

### 2.11 Pending 存储

**旧实现（server.py:150-174）**：全局只有**一个槽**；`put` 用 `secrets.token_urlsafe(16)`（16 字节 → 22 个 URL-safe 字符）覆盖旧的；`take` 在锁内比较 `compare_digest`、检查 `expires < time.time()`，**匹配才清空**。实测：令牌错误时槽**保持原样**（"pending still present after wrong-token confirm: True"）；过期的项也不会被清掉，直到被覆盖。TTL=1800 s。`cancel` 无条件 `drop()`，**任何请求都能取消**。无会话隔离（存在的访问模型：单人 + Cloudflare Access）。
- 令牌**不出现在 URL 里**，只在 POST 的 JSON 正文（app.js:479），所以访问日志不会泄漏它。

**Rust**
```rust
pub struct PendingStore { slot: std::sync::Mutex<Option<PendingImport>> }
impl PendingStore {
    pub fn put(&self, filename, sheets, questions) -> Token;   // getrandom::fill([u8;16]) + base64url（无填充）= 22 字符
    pub fn take(&self, presented: &str) -> Option<PendingImport>; // 锁内：subtle ct_eq + Instant::now() > expires；匹配才 take()
    pub fn drop_pending(&self);
}
```
- `Mutex<Option<…>>` 只在极短的临界区持有，**绝不跨 `.await`**；并发 confirm：第一个 `take` 返回 `Some`，第二个 `None` → 409，保证"不会应用两次"。
- 令牌类型写成 `Token(String)` 并手写 `Debug` 输出 `Token(***)`，禁止 `Serialize`（预览响应里单独序列化成字符串）。
- 过期用 `Instant`；可以顺手在 `take`/`put` 时清掉过期项（无可见差异）。
- 令牌比较要接受任意 JSON 值：非字符串 → 直接当不匹配（409），这比旧版（非 ASCII 抛 `TypeError`）更好。
- 不要发明会话隔离：保持单槽，写进文档。

### 2.12 tracing 不泄露令牌/文档内容

- 旧版没有任何日志（`log_message` 空实现，server.py:197-198）；Rust 是**新增**能力，因此是新增的泄露面。
- 规则：不记录请求/响应正文；`#[instrument(skip_all)]`；`TraceLayer::new_for_http()` 默认只记录方法/URI/状态/耗时（`include_headers(false)` 默认），而令牌不在 URL 里；`X-Filename` 视为个人信息，不记录；`Token` 手写 `Debug`；`Host`/`Origin` 拒绝日志要截断并去掉控制字符（旧版做了 `re.sub(r'[\x00-\x1f\x7f]','',…)[:80]`，server.py:246、294-295）。
- `tracing-subscriber ≥0.3.20` 并关闭 `ansi`（RUSTSEC-2025-0055：用户可控字符串里的 ANSI 转义可注入终端日志）。
- 日志级别用新环境变量（建议 `XEXAM_LOG`，默认 `info`），属于新增配置，不是旧配置。

### 2.13 环境变量（旧语义，来自 server.py 与 README:500-503）

| 变量 | 旧版语义（行号） | Rust 注意 |
|---|---|---|
| `XEXAM_DATA` | `--data` 的默认值；缺省 `<APP_DIR>/data`；库文件 `<data>/xexam.db`（392-393, 380） | 默认值要改成 `/data`；路径必须可配置 |
| `XEXAM_HOST` | 默认 `127.0.0.1`（Dockerfile 设 `0.0.0.0`）（394） | 同 |
| `XEXAM_PORT` | 默认 `8765`，`int()` 解析失败会直接抛回溯（395）；**绑定失败会自动尝试 port..port+19**（423-432） | 容器里端口自动漂移会破坏端口映射，**建议去掉漂移，失败即退出**，写进差异表 |
| `XEXAM_ALLOWED_HOSTS` | 逗号分隔；每项 `strip`，去掉 `scheme://`，去掉路径，再 `host_name`（110-121）；默认集合 `{localhost, 127.0.0.1, ::1}` ∪ 配置（439） | 必须逐字符复刻字符串处理（见下） |
| `XEXAM_ALLOWED_ORIGINS` | 同一解析函数；最终 `allowed_origins = origins ∪ ALLOWED_HOSTS 的项`（436, 440）；只放行 Origin，不放行 Host | 同上 |
| `XEXAM_ALLOW_ANY_HOST` | `strip().lower()` 在 `1/true/yes/on` 才为真（435）；只跳过 **Host** 检查，Origin 检查照常 | 同上 |
| `XEXAM_BIND` | **不是服务端变量**，只在 compose 里决定宿主机端口映射 | 新架构里端口由 compose/Nginx 决定 |
| `TZ` | compose 设 `<所在时区>`，影响时间戳 | 见 §2.8 |
| CLI | `--data --host --port --no-browser --allow-shutdown --import <xlsx>` | `--import`（离线导入）、`--allow-shutdown`、自动开浏览器（`webbrowser.open`）、`progress.json` 一次性迁移（server.py:378-388、db.py:281-308）——**哪些保留是业主决定**，见 openQuestions |

`host_name` 的怪癖（必须按字符串逻辑复刻，不能换成 URL 解析库）：小写 → 以 `[` 开头取括号内；否则 `rsplit(':',1)[0]`；去掉末尾 `.`。例如无括号的 `::1` 会变成 `:`，`a:b:c` 变成 `a:b`（读代码得出，db 之外的 `host_test.py` 覆盖了主要用例）。`origin_host` 只认 `http://`/`https://` 前缀、含 `@` 返回 None、不比较协议和端口。Origin 缺失视为放行（server.py:228-230）；与 Host 同主机名或在白名单即放行。这些规则要用业主的 `host_test.py`（55 项）改写成 Rust 集成测试，并对旧服务器和新服务器各跑一遍。

### 2.14 健康检查子命令

- 旧 Dockerfile：`python -c "urllib.request.urlopen('http://127.0.0.1:8765/api/info', timeout=6)"`（Dockerfile HEALTHCHECK）。urllib 默认发 `Host: 127.0.0.1:8765`，恰好在默认白名单里。
- 新镜像若用 distroless/scratch，里面没有 curl/wget → **[建议]** 同一个二进制加子命令 `xexam-backend healthcheck`：用 `std::net::TcpStream` 发一个手写的 `GET /api/info HTTP/1.0` + `Host: localhost`，检查返回 200，读取超时 5 s，退出码 0/1。不需要任何依赖，也不要用 `Host` 之外的值（否则会被 Host 检查拦成 403）。Nginx 容器用自己的 `wget`（alpine 自带）。
- compose 的 `depends_on: condition: service_healthy` 依赖这个检查。

### 2.15 静态 musl 与 glibc、镜像大小

- 选项：`FROM scratch` + 静态 musl 二进制；`gcr.io/distroless/cc`（glibc，动态）；`debian:bookworm-slim`。
- 影响：
  - scratch：最小，但**没有 `/usr/share/zoneinfo`**（见 §2.8）、没有 shell、需要数字 `USER 10001:10001`、`/data` 要在构建期创建并 `chown`（scratch 里没有 `RUN`，可在构建阶段 `COPY --chown`）、没有 CA（本服务不出站，无影响）；
  - musl 的默认内存分配器比 glibc 慢，但本服务是单人使用，**无需关心**；
  - bundled SQLite 在 musl 下由 `cc` 编译，无障碍（原生 musl 环境）。
- 大小：**我没有测量**。经验量级（估计）：scratch+静态二进制 约 10–20 MB，distroless/cc 约 +20 MB，debian-slim 约 +75 MB；对照旧镜像 `python:3.12-slim` 约 120+ MB。S9 实测。
- 非 root：旧镜像已用 `useradd --uid 10001`（Dockerfile），保持同样的 UID，避免卷里文件权限错位（命名卷 `xexam_data` 中已有文件属于 10001）。

### 2.16 测试方法

- 单元/集成：`axum::Router` + `tower::ServiceExt::oneshot`（tower 的 `util` 特性），`http_body_util::BodyExt::collect` 读响应；每个测试用 `tempfile` 的临时 SQLite；`build_router(AppState)` 放在 `lib.rs`，测试直接构造 `AppState`。
- **黄金文件**：以我生成的 `golden/*.json`（旧实现 + 虚构 demo 题库）为起点，比较时规范化时间戳和令牌；`PyCompactFormatter` 使字节级比较成为可能。
- **差分测试（强烈建议）**：以旧 `xlsx_parser.py` 为 oracle——脚本生成大量合成工作簿（含 U+001C–U+001F、NBSP、U+3000、全角数字、孤立组合附加符号、超长行），旧解析器输出 JSON，Rust 输出 JSON，逐题比较；`proptest` 生成随机单元格文本。
- 并发：N 个并发 `confirm` 同令牌恰好 1 个 200 其余 409；并发 `attempt` + `confirm` 不损坏计数；`reset` 与 `attempt` 交错。
- 供应链：`cargo deny`（licenses/bans/advisories/sources）+ `cargo audit`；`cargo nextest`（可选）；`cargo-fuzz`（可选，需 nightly）对 `parser::xml_sheet`/`zip_guard` 做模糊测试。
- 现有可复用的外部脚本：`host_test.py`（Host/Origin，纯函数表 + 真实服务 5 个场景）、`tunnel_sim.py`（真 Chrome 经 HTTPS 代理）、`mm_check.py`；它们都以 `server.py` 为对象，改成"针对任意 base URL"后可对新旧两个后端各跑一遍。

---

## 3. 文本与数值语义陷阱（实测，直接影响数据兼容）

| # | 旧版事实（命令/行号） | Rust 风险 | 处理 |
|---|---|---|---|
| 1 | **题目 id**：`sha1(sheet + '\n' + re.sub(r'\s+','',stem)[:300] + '|' + re.sub(r'\s+','',first)[:200]).hexdigest()[:12]`（xlsx_parser.py:245-247）。id 是 `stats` 表的主键——**id 一变，学习记录就全部脱钩** | 最大的数据风险 | 见下面 #2–#4，并用旧库做回归（S5） |
| 2 | Python `re` 的 `\s` 与 `str.strip()` 是同一个集合：**29 个码位**，比 Unicode `White_Space`（25 个）多 U+001C–U+001F（`semantics_probe.out`） | Rust `regex` 的 `\s` 和 `str::trim()` 都是 `White_Space`，**少这 4 个** → 含 U+001C–U+001F 的题干 id 会变 | 自写 `is_py_space(c) = c.is_whitespace() \|\| ('\u{1c}'..='\u{1f}').contains(&c)`，正则里用 `[\s\x1c-\x1f]`；所有 `strip()`/`trim()` 都走这个函数 |
| 3 | `[:300]`、`[:200]`、`[:64]`、`[:80]`、`[:120]`、`[:40]` 都是**码位切片**（实测 400 个增补平面字符的 id 等于 300 个的） | Rust 字节切片会 panic 或结果不同 | 一律 `chars().take(n)`；开启 `clippy::string_slice` lint |
| 4 | 不做 Unicode 规范化（实测 NFC 与 NFD 的 id 不同） | 加 `unicode-normalization` 会改 id | **不要引入该 crate**；规格列的"unicode-normalization"在此不适用 |
| 5 | `percent = round(100 * good / total)`（db.py:269）：Python 的 `round` 是**银行家舍入**；平局如 1/8 → 12，3/8 → 38（实测） | Rust `f64::round()` 是"远离零"：1/8 → 13 | 用 `f64::round_ties_even()`（1.77 稳定）；`total ≤ 2000` 内共 2600 个平局、1300 个与"远离零"不同，已存入 `golden/percent_tie_cases.csv` |
| 6 | `NUM_PREFIX = ^\s*(\d{1,2})\s*[.．、)）]\s*`，`\d` 是 Unicode `Nd`（Unicode 13：650 个码位，65 段），`int()` 能解析全角数字（实测 `int('１２') == 12`，且全角"１．"能匹配）——日语内容里全角序号是真实场景 | Rust `parse::<u32>` 只认 ASCII | 自写 `nd_value(c)`（段起点表）；`regex` 的 `\p{Nd}` 在 Unicode 16+ 更多码位，差异极小 |
| 7 | `canon()`：`.lower()` 是完整 Unicode 小写（`İ` → `i̇` 两个码位；`ΑΣ` → `ας` 词尾 sigma，实测） | 逐字符 `to_lowercase` 会丢词尾 sigma | 用 `str::to_lowercase()`（实现了最终 sigma） |
| 8 | 旧版 Python 3.10.11 的 Unicode 数据库是 **13.0.0**（实测）；Docker 里的 `python:3.12-slim` 是更新的版本，即旧版自己在本机和容器里就有细微差异 | Rust 1.99 的 Unicode 更新 | 只影响 `canon()` 对新近才有大小写的字符，两边用同一函数比较，几乎无影响；记录即可 |
| 9 | `parse_letters` 的 `(?<![A-Za-z0-9])…(?![A-Za-z0-9])` 用了前后查看（xlsx_parser.py:216） | `regex` crate 不支持环视 | **手写边界检查**（几行循环），不要为此引入 `fancy-regex`（有回溯） |
| 10 | `HINT_RE` 含惰性量词（xlsx_parser.py:154）；我对两种构造输入试到 12800 次重复，Python 耗时 0.006 s，**没找到灾难回溯**（不是证明） | `regex` 线性时间、结果与回溯引擎的 leftmost-first 语义一致 | 直接用 `regex`；这是 Rust 方案优于旧版的一点 |
| 11 | `make_id` 之后冲突去重 `id`, `id-2`, `id-3`…（xlsx_parser.py:401-406） | — | 照抄 |
| 12 | `_q_hash` 用 `json.dumps(sort_keys=True)` 比较"同 id 的题有没有变"，排除 `row`（db.py:78-80） | — | 比较两个 `serde_json::Value`（忽略 `row`）；对象相等与键顺序无关 |
| 13 | `plan_import` 的 `removed_samples` 取 `stem[:40]`（db.py:194） | 同 #3 | `chars().take(40)` |
| 14 | `apply_import` 对"下架"题的 `seq` 按 `SELECT id FROM questions WHERE active=1` 的无序结果赋值（db.py:221-223） | 顺序取决于查询计划 | 对可见行为**无影响**（下架题不出现在 `/api/bank` 的 `questions` 里，恢复时会被重新赋 `seq`） |
| 15 | `clean_filename`：`unquote` → 反斜杠换斜杠 → 取最后一段 → 去控制字符 → `strip()` → `[:120]` 或 `未命名.xlsx`（server.py:124-127；实测 `..%2F..%2Fevil%5Cname.xlsx` → `name.xlsx`） | `percent-encoding` + `decode_utf8_lossy` 等价于 `unquote`（utf-8, replace） | `HeaderValue::to_str()` 对非 ASCII 会失败，按字节处理 |

---

## 4. 本机 Windows 环境（只读检查，无安装）

| 项 | 结果 |
|---|---|
| rustc / cargo / rustup | `where` 找不到；`~/.cargo`、`~/.rustup` 不存在（**我检查时**）。协调者后来的 spike 报告：本机现有 rustc/cargo 1.99.1（stable-x86_64-pc-windows-gnu）、gcc 16.2、clippy、rustfmt——时间上在我检查之后，按协调者为准 |
| MSVC | 无 `cl.exe`、无 `vswhere.exe`、无 `C:\Program Files (x86)\Microsoft Visual Studio`、无 Windows Kits；注册表仅有 VS 14.0 的空键。**没有可用的 MSVC Build Tools** |
| `link` | `where link` → Git for Windows 目录下的 `usr\bin\link.exe`，`link --version` 显示是 **GNU coreutils 8.32 的 `link`**，不是 MSVC 链接器。若将来装了 MSVC 目标，Git for Windows 的这个 `link.exe` 排在前面会**抢占 PATH**（经典坑） |
| 其它编译器 | PATH 上没有 gcc/clang/cmake/make/ninja（后来 spike 有 gcc 16.2） |
| WSL | `wsl -l -v`：`docker-desktop`、`Ubuntu-18.04`、`Ubuntu`，均为 WSL2、均 **Stopped** |
| Docker | CLI 28.1.1 存在；**我检查时引擎未运行**（`docker info` 连不上管道）；协调者后来说 Docker Desktop 已运行 |
| Node / Python / winget | Node v26.13.0；Python 3.10.11（SQLite 3.40.1）；winget 可用 |

**含义**：Windows 上开发用 gnu 目标即可；发布产物在 Docker（Linux）里构建，不依赖 Windows 工具链。若业主想用 MSVC 目标，需要安装 VS Build Tools（体积很大）。

---

## 5. 设计提案（仅文档，未创建项目）

### 5.1 `backend/Cargo.toml` 草案

```toml
[package]
name = "xexam-backend"
version = "0.3.0"            # 与仓库 VERSION 文件保持一致的方式见 openQuestions（旧 /api/info 返回 "0.2" 这种两段式）
edition = "2024"
rust-version = "1.89"        # rusqlite 0.41.1 的 MSRV
publish = false

[dependencies]
axum = { version = "0.8.9", default-features = false, features = ["http1", "tokio"] }   # 不用 json/form/query 提取器（行为不同），不开 http2
tokio = { version = "1.54.1", features = ["rt-multi-thread", "macros", "signal", "net", "time", "sync"] }  # >=1.54.1（1.54.0 已 yank）
tower = { version = "0.5.3", default-features = false, features = ["util", "limit"] }
tower-http = { version = "0.7.1", default-features = false, features = ["trace", "timeout", "set-header", "catch-panic", "limit"] }
serde = { version = "1.0.229", features = ["derive"] }
serde_json = { version = "1.0.154", features = ["preserve_order"] }
rusqlite = { version = "0.41.1", default-features = false, features = ["bundled"] }       # 可选再加 "backup"
thiserror = "2.0.21"
anyhow = "1.0.104"                                            # 仅 main.rs / 启动阶段
tracing = "0.1.44"
tracing-subscriber = { version = "0.3.23", default-features = false, features = ["fmt", "env-filter", "std"] }  # 不开 ansi
zip = { version = "9.5.0", default-features = false, features = ["deflate-flate2-zlib-rs"] }  # 特性名待 S3 用 cargo tree 确认；协调者 spike 用的是 "deflate"
quick-xml = "0.42.0"                                          # default = []
regex = "1.13.1"
sha1 = "0.11.0"                                               # 仅用于旧题目 id 指纹
getrandom = "0.4.3"                                           # 令牌：getrandom::fill(&mut [u8;16])
subtle = "2.6.1"                                              # 或 constant_time_eq 0.6.1
base64 = "0.23.1"                                             # 也可手写 16 字节的 base64url，省一个依赖
percent-encoding = "2.3"
jiff = "0.2.41"                                               # 本地时间（TZ=<所在时区>）；特性名待 S8 确认
# 不引入：rand、sha2、md5、unicode-normalization、fancy-regex、chrono、time、utoipa-swagger-ui

[dev-dependencies]
tempfile = "3"
http-body-util = "0.1.5"
tower = { version = "0.5.3", features = ["util"] }

[features]
# openapi = ["dep:utoipa", "dep:utoipa-axum"]   # S10 的结论出来之前先不启用

[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
strip = true
panic = "unwind"             # 配合 CatchPanicLayer / JoinError::is_panic，解析器的 bug 不应整进程退出
overflow-checks = true       # 解析器里的算术溢出要报错而不是回绕

[lints.rust]
unsafe_code = "forbid"
[lints.clippy]
string_slice = "warn"        # 禁止按字节切字符串（#3）
unwrap_used = "warn"
indexing_slicing = "warn"
```

### 5.2 模块布局

```
backend/
  Cargo.toml  Cargo.lock  rust-toolchain.toml  deny.toml
  src/
    main.rs                 # 子命令: serve | healthcheck | import <xlsx> | migrate-progress（后两者待业主确认）；初始化 tracing；运行时；信号；优雅停机
    lib.rs                  # pub fn build_router(AppState) -> Router，供集成测试使用
    error.rs                # AppError（IntoResponse）、ImportError
    config/    mod.rs env.rs     # Config::from_env_and_args（XEXAM_*）；AllowedHosts / AllowedOrigins / allow_any_host
    models/    question.rs sheet.rs progress.rs import.rs api.rs   # 字段顺序 = 旧 JSON 键顺序
    api/       mod.rs router.rs info.rs bank.rs progress.rs imports.rs upload.rs import_confirm.rs import_cancel.rs attempt.rs reset.rs shutdown.rs json.rs(PyJson+PyCompactFormatter) fallback.rs guard.rs(Host/Origin 中间件)
    services/  import_service.rs   # parse_upload / validate_import / plan_import / create_pending_import / confirm_pending_import / cancel_pending_import / apply_import
               pending.rs attempt_service.rs clock.rs
    parser/    mod.rs zip_guard.rs xml_sheet.rs shared_strings.rs workbook.rs question.rs text.rs(is_py_space, canon, nd_value) id.rs(make_id)
    database/  mod.rs schema.rs store.rs queries.rs legacy_progress.rs
    security/  host.rs origin.rs body.rs token.rs filename.rs headers.rs
  tests/       common/mod.rs  api_*.rs  parser_golden.rs  parser_differential.rs  security_host_origin.rs  concurrency.rs  shutdown.rs
  tests/fixtures/  tests/golden/
```
规格的概念函数名都放在 `services::import_service`，按它们的字面名字命名，方便对照。

### 5.3 主要类型

```rust
#[derive(Clone)]
pub struct AppState(Arc<Inner>);
struct Inner {
    cfg: Config,                         // allowed_hosts: HashSet<String>, allowed_origins, allow_any_host, allow_shutdown, limits, version
    db: Db,                              // Arc<std::sync::Mutex<rusqlite::Connection>>；Db::run(|conn| …) = spawn_blocking + 拿锁
    pending: PendingStore,               // Mutex<Option<PendingImport>>
    upload_slots: tokio::sync::Semaphore,// 2 个许可，限制并发解析
    shutdown: Option<tokio::sync::mpsc::Sender<()>>, // 仅 --allow-shutdown 时存在
}

pub struct PendingImport { token: Token, filename: String, sheets: Vec<SheetInfo>, questions: Vec<Question>, expires: Instant }
pub struct Question { kind: Kind, tag: String, stem: String, options: Vec<Opt>, items: Vec<Item>, answer: Vec<u32>, translation: String, zh: Option<Zh>, explanation: String, mnemonic: String, note: String }  // id/sheet/row 在读取时追加，顺序见 §2.3
```

### 5.4 错误枚举（要点）

```rust
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("forbidden host")]            ForbiddenHost { seen: String },   // GET: text/plain 403
    #[error("denied")]                    Denied(String),                   // POST: {"error": …} 403（Host/Origin）
    #[error("bad content type")]          UnsupportedMediaType,             // 415 {"error":"bad content type"}
    #[error("too large")]                 JsonTooLarge,                     // 413 {"error":"too large"}
    #[error("upload too large")]          UploadTooLarge { max_mb: u64 },   // 413 + 中文
    #[error("bad json")]                  BadJson,                          // 400
    #[error("{0}")]                       BadRequest(String),               // 400 + 中文（xlsx 校验 / 没有成绩）
    #[error("not found")]                 NotFound,                         // 404（GET 文本 / POST JSON）
    #[error("pending gone")]              PendingGone,                      // 409 + 中文
    #[error("shutdown disabled")]         ShutdownDisabled,                 // 403 + 中文
    #[error("db error")]                  Db(#[from] DbError),              // 500；中文前缀，细节只进日志
    #[error("internal")]                  Internal,                         // 500 通用
}
```
`ImportError`（parser 层）单独定义，变体对应旧的每条中文 `ValueError` 文案。

### 5.5 `Cargo.toml` 之外的 Nginx / 容器相关约束（给部署同事）

- Nginx：`proxy_http_version 1.1`、`proxy_set_header Host $host`、不转发 `X-Forwarded-*` 给后端当作信任来源、`client_max_body_size 20m`、`client_header_timeout`/`client_body_timeout`、`error_page 413` 返回 JSON、`proxy_request_buffering on`、`limit_except GET HEAD POST`。
- 后端 compose：私有网络、不发布端口、`XEXAM_HOST=0.0.0.0`（容器内）、`XEXAM_ALLOWED_HOSTS` 必须包含公网域名（Cloudflare 隧道直达 Nginx 时 Host 就是公网域名）、`stop_grace_period` ≥ 10 s、卷 `data:/data`，UID 10001。

---

## 6. 不确定的选择与对应 spike

| 编号 | 不确定项 | 可选方案 | 用什么小 spike 解决 |
|---|---|---|---|
| S1 | 处理器在读完 body 之前就回 413，浏览器/curl 看到的是 413 还是连接重置；有无排空；在 Nginx 之后是否一致 | 手写排空 vs 依赖 Nginx | 20 MB+1、50 MB、分块上传各一次，直连与经 Nginx 各一次，记录客户端看到什么 |
| S2 | 旧库与 rusqlite 双向互读写；`journal_mode` 的 pragma 写法；`BEGIN IMMEDIATE` 与 busy 行为 | 同 §2.6/2.7 | 用**旧代码在临时目录**新建库（不碰业主的库，也不碰其副本之外的任何东西）→ Rust 读 `bank()/progress()` 与黄金文件比对 → Rust 写入 → 旧代码再读；`PRAGMA integrity_check` |
| S3 | zip crate：重复条目名、zip64、数据描述符、加密、未知压缩方法、重叠条目、声明大小撒谎、`take(limit)` 行为；特性名 `deflate-flate2-zlib-rs` 是否可用；压缩比阈值 | zip crate vs 自写最小中央目录读取 + `flate2` | 构造 8–10 个恶意 ZIP（Python 脚本生成），断言全部在上限内被拒绝，峰值 RSS 有界；同时统计业主两份题库和 demo 的各成员压缩比（只读元数据） |
| S4 | quick-xml 与 ET 的文本语义是否一致：注释/CDATA/GeneralRef/换行规范化/BOM/xml:space/默认命名空间与前缀命名空间 | quick-xml vs roxmltree | 以 ET 为 oracle，对一批小 XML 逐项比较 `<t>` 文本和属性值 |
| S5 | 全部文本语义（§3）：`is_py_space`、`nd_value`、`canon`、`make_id`、`round_ties_even`；对旧库里**所有现有题目**重算 id 是否一致 | 自写函数 | 差分测试 + 用旧解析器与 Rust 解析器分别解析同一批合成工作簿，id 与 JSON 逐项比较 |
| S6 | `PyCompactFormatter` 字节兼容；HEAD 的 `Content-Length`；404/405/501 形态；Host 缺失时 hyper 的行为；`//path` 行为 | 保持旧形态 vs 改标准形态 | 对旧服务器（进程内探针）与新 Router 发同一批请求，逐字节/逐头比较 |
| S7 | `docker stop` 在有 Nginx 上游 keep-alive 连接时是否 <9 s；进行中的导入是否完成；超时包装是否生效 | `with_graceful_shutdown` + `timeout` | 容器里启动 → 制造一个慢请求和一个长连接 → `docker stop` 计时 |
| S8 | 本地时间：jiff 还是 chrono；scratch/distroless/debian-slim 中 `TZ=<所在时区>` 是否生效 | jiff（内置 tzdb） vs chrono + tzdata | 三种基础镜像各输出 `%Y-%m-%d %H:%M` |
| S9 | 镜像大小、构建时间、arm64 交叉编译（CI 目前是 QEMU）；musl 还是 glibc | rust:alpine 原生 musl、`cargo-zigbuild`、原生 arm64 runner | 实测 amd64 与 arm64 的构建耗时和最终镜像大小 |
| S10 | OpenAPI：utoipa 6 派生 vs 手写 `openapi.yaml`；openapi-typescript 的 peer `typescript ^5.x` 与前端 TS 版本 | 手写 + 契约测试 vs 派生 | 12 个端点里挑 3 个，两种方式各出一份并与黄金响应对比；`npm i --dry-run` 看 peer 冲突 |
| S11 | Slowloris：直连后端（无 Nginx）是否需要自建 hyper-util accept 循环 + `TokioTimer` | `axum::serve` + Nginx vs 自建循环 | 用一个慢速客户端脚本逐字节发请求头，观察后端是否在 30 s 内断开 |
| S12 | `spawn_blocking` 解析在超时后仍在跑；协作式截止检查的开销 | 每 N 个事件检查一次 | 构造高密度 XML，断言超时后线程在可接受时间内退出 |

---

## 7. 给四份文档的输入（对应规格的 12 项）

- **5 安全控制（必须保留）**：Host 校验（`XEXAM_ALLOWED_HOSTS` / `ALLOW_ANY_HOST`）、写请求 Origin 校验（只比主机名；缺失放行；`XEXAM_ALLOWED_ORIGINS`）、Content-Type 子串检查、`nosniff` + `no-store`、令牌 CSPRNG + 常量时间比较 + 一次性 + 30 分钟、`BEGIN IMMEDIATE` 事务、上传魔数/ZIP/实体检查、路径穿越不适用（Rust 不再服务静态文件）、`/api/shutdown` 默认禁用、非 root。
- **7 已有缺陷/技术债**：UTF-16 XML 绕过实体检查；`seconds=inf`/`results` 非列表/非 ASCII 令牌导致连接被断开；错误信息泄露 Python 异常名和 sqlite 文本；`Server` 头泄露版本；解析器无行数/单元格数/深度上限，单个 XML 成员最多 100 MiB 整个读进内存；线程无上限；`NameToInfo` 与 `infolist()` 的重复条目差异；Content-Type 子串匹配；`apply_import` 计划与写入之间有竞态窗口；负数共享串索引按 Python 负索引；过期 Pending 不清除；`bank()` 两次拿锁。
- **10 兼容性问题**：JSON 空格风格（smoke-test 依赖）、键顺序、`percent` 舍入、`\s` 集合、全角数字、码位切片、501→405、HTTP/1.0→1.1、分块上传宽松度、Nginx 的 HTML 413、Host 重写、时区、`/api/shutdown` 与 CLI（`--import`、`progress.json` 迁移）去留。
- **11 数据迁移风险**：id 漂移（最高）；时间戳时区；SQLite 3.40 → 3.54 的文件格式；`journal_mode`/`synchronous` 必须每次设置；schema 只补不改；`progress.json` 一次性迁移不能丢。
- **12 回滚**：保留 Python 实现（legacy 目录或分支）；先在副本上验证（`PRAGMA integrity_check`）；回滚 = 切回旧镜像 + 同一个命名卷，因为 schema 不变，旧版可直接读 Rust 写过的库（S2 验证）。

---

## 8. 未验证（notVerified）

1. 没有运行 `cargo audit` / `cargo deny`，只读了 RustSec 库；没有看 hyper 1.x 的 HTTP/2 类公告（后端不启用 http2）。
2. zip 重复条目名、加密、zip64 等行为未实测；`deflate-flate2-zlib-rs` 特性名依据 features 表，未编译验证。
3. quick-xml 的 EOL 规范化与 BOM 行为未实测；xml-rs 的 DOCTYPE/实体行为未读源码。
4. axum-extra `Host` 提取器信任 `X-Forwarded-Host` 的说法来自记忆，未查源码。
5. jiff 的特性名、base64 0.23 的 API、`#[tokio::main]` 运行时 drop 对阻塞任务的行为未查证。
6. 没有测量任何镜像大小、构建时间、arm64 交叉编译；没有做 HEAD 的 `Content-Length` 实测。
7. 没有读业主的两份私有题库的内容；也没有统计其压缩比（只在 spike 里做元数据统计）。
8. `//x/api/info` 的旧版路径行为来自 urllib 文档，没有运行。
9. 真实服务器中"处理器抛异常 → 断连"是标准库行为，没有起服务器实测（规则禁止开端口）。
10. 我对旧 `Handler` 的 60 余项请求全部是**进程内假 socket**，没有走真实 TCP，因此没有观察到 Nagle/排空/连接重置这类网络层现象。
11. 本地工具链：我检查时没有 rustc；之后的状态以协调者的 spike 为准。

## 9. 开放问题（openQuestions，需业主/协调者决定）

1. 非 Docker 的本机运行方式（`start.bat` + `--allow-shutdown` + 自动开浏览器 + `--import` 离线导入）在新架构里是否保留？若保留，Rust 二进制要不要内置静态文件服务（与"只有 Nginx 提供静态文件"冲突）？
2. `progress.json` → 数据库的一次性迁移（server.py:378-388、db.py:281-308）：移植、改成显式子命令，还是确认业主库已迁移后删除？
3. 501（其他方法）与 404（POST 到 GET-only 路由）是保持旧形态还是改标准 405？是否接受"分块上传被接受"这一有意差异？
4. JSON 是否必须字节级兼容（`PyCompactFormatter`），还是改冒烟测试用语义比较？
5. 上传的 ZIP 限额：保持 100 MiB / 300 MiB 的旧上限，还是收紧（正常 1–2 MB）？是否加行数/单元格数上限？
6. `/api/info` 的 `version` 是 `0.2` 这种两段式，Cargo 需要三段式：如何保持与 `VERSION` 文件及 CI 的"README 最新条目 = VERSION"检查一致（`include_str!`、构建参数或 `build.rs`）？
7. 去掉 `XEXAM_PORT` 的端口漂移（port..port+19）是否可接受？
8. OpenAPI 用派生还是手写（S10）；前端 TypeScript 版本是否会与 `openapi-typescript` 的 `typescript ^5.x` 冲突？
9. 镜像基础：scratch 静态 musl、distroless/cc、debian-slim 三选一（取决于 S8/S9）；arm64 构建用 QEMU（极慢）还是交叉编译？
10. 不带 `Origin` 的 POST 放行（命令行工具）与 Content-Type 子串匹配这两条旧规则，是否作为"保持行为"的一部分原样保留？
