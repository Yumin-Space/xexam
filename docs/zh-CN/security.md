[English](../en/security.md) | [日本語](../ja/security.md) | **简体中文**

# 安全

> 范围：一个**个人**使用的自托管程序，没有用户系统、没有登录。安全靠三件事：**后端不对外**、**Host/Origin 守卫**、**前面加 Cloudflare Access 这样的身份验证**。
> 审计依据：[audit/security.md](history/audit/security.md)。测试：`backend/tests/`（对着已提交的旧版快照跑：172 份黄金响应、解析夹具、id 向量；v0.5 的路由在 `v05_banks_records_settings.rs` 里）和 `tests/integration/nginx_check.py`。迁移时还用一批 340 项的恶意请求探针把旧版和新版逐项比较过（0 项未解释的差异，51 项登记为有意加固），这套探针随旧 Python 代码一起移除了。

## 1. 威胁模型

| 谁 | 能做什么 | 我们的防线 |
|---|---|---|
| 同一网络里的别的机器 | 直接连端口 | 默认只绑定 `127.0.0.1`；要开放必须自己写 `XEXAM_BIND`；**程序自己没有登录**，开放后能连上的人就能查看题库和记录、导入、删除、修改设置 |
| 恶意网页（在你的浏览器里） | 让浏览器向 `localhost:8765` 发写请求或删除请求（CSRF） | 写请求检查 `Origin` 主机名必须和 `Host` 一致或在白名单里；JSON 接口要求 `Content-Type: application/json`（跨站表单发不出）。删除只能通过这些 POST 接口，GET 删不了任何东西 |
| DNS 重绑定 | 让恶意域名解析到 127.0.0.1，读你的数据 | 检查 `Host` 白名单；GET 也检查 |
| 恶意 / 损坏的 .xlsx | 压缩炸弹、XML 实体展开、畸形结构拖垮服务 | 见 §3 |
| 代理链上的伪造头 | `X-Forwarded-Host` 之类 | 后端**只读真实的 `Host` 头**；Nginx 把所有转发头清空 |
| 供应链 | 依赖里的漏洞 | 依赖锁定（`Cargo.lock`、`package-lock.json`）；镜像里没有包管理器以外的多余内容；见 §7 |

**不在范围内**：多用户隔离、登录、审计日志、加密存储。要从互联网访问，请在前面放 Cloudflare Access（或同类）。

## 2. 网络

```mermaid
flowchart LR
    I(("互联网")) -.->|"只能经 Cloudflare Access"| N["nginx :8080<br/>（宿主机 XEXAM_BIND）"]
    L(("本机 / 局域网")) -->|"XEXAM_BIND"| N
    N --> B["backend :8765<br/>（私有网络，无 ports）"]
    B -. "internal: true<br/>不能访问外网" .-> X(("✗"))
```

- 后端**永远不发布端口**。`docker/docker-compose.yml` 里 `backend` 没有 `ports:`，网络 `internal: true`；`tests/integration/stack_check.py` 断言「从宿主机连不到后端」。
- `XEXAM_BIND` 必须写成 `IP:端口`。只写端口号（如 `8766`）= 绑定所有网卡，是常见的误操作，文档里反复提醒。
- Linux 上主机防火墙（ufw / firewalld）**拦不住** Docker 发布的端口；限制来源要在 `DOCKER-USER` 链里加规则，或把 `XEXAM_BIND` 写成具体的局域网 IP。

## 3. 上传（.xlsx）

下面的步骤按顺序执行。任何一步失败都返回 400/413 和固定的中文说明，**不写库**：

1. 请求体大小：流式计数，**20 MiB**，按真实字节，不信任 `Content-Length`；Nginx 在前面还有一道 `client_max_body_size 20m`（返回同样文案的 JSON）。
2. 按**内容**判断：魔数 `PK`、ZIP 结构、含工作簿。旧版 `.xls`、空文件、改了扩展名的文件各有说明。
3. ZIP：条目数 ≤ 10,000；**重复的条目名拒绝**（避免「检查的是一个、读的是另一个」）；解压后总大小有上限；用 `take(limit)` 有界读取，声明的大小不可信。
4. XML：**任何 DOCTYPE / 实体声明一律拒绝**，不管扩展名、不管编码（UTF-16 也拒绝）；深度 ≤ 64、节点 ≤ 100 万（100 × 10,000）。
5. 解析：有协作式的截止时间（默认 30 秒）——解析器每处理一批行 / XML 事件就看一次时钟，到了**自己停下**，超时后不会继续占着 CPU 和内存；同时最多 2 个解析在跑（`PARSE_CONCURRENCY`），多出来的排队，排队时间也算在时限里。
   另外还有内存上限：同一个共享字符串被很多单元格引用时只存一份（不深拷贝），所有 A/B/C 单元格引用的字节总数不超过 64 MB；一个 XML 元素最多 256 个属性。
6. 预览放进内存里**唯一的待确认槽位**，30 分钟过期；确认码 128 位随机，一次性，常量时间比较；错误的确认码不会消耗槽位里的内容。
7. 确认时在**一个事务**里写入；这份文件里没有可用题时拒绝导入（防止把题库清空）。

旧版只扫描小写 `.xml` / `.rels` 的 ASCII 字节，UTF-16 或别的扩展名可以绕过——这是新版加固的地方（登记在 [api.md](api.md) 的差异表里）。

## 4. Host 与 Origin

| 检查 | 对象 | 规则 |
|---|---|---|
| Host | 所有请求 | 在白名单里：默认 `localhost`、`127.0.0.1`、`::1`；`XEXAM_ALLOWED_HOSTS` 追加；`XEXAM_ALLOW_ANY_HOST=1` 关闭检查 |
| Origin | 写请求 | 没带 → 放行（命令行工具不是跨站请求）；带了 → **主机名**必须和 `Host` 的主机名一致，或在 `XEXAM_ALLOWED_ORIGINS` / 白名单里 |

要点：

- **只比较主机名**，不比协议和端口：经 HTTPS 域名访问时 Origin 是 `https://example.com`，Host 是 `example.com`，不会被误拦。
- 守卫读的是真实的 `Host` 头。**不读** `X-Forwarded-Host` / `Forwarded`——否则等于把守卫交给了客户端。Nginx 用 `proxy_set_header Host $http_host` 原样转发，并清空其余转发头。
- 被拒绝时响应里写明「你的 Host / Origin 是什么、该把哪个值加进哪个选项」，日志里也有一行。
- `XEXAM_ALLOW_ANY_HOST=1` 只在**前面已经有身份验证**时使用；它不关闭 Origin 检查。

## 5. 题库、记录与设置（v0.5）

v0.5 新增了四个 GET 路由（`/api/banks`、`/api/records`、`/api/record`、`/api/settings`）、四个 POST 路由（`/api/banks/delete`、`/api/records/delete`、`/api/settings`、`/api/settings/reset`）和上传请求头 `X-Bank-Target`。这些都不需要新的防护手段，规则如下：

- **同样的守卫。** 新路由和旧路由走同一个分发函数，检查顺序也一样：所有请求都查 Host；POST 还要查 Origin、`Content-Type: application/json`、1,000,000 字节的请求体上限，以及「请求体必须是 JSON 对象」。Nginx 对它们和其他 `/api/` 路径一视同仁（方法、限流、超时）。`backend/tests/v05_banks_records_settings.rs` 里的 `new_routes_go_through_the_same_guards` 对每个新路由都检查了这一点。
- **请求格式很严格。** `id` 必须是整数（布尔值不算），`withRecords`、`withStats` 这类开关必须是布尔值，`all` 必须正好是 `true`。`GET /api/record` 只接受恰好一个 `id`，由 1–18 位 ASCII 数字组成。其他情况一律 400，什么都不改。id 只会以参数的形式传给 SQLite。
- **设置的校验。** 只读取四个已知的键。`theme` 必须是 `system`、`light` 或 `dark`；`uiLang` 是 `en`、`zh` 或 `ja`；`questionLang` 是 `ja`、`zh` 或 `en`；`autoAdvance` 是布尔值。只要有一个值不合法 → 400，什么都不写；不认识的键直接忽略。读回设置时，库里存着的值如果不合法，就忽略它、改用默认值。前端在应用这些值之前会再检查一遍，而且它们只会进入 `data-theme` 和 `lang` 属性，从不进入 HTML。
- **`X-Bank-Target`。** 在 Host、Origin、Content-Type 检查和大小限制都通过之后才读取，并且在解析文件之前。接受的值：`new`，或由 1–18 位 ASCII 数字组成的题库 id（前后的空格忽略）。其他值 → 400。不存在的题库 id → 在解析之前就返回 404，不耗费解析时间。被拒绝的上传不会动待确认槽位。
- **删除数据都要明确操作。** GET 不会删除任何东西，也没有哪个操作会顺带删除数据。两个删除路由都是带 JSON 请求体的 POST，所以跨站表单发不出来（Content-Type），别的网站上的脚本会被 Origin 检查拦下。每次删除都在一个事务里完成。界面上每次删除都会弹出确认对话框，写明要删掉什么，并说明删除后无法恢复。删除题库时一并删除它的学习数据、删除全部记录时一并清空错题本，都是单独的复选框，默认不勾选。如果一次导入会让题目下架，确认前会再问一次。没有撤销，也没有回收站：要找回数据只能靠你自己的备份（[database.md](database.md) 第 5 节）。
- **迁移前的备份。** 迁移到 schema v2 之前，后端会把数据库复制到数据目录下的 `backups/` 里。它从不覆盖已有的文件；如果复制失败，它会拒绝启动，而不是在没有备份的情况下迁移。

## 6. HTTP 层

| 项 | 设置 |
|---|---|
| 响应头 | 后端的所有响应：`Cache-Control: no-store`、`X-Content-Type-Options: nosniff`。`index.html`：另有 `X-Frame-Options: DENY`、`Referrer-Policy: no-referrer`、`Permissions-Policy`（相机 / 麦克风 / 定位全关）、严格 CSP。`/assets/`（脚本 / 样式）和 Nginx 自己生成的错误 JSON 只带 `X-Content-Type-Options`（及 assets 的缓存头），因为 nginx 的 `add_header` 在 location 里一旦写了就不再继承上层；它们不是可被嵌入或执行的页面 |
| CSP | `default-src 'self'; img-src 'self' data:; style-src 'self'; script-src 'self'; base-uri 'none'; form-action 'self'; frame-ancestors 'none'`——没有内联脚本和样式 |
| HSTS | **不设**：TLS 在 Cloudflare 终止，源站是 HTTP |
| CORS | 不发 CORS 头（同源） |
| 方法 | `GET` / `HEAD` / `POST`；其他方法 Nginx 直接回 403（`limit_except`），到不了后端；后端自己对它们回 405 + `Allow` |
| 限流 | Nginx：`/api/` 10 请求/秒（突发 30）；`/api/upload` 1 请求/秒（突发 5）；超限 429 |
| 超时 | 头 15 秒、体 30 秒（防慢速攻击）；上游连接 5 秒、读取 90 秒 |
| 隐藏文件 | `/.xxx` 一律 404；不列目录 |
| `Server` 头 | 后端不发；Nginx `server_tokens off` |

## 7. 容器

- 两个容器都以**非 root** 运行（后端 uid 10001），`cap_drop: ALL`，`no-new-privileges`，根文件系统 `read_only`，只有 `/tmp`（tmpfs）和后端的 `/data`（命名卷，`backups/` 也在里面）可写。
- **镜像里没有机密**：没有 `.env`、没有数据库、没有真实题库（`.dockerignore` 排除）。
- 后端镜像是静态链接（musl）的单个可执行文件 + tzdata，没有 shell 以外的工具链。
- `docker compose down -v` 会删除数据卷。文档里明确提示不要加 `-v`。

## 8. 日志与隐私

- 日志不记确认码、不记上传文件的内容、不记题目内容。
- 被拒绝的请求记 Host / Origin 与原因，供排查。
- 题库是用户自己的 Excel，**仓库里没有**（`.gitignore` 排除 `*.xlsx` 等，仅 `tests/baseline/fixtures/` 里虚构的测试夹具除外）。
- `backups/` 里的文件是数据库的完整副本。它们和数据库在同一个数据卷里，备份数据卷时也会一起备份进去；请像对待数据库本身一样对待它们。程序从不删除它们。
- 浏览器在 `localStorage` 里只存两样小东西：正在进行的考试（题目 id、答案、「稍后再看」标记、用时；不含题目文字），以及一份设置副本（页面首次绘制时用）。在公用电脑上，用完后请清除这个网站的数据。

## 9. 已知的限制（如实写出）

- **所有路由都没有登录。** 开放端口 = 任何能连上的人都能查看题库和考试记录、导入文件、删除题库和记录、修改设置。v0.5 的路由也没有自己的身份验证。要从外部访问，请在前面加身份验证（Cloudflare Access 或同类）。
- **没有账号。** 存在数据库里的设置对使用这个实例的所有人都生效。
- **服务端信任 `correct`** 和存下来的 `answer`，也不检查 id 是否在题库里（与旧版一致）：判分在浏览器里，你对自己的成绩作弊只会骗到自己。`answer` 只是存下来供回顾，从不用于判分或统计。
- **单个待确认槽位**：并发上传会互相替换（单人使用，不引入会话概念）。
- Cloudflare 路径只做了**本机模拟**：浏览器级的模拟（真实 Chrome + HTTPS 反向代理）是对旧 Python 版做的；对新栈只做了请求头级别的检查（经 Nginx 后 Host / Origin 守卫仍生效）。没有用真正的 Cloudflare 测过。
- iPhone Safari 没有在真机上测过；构建目标是 iOS 15.4+。
- 新镜像的 arm64 版本：CI 里在 QEMU 下**构建并启动**过（`/api/info` 报 `aarch64`），但没有在真正的 ARM 硬件上跑过。aarch64 的后端产物不带 Cortex-A53 勘误 #843419 的链接器规避（见 [deployment.md](deployment.md)）。

## 10. 漏洞报告

这是个人项目的私有仓库；发现问题请直接联系仓库所有者。
