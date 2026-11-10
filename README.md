# Xexam

在浏览器里做题的模拟考程序。题目来自你自己的 Excel，上传一次就存进 SQLite；做完题**交卷后才计分**，并列出每道错题的答案、解析和记忆口诀。手机（iPhone）和电脑都能用。

- **v0.3 的新架构**：**Rust**（Axum）后端 + **TypeScript**（Vite，原生 DOM）前端 + **Nginx** 反向代理，Docker Compose 两个容器；只有 Nginx 对外，后端在私有网络里。详见 [docs/architecture.md](docs/architecture.md)。
- 数据库**没有变**：仍是一个 SQLite 文件 `xexam.db`，放在 **Docker 命名卷** `xexam_data` 里，更新或删除容器都不会丢；新旧版本读写同一个文件，随时能回滚（[docs/migration.md](docs/migration.md)）。
- 日语 / 中文随时对照切换；错题本；成绩历史。
- 仓库里**没有题库**：题库是你自己的 Excel，不会被提交（`.gitignore` 已排除）。
- 旧的 Python 实现（v0.2.x）完整保留：标签 `python-v0.2`，镜像 `ghcr.io/yumin-space/xexam:v0.2.2`。

| | 新架构（v0.3，本分支） | 已发布的旧版（v0.2.x，Python） |
|---|---|---|
| 版本 | **v0.3**（见 [更新日志](#更新日志)） | v0.2.2（[Releases](../../releases) 页） |
| 怎么运行 | `docker compose -f docker-compose.modern.yml up -d --build`（从源码构建，镜像**还没有发布**） | `docker compose up -d`（拉取 `ghcr.io/yumin-space/xexam`，`linux/amd64` + `linux/arm64`） |
| 端口 | `8765`（Nginx，默认只绑定本机） | `8765`（默认只绑定本机） |

> ⚠️ 这个分支（`modernize/rust-ts`）里新旧两套代码并存：根目录的 `server.py`、`app/`、`web/` 是旧的 Python 实现；新实现在 `backend/`、`frontend/`、`nginx/`。把新实现换成默认、把 Python 移到 `legacy/python/`，以及新镜像的发布流水线，是合并时另做的一步（见 [docs/migration.md](docs/migration.md) 的「还没做的事」）。**在那之前不要把这个分支合并进 `main`**：`main` 上的流水线会按旧方式发布镜像。

文档：[架构](docs/architecture.md) · [API](docs/api.md)（[OpenAPI](docs/openapi.yaml)）· [数据库](docs/database.md) · [安全](docs/security.md) · [部署](docs/deployment.md) · [迁移与回滚](docs/migration.md) · [开发](docs/development.md) · [迁移计划](docs/migration-plan.md)

---

## 目录

- [截图](#截图)
- [架构](#架构)
- [快速开始（安装部署）](#快速开始安装部署)
- [使用方法](#使用方法)
- [Excel 题库格式](#excel-题库格式)
- [数据、备份和迁移](#数据备份和迁移)
- [更新日志](#更新日志)
- [自动构建与发布](#自动构建与发布)
- [不用 Docker](#不用-docker)
- [让手机访问](#让手机访问)
- [通过 Cloudflare Tunnel 从互联网访问](#通过-cloudflare-tunnel-从互联网访问)
- [目录结构](#目录结构)
- [安全说明](#安全说明)

---

## 截图

截图用的是**虚构的演示题库**（5 道日语小题），不是任何真实考试内容。左边是电脑，右边是 iPhone 宽度（390 px）。

### 首页：选范围，开始考试

首页能看到题库有几个工作表、错题本里有几道待巩固的题，以及最近的成绩。

<table>
<tr>
<td width="62%"><img src="docs/screenshots/desktop-1-home.png" alt="首页（电脑）"></td>
<td width="38%"><img src="docs/screenshots/iphone-1-home.png" alt="首页（iPhone）"></td>
</tr>
</table>

### 答题：右上角切换日语 / 中文，右侧是答题卡

答题时**不显示对错、不显示分数**。答题卡上：实心 = 已答，半实心 = 热点题只答了一部分，空心 = 未答，黄色小方块 = 已标记。iPhone 上「上一题 / 下一题」固定在屏幕底部，长题目不用滑到底才能翻页。

<table>
<tr>
<td width="62%"><img src="docs/screenshots/desktop-2-quiz.png" alt="答题（电脑）"></td>
<td width="38%"><img src="docs/screenshots/iphone-2-quiz.png" alt="答题（iPhone）"></td>
</tr>
</table>

点「中文」切换成 Excel 里 B 列的翻译。切换只换文字，已选的答案、选项字母、计时都不变，判分也不变。

<table>
<tr>
<td width="62%"><img src="docs/screenshots/desktop-2b-quiz-zh.png" alt="切换成中文（电脑）"></td>
<td width="38%"><img src="docs/screenshots/iphone-2b-quiz-zh.png" alt="切换成中文（iPhone）"></td>
</tr>
</table>

### 交卷：先看分数，再看每题对错

<table>
<tr>
<td width="62%"><img src="docs/screenshots/desktop-3-result.png" alt="成绩（电脑）"></td>
<td width="38%"><img src="docs/screenshots/iphone-3-result.png" alt="成绩（iPhone）"></td>
</tr>
</table>

### 错题解析：你的答案、正确答案、解析、记忆口诀

<table>
<tr>
<td width="62%"><img src="docs/screenshots/desktop-4-review.png" alt="错题解析（电脑）"></td>
<td width="38%"><img src="docs/screenshots/iphone-4-review.png" alt="错题解析（iPhone）"></td>
</tr>
</table>

### 题库管理：上传 Excel，先预览再导入

上传后会先显示「新增几题、更新几题、恢复几题、下架几题、没变化几题、读不出来几题」。确认前不会写入任何东西。

<table>
<tr>
<td width="62%"><img src="docs/screenshots/desktop-5-admin.png" alt="题库管理（电脑）"></td>
<td width="38%"><img src="docs/screenshots/iphone-5-admin.png" alt="题库管理（iPhone）"></td>
</tr>
</table>

---

## 架构

下面的图都是 [Mermaid](https://mermaid.js.org/) 写的，GitHub 上直接显示成图。图里的名字（文件、接口、数据表）都是代码里真实存在的。

### v0.3：Nginx + Rust 后端 + SQLite

```mermaid
flowchart TB
    U["浏览器"] -->|"HTTP / HTTPS"| N
    subgraph 公开["对外暴露：只有 Nginx"]
        N["nginx（xexam-web）<br/>前端静态文件 · /api/ 反向代理<br/>20 MB 上限 · 安全头 · 限流"]
    end
    subgraph 内网["Docker 私有网络（不发布端口）"]
        B["backend（xexam-backend，Rust / Axum）<br/>REST API · XLSX 解析 · 导入流程 · Host/Origin 守卫"]
    end
    N -->|"/api/*  Host 原样透传"| B
    B --> D[("SQLite<br/>命名卷 xexam_data → /data/xexam.db")]
    CF["Cloudflare Tunnel + Access（自己配置）"] -.-> N
```

浏览器只通过 [REST 接口](docs/api.md) 和后端说话；前端里没有数据库访问。判分仍在浏览器里（服务端只收 `{题目 id, 对错}`）。目录、模块、数据流见 [docs/architecture.md](docs/architecture.md)。

**下面的 1–7 节是 v0.2.x（Python）的详细图。** 数据流 A/B、数据库、页面跳转对 v0.3 同样适用（行为被固定下来，新旧逐项对照过）；「容器里面」和「CI/CD」两节只描述旧版。

### 1. 总览：谁和谁连着，数据怎么走（v0.2.x）

一句话：**浏览器**只和**一个容器**说话；容器里的 Python 服务读写 **SQLite**；SQLite 文件放在 **Docker 卷**里；题库从你本机的 **Excel** 上传进来。在家里可以直接连容器；在外面经过 **Cloudflare**（你的域名 + Access 验证 + 隧道）连进来，路由器不用开任何端口。

```mermaid
flowchart TB
    subgraph USER["你的设备（电脑 / iPhone，在家里或外面）"]
        direction TB
        XLSX[("📄 Excel 题库<br/>A 题目 · B 中文翻译<br/>C 答案解析口诀<br/>留在你自己的电脑上")]
        LS[("浏览器本地存储 localStorage<br/>· 做到一半的考试（暂存）<br/>· 选的语言")]
        BROWSER["🌐 浏览器<br/>单页前端 web/app.js<br/>负责：显示、答题、判分、<br/>日语/中文切换"]
        XLSX -.->|"你选择文件"| BROWSER
        LS <-.-> BROWSER
    end

    subgraph CF["Cloudflare（只有从互联网访问时才经过）"]
        direction TB
        EDGE["🌍 你的域名<br/>xexam.你的域名.com<br/>HTTPS"]
        ACCESS["🔐 Cloudflare Access<br/>你设的身份验证"]
        EDGE --- ACCESS
    end

    subgraph LAN["你的局域网"]
        direction TB
        CFD["🚇 cloudflared<br/>隧道客户端<br/>可以在局域网里<br/>任何一台机器上"]
        subgraph HOST["Docker 主机（x86 或 ARM）"]
            direction TB
            subgraph CONTAINER["容器 xexam（非 root 用户 uid 10001）"]
                direction TB
                SERVER["⚙️ server.py<br/>Xexam 的 HTTP 服务 :8765<br/>负责：接口、上传检查、<br/>导入预览/确认<br/>检查 Host（白名单<br/>XEXAM_ALLOWED_HOSTS）<br/>和写请求的 Origin（可另加<br/>XEXAM_ALLOWED_ORIGINS）"]
                PARSER["📖 app/xlsx_parser.py<br/>负责：读 .xlsx，<br/>解析成题目"]
                DB["🗄️ app/db.py<br/>负责：SQLite 读写、<br/>导入计划、错题统计"]
                SERVER --> PARSER
                SERVER --> DB
            end
            VOL[("💾 Docker 卷 xexam_data<br/>挂载在容器的 /data<br/>题库、成绩、错题记录<br/>都在一个文件 xexam.db 里")]
            DB <-->|"读写"| VOL
        end
        CFD -->|"HTTP 到 主机IP:8765"| SERVER
    end

    BROWSER -->|"B. 在外面：<br/>https://你的域名"| EDGE
    ACCESS -.->|"隧道（出站连接，<br/>不用开路由器端口）"| CFD
    BROWSER -->|"A. 在家里：直接访问<br/>http://主机IP:8765<br/>（别的设备连要先设<br/>XEXAM_BIND 和<br/>XEXAM_ALLOWED_HOSTS）"| SERVER
    BROWSER -->|"① 上传 Excel · ③ 确认导入<br/>⑤ 交卷成绩"| SERVER
    SERVER -->|"② 预览 / ④ 题库和成绩"| BROWSER

    classDef user fill:#E7EEF9,stroke:#2D56A6,color:#1B2A47
    classDef app fill:#EAF6EF,stroke:#1F7650,color:#1B2A47
    classDef store fill:#FBF3D7,stroke:#B59A1F,color:#1B2A47
    classDef cf fill:#FDEBD8,stroke:#D9822B,color:#1B2A47
    class XLSX,BROWSER,LS user
    class SERVER,PARSER,DB app
    class VOL store
    class ACCESS,EDGE,CFD cf
```

**要点**

- 判分发生在**浏览器**里（交卷时），服务端只负责保存结果和错题统计。
- 服务端**从不修改**你的 Excel：Excel 只是上传的原料，上传后数据就在 SQLite 里了，之后不再依赖这份 Excel。
- 浏览器和容器之间**只有 HTTP**，没有别的连接。经 Cloudflare 访问时，HTTPS 在 Cloudflare 那一端结束，隧道把请求以普通 HTTP 转给容器。
- 程序**自己没有登录**。从互联网访问时，身份验证靠 Cloudflare Access；见 [通过 Cloudflare Tunnel 从互联网访问](#通过-cloudflare-tunnel-从互联网访问)。

### 2. 容器里面：每个文件做什么

```mermaid
flowchart TB
    REQ(["浏览器发来的请求"]) --> GUARD

    subgraph SERVER["server.py：Xexam 的 HTTP 服务（Python 标准库 http.server）"]
        direction TB
        GUARD{{"安全检查（每个请求先过）<br/>Host：本机，或<br/>XEXAM_ALLOWED_HOSTS<br/>里的域名/IP<br/>写请求若带 Origin：<br/>和 Host 同一个主机，<br/>或在<br/>XEXAM_ALLOWED_ORIGINS 里<br/>（没带 Origin 的，例如命令行，放行）"}}
        REJECT["返回 403<br/>并说明是哪一条<br/>没通过、怎么放行"]
        subgraph ROUTES["按路径分发"]
            direction TB
            UPLOAD["POST /api/upload<br/>超过 20 MB：读取时就拒绝<br/>check_xlsx：检查是不是<br/>正常的 .xlsx<br/>拒绝：空文件 · 旧 .xls<br/>非 .xlsx · 损坏<br/>XML 实体 · 解压后过大"]
            PENDING["POST /api/import/confirm<br/>· /api/import/cancel<br/>Pending：暂存解析好的题目<br/>只留最近一次 · 30 分钟有效<br/>一次性校验码"]
            READ["GET /api/bank<br/>/api/progress<br/>/api/info · /api/imports<br/>读取接口"]
            WRITE["POST /api/attempt<br/>/api/reset<br/>写入接口"]
            STATIC["GET / 及 web/ 下的文件<br/>静态文件：从 web/ 读取<br/>不允许 .. 穿越，<br/>也读不到数据库"]
            UPLOAD ~~~ PENDING
            STATIC ~~~ READ
            READ ~~~ WRITE
        end
        GUARD -->|"不通过"| REJECT
        GUARD -->|"通过"| ROUTES
        REJECT ~~~ ROUTES
    end

    UPLOAD --> PARSER
    subgraph PARSE["app/xlsx_parser.py"]
        direction TB
        PARSER["parse_workbook<br/>zipfile + ElementTree<br/>读 .xlsx"] --> Q["parse_question<br/>识别：单选 · 多选 · 热点题<br/>取出：答案 · 解析 · 口诀"]
        Q --> T["parse_translation<br/>把 B 列拆成和日语<br/>一样的结构；拆不对<br/>就不提供中文切换"]
    end

    PARSER -->|"题目列表<br/>+ 读不出来的原因"| PENDING
    UPLOAD -->|"plan_import<br/>只算不写"| STORE
    PENDING -->|"确认后 apply_import<br/>一个事务"| STORE
    READ --> STORE
    WRITE --> STORE

    subgraph DBM["app/db.py：Store"]
        STORE["SQLite 读写<br/>journal_mode=DELETE<br/>synchronous=FULL"]
    end

    STORE --> FILE[("/data/xexam.db<br/>Docker 卷 xexam_data")]
    STATIC --> WEB["web/<br/>index.html · app.js<br/>style.css · icon.svg<br/>apple-touch-icon.png"]
    T ~~~ WEB
    classDef ok fill:#EAF6EF,stroke:#1F7650,color:#1B2A47
    classDef bad fill:#F9E3E0,stroke:#B2312A,color:#1B2A47
    classDef store fill:#FBF3D7,stroke:#B59A1F,color:#1B2A47
    class UPLOAD,PENDING,READ,WRITE,STATIC,PARSER,Q,T,STORE ok
    class REJECT bad
    class FILE store
```

> 图里没画的一个接口：`POST /api/shutdown`。只有用 `--allow-shutdown` 启动（`start.bat`）时才可用，Docker 里一律返回 403。

### 3. 数据流 A：更新题库（上传 → 预览 → 确认）

这是你每次改了 Excel 之后要走的流程。**预览阶段数据库一个字节都没动**，所以传错文件也不怕。

```mermaid
sequenceDiagram
    autonumber
    actor U as 你
    participant B as 浏览器<br/>web/app.js
    participant S as server.py<br/>HTTP 服务
    participant P as xlsx_parser<br/>解析 Excel
    participant D as db.Store<br/>app/db.py
    participant F as xexam.db<br/>卷 xexam_data

    U->>B: 选择 / 拖入<br/>Excel（.xlsx）
    B->>S: POST /api/upload<br/>文件内容 + 文件名
    S->>S: check_xlsx：是否正常的 .xlsx、<br/>解压后大小、有无 XML 实体声明
    alt 文件不合格 / 读不了
        S-->>B: 400（太大则 413）<br/>+ 中文原因<br/>如「这是旧版 .xls」
        B-->>U: 显示错误原因<br/>（数据库没动）
    else 合格
        S->>P: parse_workbook<br/>内存里解析，不写盘
        P-->>S: 题目列表 +<br/>读不出来的题<br/>工作表 / 行号 / 原因
        S->>D: plan_import：只读，对比库里现有的题
        D->>F: SELECT questions<br/>有下架时再查 stats
        D-->>S: 数量：新增 / 更新 / 恢复上架 /<br/>下架 / 没变化 / 读不出来
        S->>S: Pending.put：暂存解析结果<br/>（只留最近一次），<br/>生成一次性校验码
        S-->>B: 预览结果 + 校验码
        B-->>U: 显示「检查结果」<br/>（还没有写入）
    end
    U->>B: 点「确认导入」
    alt 有题要下架
        B-->>U: 弹窗：这次会<br/>下架 N 道题
        U->>B: 点「继续导入」<br/>（点「回去检查」<br/>则中止）
    end
    B->>S: POST<br/>/api/import/confirm<br/>校验码
    S->>S: Pending.take：校验码对、<br/>没过 30 分钟才取出，用过即作废
    S->>D: apply_import：先重新比对一次
    D->>F: BEGIN<br/>写入 / 更新 / 恢复<br/>下架 · 记导入记录<br/>COMMIT
    Note over D,F: 一个事务：<br/>中途出错整体回滚，<br/>不留半截数据
    D-->>S: 导入结果（各项数量）
    S-->>B: 导入结果 + 最新题库
    B-->>U: 显示「已导入」
```

**导入的规则**（上传的文件被当成**完整的题库**）

| 情况 | 结果 |
|---|---|
| 文件里新出现的题 | 新增 |
| 文件里改过的题（选项、答案、解析、翻译） | 更新，答题记录不受影响 |
| 库里有、文件里找不到的题 | **下架**（不再出现在考试里），答题记录保留；导入前会弹窗提醒 |
| 下架的题以后又出现在文件里 | 自动**恢复上架**，答题记录和错题本也回来 |
| 改了某道题的**题干** | 被当成一道**新题**（旧题下架，旧题的答题记录不会带到新题上） |
| 整份文件一道可用的题都没有 | 拒绝导入，防止把题库清空 |

> 题目的身份由「工作表名 + 题干 + 第一个选项的内容指纹」决定，**与行号无关**：在 Excel 里挪动行的顺序，答题记录跟着题走。

### 4. 数据流 B：做一场考试

```mermaid
sequenceDiagram
    autonumber
    actor U as 你
    participant B as 浏览器（app.js）
    participant LS as localStorage
    participant S as server.py
    participant F as xexam.db（卷）

    B->>S: GET /api/bank、/api/progress、/api/info
    S->>F: 读取上架的题 + 成绩 + 错题统计
    S-->>B: 题库、学习记录、版本号
    U->>B: 选工作表 / 出题范围 / 题数 → 开始考试
    Note over B: 从「全部题目」或「错题本」里抽题，可打乱顺序
    loop 每一题
        U->>B: 选答案 / 切换日语中文 / 标记
        B->>LS: 暂存进度（答案、标记、用时）
        Note over B: 答题时不显示对错
    end
    opt 中途退出
        U->>B: 点「暂存退出」
        Note over B,LS: 下次回首页会出现「继续作答」（暂存在这个浏览器里）
    end
    U->>B: 点「交卷」
    B->>B: 判分：单选/多选 = 选的集合必须完全一致；热点题 = 每个小项都对
    B->>S: POST /api/attempt（每题只发 {题目 id, 对/错}）
    S->>F: 事务：更新每题的累计 / 连对次数 → 写一条成绩 → 只保留最近 60 场
    S-->>B: 最新学习记录
    B->>LS: 清掉暂存
    B-->>U: 成绩页：百分比 + 每题对错 + 错题的答案/解析/口诀
```

> 提交的只有「题目 id + 对错」，不会上传你选了什么，也不会上传题目内容。

### 5. 数据库：5 张表

整个数据库就是 `/data/xexam.db` 这一个文件。

```mermaid
%%{init: {"er": {"nodeSpacing": 40, "rankSpacing": 40}}}%%
erDiagram
    direction LR
    questions {
        TEXT id PK "指纹：工作表+题干+首选项/首小项"
        TEXT sheet "工作表名（与 active 建有索引）"
        INTEGER row_no "Excel 里的行号（只记录，不影响 id）"
        INTEGER seq "题库里的顺序"
        TEXT data "整题 JSON：题干 选项 答案 解析 口诀 翻译"
        INTEGER active "1 上架 · 0 下架"
        TEXT added_at "首次入库时间"
        TEXT updated_at "最近一次被导入或下架的时间"
    }
    stats {
        TEXT question_id PK "= questions.id，下架后仍保留"
        INTEGER attempts "累计答过几次"
        INTEGER correct "答对几次"
        INTEGER wrong "答错几次"
        INTEGER streak "当前连续答对几次"
        INTEGER last "上一次：1 对 · 0 错（可空）"
        TEXT at "最近一次作答时间（可空）"
    }
    attempts {
        INTEGER id PK "自增，只留最近 60 场"
        TEXT at "交卷时间"
        TEXT sheets "考了哪些工作表（JSON）"
        TEXT mode "all 全部题目 · weak 错题本 · retry 重做错题"
        INTEGER total "题数"
        INTEGER correct "答对数"
        INTEGER percent "得分百分比"
        INTEGER seconds "用时（秒）"
    }
    imports {
        INTEGER id PK "自增"
        TEXT at "导入时间"
        TEXT filename "上传的文件名"
        INTEGER added "新增的题数"
        INTEGER updated "同 id 但内容变了的题数"
        INTEGER removed "这次没出现而下架的题数"
        INTEGER restored "下架后又出现而恢复的题数"
        INTEGER skipped "读不出来而跳过的行数"
        INTEGER total "导入后上架的题数"
    }
    meta {
        TEXT key PK "如 schema、legacy_migrated"
        TEXT value "版本号 / 迁移时间"
    }
    questions |o--o| stats : "按 id 对应<br/>无外键约束"
```

**为什么 `stats` 不随题目一起删：** 题目下架只是把 `active` 改成 0，`stats` 里的答题记录原样保留；以后这道题重新出现在 Excel 里，会自动恢复上架，错题本也跟着回来。

**错题本怎么算：** 一道题只要 `wrong > 0` 且 `streak < 2`（连续答对不到 2 次）就在错题本里；连续答对 2 次后自动移出。

### 6. 自动构建与发布（CI/CD）

每次你把代码推到 GitHub，GitHub Actions 自动做完**检查版本和更新日志 → 两种 CPU 各自构建并测试 → 都通过才发布镜像 → 在 Releases 页建一个同版本的 Release**。

```mermaid
flowchart TB
    DEV["你：改 VERSION，在 README 更新日志写一条<br/>然后 git push 到 main"] --> CHK

    subgraph WF["GitHub Actions：.github/workflows/docker.yml"]
        direction TB
        CHK["job 1：check<br/>VERSION 格式正确，且 README 更新日志最上面一条 = VERSION"]
        subgraph TEST["job 2：test（两种 CPU 各跑一遍）"]
            direction TB
            T1["amd64（x86）<br/>构建镜像 → scripts/smoke-test.sh"]
            T2["arm64（ARM，QEMU 模拟）<br/>构建镜像 → 同一份冒烟测试"]
        end
        PUB["job 3：publish<br/>构建 amd64+arm64 → 合成一个镜像清单 → 推送<br/>→ 回头核对：两种架构都在，latest 和版本号指向刚推送的镜像"]
        REL["job 4：release<br/>在 Releases 页建（或更新）v版本号<br/>说明 = README 更新日志里这个版本的一段"]
        STOP["❌ 任何一步失败：不发布<br/>latest 保持上一个能用的版本，Releases 页也不出现这个版本"]
        CHK --> T1
        CHK --> T2
        T1 -->|"两种架构都通过"| PUB
        T2 --> PUB
        PUB -->|"镜像已发布并核对"| REL
        TEST -.->|"失败"| STOP
    end

    PUB --> REG[("Packages：ghcr.io/yumin-space/xexam<br/>标签：latest · 版本号 · sha-xxxxxxx")]
    REL --> RELP[("Releases 页：v版本号 + 同名 git 标签")]
    REG -->|"docker compose pull<br/>x86 拿 amd64 · ARM 拿 arm64"| RUN["你的服务器 / 电脑 / 树莓派"]

    classDef ok fill:#EAF6EF,stroke:#1F7650,color:#1B2A47
    classDef bad fill:#F9E3E0,stroke:#B2312A,color:#1B2A47
    classDef store fill:#FBF3D7,stroke:#B59A1F,color:#1B2A47
    classDef user fill:#E7EEF9,stroke:#2D56A6,color:#1B2A47
    class CHK,T1,T2,PUB,REL ok
    class STOP bad
    class REG,RELP store
    class DEV,RUN user
```

**冒烟测试检查什么（每种架构都跑一遍）**

| 检查 | 目的 |
|---|---|
| 镜像的 CPU 架构 = 期望的架构 | 防止悄悄构建成了另一种架构 |
| 容器不是 root 用户 | 安全 |
| 首页能打开；全新的卷里题库为空 | 基本可用 |
| `/api/info` 里的版本号 = `VERSION` | 镜像里带的版本是对的 |
| 图标和页面文件能取到 | iPhone「添加到主屏幕」要用 |
| 陌生 `Host` 被拒绝（403） | 防 DNS 重绑定 |
| 数据库文件写进了卷 | 数据在卷里，不在容器里 |
| 重启后服务正常、健康检查变 `healthy` | 稳定 |
| 跨站 Origin 的 POST 被拒绝（403），同源的 POST 放行 | 防跨站操作，同时确认没有把正常使用拦住 |
| **删除容器后用同一个卷重建，数据还在** | 这正是「更新镜像不丢数据」的保证 |
| `docker stop` 在 9 秒内正常结束 | 程序响应停止信号，数据库干净关闭，不是等 10 秒被强杀 |

### 7. 页面之间怎么走

```mermaid
stateDiagram-v2
    [*] --> 首页
    首页 --> 题库管理: 点「题库管理 / 上传 Excel」
    题库管理 --> 首页: 返回首页 / 去考试
    首页 --> 答题: 开始考试 / 继续作答
    答题 --> 首页: 暂存退出（进度存在浏览器）
    答题 --> 成绩页: 交卷（弹窗确认）
    成绩页 --> 答题: 只重做错题 / 再考一次
    成绩页 --> 首页: 回首页
    首页 --> 首页: 清空学习记录（弹窗确认）
    答题 --> 答题: 上一题 / 下一题 / 点日语中文切换（已选答案不变）
```

答题页里单选、多选、热点题三种题型的界面略有不同（单选选一个、多选选几个、热点题每个小项各选一个），但都在同一个「答题」页面里切换，进度保存在浏览器里。

---

## 快速开始（安装部署）

### 需要什么

- 一台装了 **Docker** 的机器：Windows / macOS / Linux 都行，**x86 或 ARM 都行**（树莓派、云上的 ARM 服务器、苹果芯片的 Mac）。
- 一个浏览器。

### 方式 A：直接拉取现成的镜像（推荐）

只需要仓库里的 [`docker-compose.yml`](docker-compose.yml) 这**一个文件**，复制到任意机器的任意文件夹即可，不需要其他代码。

```bash
# 第一次：登录 GitHub 的容器仓库（镜像是私有的，所以要登录一次）
docker login ghcr.io -u Yumin-Space
# 密码处填一个有 read:packages 权限的 GitHub 令牌
# （GitHub → Settings → Developer settings → Personal access tokens）

# 在 docker-compose.yml 所在的文件夹里：
docker compose up -d
```

然后浏览器打开 <http://localhost:8765>，点「题库管理 / 上传 Excel」导入你的 Excel。

| 操作 | 命令 |
|---|---|
| 更新到最新版本 | `docker compose pull && docker compose up -d` |
| 固定在某个版本 | 把 `docker-compose.yml` 里的 `:latest` 改成 `:v0.2`，再 `docker compose up -d` |
| 停止 / 再启动 | `docker compose stop` / `docker compose start` |
| 看日志 | `docker compose logs -f` |
| 看现在是哪个版本、什么架构 | 浏览器打开 <http://localhost:8765/api/info> |

`docker-compose.yml` 的关键部分：

```yaml
name: xexam
services:
  xexam:
    image: ghcr.io/yumin-space/xexam:latest   # latest = 最新；也可以写 :v0.2 固定版本
    container_name: xexam
    restart: unless-stopped
    ports:
      - "${XEXAM_BIND:-127.0.0.1:8765}:8765"  # 默认只绑定本机；要给局域网里的 cloudflared 用，在 .env 里设 XEXAM_BIND=0.0.0.0:8765
    volumes:
      - data:/data                            # 题库和学习记录都在这个卷里
    environment:
      XEXAM_ALLOWED_HOSTS: "${XEXAM_ALLOWED_HOSTS:-}"   # 允许哪些域名/IP 访问；留空 = 只有本机
volumes:
  data:
```

可调的选项都通过 `docker-compose.yml` 旁边的 `.env` 文件设置（不需要改 compose 文件本身）：

| `.env` 里的名字 | 默认 | 作用 |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | 对外监听的地址和端口，必须写成 `IP:端口`。默认只有本机能连；`0.0.0.0:8765` = 局域网里别的机器也能连。⚠️ 只写端口号（如 `8766`）= 所有网卡都能连 |
| `XEXAM_ALLOWED_HOSTS` | 空 | 允许用哪些域名 / IP 访问，逗号分隔，例如 `xexam.example.com,192.168.1.20` |
| `XEXAM_ALLOWED_ORIGINS` | 空 | 反向代理把 Host 改写成源站地址时才需要：填你的公网域名 |
| `XEXAM_ALLOW_ANY_HOST` | `0` | 设 `1` = 不再检查 Host（只在前面已经有身份验证时用） |

### 方式 B：从源码构建（改代码时用）

需要完整的仓库代码：

```bash
git clone git@github.com:Yumin-Space/xexam.git
cd xexam
docker compose -f docker-compose.build.yml up -d --build
```

它和方式 A 用**同一个卷**（`xexam_data`），所以两种方式的数据是同一份；容器名也相同，所以不能同时运行，切换前先 `docker compose stop`。

### 镜像的版本标签

| 标签 | 含义 | 什么时候用 |
|---|---|---|
| `latest` | 最近一次成功发布的版本 | 想一直用最新版 |
| `v0.2`、`v0.3`… | 固定的某个版本（v0.2 是第一个发布的版本） | 想锁定版本、需要回退 |
| `sha-xxxxxxx` | 某一次提交 | 排查问题 |

---

## 使用方法

### 第一次：导入题库

1. 打开页面，题库是空的，点「去上传题库」（或右上角「题库管理 / 上传 Excel」）。
2. 选择你的 Excel（也可以拖进虚线框）。
3. 看**检查结果**：新增几题、哪几题读不出来（会写明工作表、行号和原因）。
4. 没问题就点「确认导入」。

### 以后更新题库

1. 在自己电脑上改 Excel：新增工作表、新增或修改题目都可以。
2. 保存后，把**整份** Excel 再上传一次。
3. 看检查结果，确认导入。规则见上面 [数据流 A](#3-数据流-a更新题库上传--预览--确认)。

### 考试

1. 首页勾选要考的工作表，选「全部题目」或「错题本」，设置题数，点「开始考试」。
2. 答题过程中**不显示对错、不显示分数**。可以随时跳题、用答题卡看进度、标记不确定的题。
3. 点「交卷」后才计分（答对的百分比），并列出每道错题的：你的答案、正确答案、解析（怎么选）、记忆口诀、中文翻译。
4. 单选、多选要**选对全部**才算对；热点题**每个小项都对**才算这题对。
5. 没做完可以「暂存退出」，下次回到首页「继续作答」（暂存放在浏览器里，换浏览器就没有了）。

### 日语 / 中文对照

题目右上角有「日语 / 中文」两个按钮。中文用的是 Excel 里 **B 列**的翻译。如果某道题 B 列没写翻译，或翻译里的选项 / 小项数量和 A 列对不上，这道题的「中文」按钮会**置灰**（不会显示错位的内容）。选项本身（`はい / いいえ`、`Apple` 这类答案原文）不翻译。

### 错题本

答错的题进入错题本，之后**连续 2 次答对**自动移出。首页选「错题本」只练这些题；交卷后也可以点「只重做错题」。

### 键盘（电脑）

`←` `→` 切换题目，`A`–`E` 选答案，`M` 标记。

### 在 iPhone 上使用

- 在 iPhone 的 Safari 里打开服务地址（需要先做 [让手机访问](#让手机访问)）。需要 **iOS 15.4 或更新**（更早的版本没有 `<dialog>`，页面会改用系统自带的确认框，样式略有不同）。
- 点分享按钮 →「添加到主屏幕」，之后像独立应用一样全屏打开，图标是答题卡样式的 Xexam。
- 所有按钮都不小于 44 px，适合手指点；刘海和底部横条的安全区已处理；「上一题 / 下一题」固定在屏幕底部。

---

## Excel 题库格式

每个工作表是一组题，一行一道题。

| 列 | 内容 |
|---|---|
| **A** | 题目。选择题：题干后空一行，选项写成 `A. 内容`，一行一个。热点题：第一行写 `ホットスポット`，小项写成 `1. 内容　はい / いいえ` |
| **B** | 中文翻译（可以不写）。要和 A 列保持同样的结构：选项也写成 `A. 内容`，热点题的小项写成 `1. 内容`，这样才能切换成中文 |
| **C** | 以 `【答案】` 开头，后面可以接 `【解析·怎么选】` 和 `【记忆口诀】` |

C 列的答案写法：

- 单选 / 多选：`【答案】C` 或 `【答案】C、E`
- 热点题：`【答案】1 是；2 否；3 是`。小项的答案可以写 `是/否`、`はい/いいえ`，或者直接写选项文字（比如 `Apple`）

一个例子（C 列）：

```
【答案】C、E

【解析·怎么选】先排除……再比较……

【记忆口诀】……
```

读不出来的题**不会被猜**：它们会在检查结果里列出原因（多半是 C 列没写 `【答案】…`，或答案字母不在选项里），不会悄悄判错。

---

## 数据、备份和迁移

数据保存在一个 **Docker 命名卷**里，不是你电脑上看得见的文件夹。

- 卷名是 `xexam_data`（项目名 `xexam` + 卷名 `data`），里面只有 `xexam.db`：题库、成绩、错题记录都在这一个文件里。
- 更新镜像、`docker compose down`、删除并重建容器，**数据都还在**。
- ⚠️ **不要运行 `docker compose down -v`**：`-v` 会把卷一起删掉，数据就没了。

### 备份和恢复

先停止程序再操作，数据库更安全。

```powershell
docker compose stop

# 备份：把卷打包成当前文件夹里的 xexam-data.tgz
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data.tgz -C /data .

docker compose start
```

```powershell
# 恢复（会覆盖卷里的同名文件）
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar xzf /backup/xexam-data.tgz -C /data
docker compose start
```

换电脑：在新电脑上登录 `ghcr.io`，放好 `docker-compose.yml`，先 `docker compose up -d` 建好卷，再按上面的方法恢复备份。

### 迁移旧版的学习记录

如果你有旧版留下的 `progress.json`（旧版把成绩放在这个文件里），在**还没做过任何考试**的新卷上，用一个临时容器把它放进卷里，再重启：

```powershell
docker compose up -d
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/src:ro" alpine sh -c "cp /src/progress.json /data/progress.json && chown 10001:10001 /data/progress.json"
docker compose start
```

重启时会自动把里面的成绩和错题记录迁移进数据库，**只迁移一次**，日志里会写「已把旧的学习记录迁移进数据库」（`docker compose logs` 能看到）。数据库里已经有学习记录时不会覆盖。

> 这里没有用 `docker cp`：它默认以 root 身份写入，文件属于 root，而容器里的程序是非 root 用户，可能读不了。上面的做法把文件的所有者设成了程序用户（uid 10001）。

---

## 更新日志

每次更新都要在这里写一条，**版本号和仓库根目录的 `VERSION` 文件一致**（自动构建会检查，不一致就不让构建）。最新的镜像同时有 `latest` 和对应的 `vX.Y` 标签。

### v0.3 — 2026-11-10

> 状态：在分支 `modernize/rust-ts` 上完成，**尚未合并到 `main`，镜像尚未发布**（`main` 上的流水线仍发布 v0.2.x 的 Python 镜像）。

**重写成 Rust + TypeScript + Nginx。数据库文件和 schema 没有变，API 向后兼容。**

- **后端改成 Rust**（Axum 0.8 + Tokio + rusqlite，工具链固定 1.99.1）：REST 接口、XLSX 解析、导入（预览不写库 → 一次性确认码 → 一个事务写入）、成绩与错题统计、Host/Origin 守卫、`xexam-backend import / healthcheck / migrate-progress` 子命令，行为与 Python 版一一对应（172 份旧版响应逐项对照，0 项未解释的差异；85 条题目 id 向量、49 个合成工作簿的解析输出全部一致；题目 id 与旧版逐字节相同，所以学习记录不会脱钩）。
- **前端改成 TypeScript**（strict）+ **Vite**，原生 DOM、没有框架：API 层、类型、页面、组件、状态、判分规则（纯函数 + 单元测试）分开；没有内联样式，CSP 可以很严。构建目标 `safari15`，iPhone 需要 iOS 15.4 以上。
- **Nginx 反向代理 + Docker Compose**：两个容器（`xexam-web`、`xexam-backend`），**只有 Nginx 对外**，后端在 `internal` 私有网络里不发布端口；非 root、`cap_drop: ALL`、只读根文件系统；Host 原样透传、`X-Forwarded-*` 清空；20 MB 上限、安全头、限流、超时；后端不可达时返回 JSON 502。新的 compose 文件是 `docker-compose.modern.yml`。
- **更严的上传检查（有意的差异）**：任何编码 / 扩展名里的 DOCTYPE 和实体声明都拒绝；重复的 ZIP 条目名拒绝；ZIP 条目数、XML 深度和节点数有上限；错误文案不再回显库的异常文本；不支持的 HTTP 方法回 405（以前是 501）。完整差异表见 [docs/api.md](docs/api.md)。
- **测试**：后端 130 个、前端 19 个；`tests/integration/` 里 Nginx 38 项、整栈 34 项、回滚演练 19 项；CI 新增 `modern.yml`（frontend / backend / stack，**从不发布**）。
- **回滚**：Python 实现完整保留（标签 `python-v0.2`，镜像 `ghcr.io/yumin-space/xexam:v0.2.2`）；新后端写过的库旧版仍能读写（`tests/integration/rollback_check.py` 用合成数据演练过往返）。步骤见 [docs/migration.md](docs/migration.md)。
- **新增文档**：`docs/` 下的架构、API（含 OpenAPI）、数据库、安全、部署、迁移、开发，以及 Phase 0 的审计、风险、迁移计划和测试策略。
- 判分仍在浏览器里（规格建议放服务端，这里有意不搬，理由见 [docs/migration-plan.md](docs/migration-plan.md)）。
- **没有验证的东西（如实写）**：真正的 Cloudflare 隧道、iPhone 真机、新镜像的 arm64 构建；没有用业主的真实数据库副本演练（请按 [docs/migration.md](docs/migration.md) 在自己的机器上做一遍）。

### v0.2.2 — 2026-11-10

- 新增 `tests/baseline/`：旧版 Python 的行为基线（172 份真实响应、50 个 XLSX 夹具及解析输出、85 条题目 id 向量、示例数据库、安全探针、Host/Origin 检查和隧道模拟脚本），后面的 Rust 重写要和它逐项对照。工具里不再写死任何本机路径，可用 `XEXAM_LEGACY_ROOT` 指定被测代码。
- 修正上一版更新日志里关于 v0.2 Release 的说法。

### v0.2.1 — 2026-11-10

- **每次构建成功后自动在 GitHub 的 Releases 页建一个 Release**（以前只推了镜像，Releases 页一直是空的）：流水线新增第 4 个 job `release`，在镜像发布并核对成功之后运行，说明取自本更新日志这个版本的那一段，并自动打 git 标签 `v<版本>`。
- v0.2 的 Release 手动补建了一次（对应 v0.2 的提交 `1ee64c8`，没有标成 Latest）；从 v0.2.1 起由流水线自动建。
- 新增 `scripts/release-notes.py`：从 README 抽出某个版本的更新日志；本机也能运行，用来预览 Release 页会显示什么。

### v0.2 — 2026-11-10

- **可以通过自己的域名从互联网访问**（Cloudflare Tunnel + Cloudflare Access）：
  - 允许的域名放进 `XEXAM_ALLOWED_HOSTS`；写请求的 `Origin` 改成只比较主机名，所以经 **HTTPS** 域名访问时上传、交卷不再被 403 拦下。
  - 代理改写了 `Host` 时，可以用 `XEXAM_ALLOWED_ORIGINS` 单独放行公网域名；前面已有身份验证时可设 `XEXAM_ALLOW_ANY_HOST=1`。
  - 被拒绝时会说明是哪一条没通过、该怎么放行；登录过期时页面会提示刷新重新登录。
- 端口绑定改成可配置（`XEXAM_BIND`），默认仍然只绑定本机。
- 收到 `docker stop` 的停止信号后正常退出（以前要等 10 秒被强杀），数据库干净关闭。
- CI：拆成「检查版本和更新日志 → 两种架构各自测试 → 发布」三步（v0.2.1 起再加第四步：建 Release）；冒烟测试挪进 `scripts/smoke-test.sh`，本机装了 Docker 也能直接跑；发布后按摘要核对镜像清单。
- `/api/info` 增加 `arch`（当前 CPU 架构）。
- 审计后的修复（每一条都有对应的实测）：
  - 服务端：提前拒绝的请求（403/415）先把请求体读掉再回复，浏览器看到的是真正的原因而不是「连不上服务」；被拒绝的请求在日志里留一行（`docker compose logs` 能看到）；畸形的 JSON（数字当列表、`Infinity`、过深嵌套、非 ASCII 的确认码）返回 400/409，而不是让连接断掉；POST 的 `Content-Type` 必须真的是 `application/json`；`XEXAM_ALLOWED_HOSTS` 用空格或分号分隔也能识别。
  - 数据库：`COMMIT` 失败时会回滚，不会让连接一直卡在事务里；格式不对的旧 `progress.json` 不再让程序起不来；数据目录里只有别的 `.db` 文件时，启动日志会提醒（不会改动那个文件）。
  - 页面：交卷时成绩提交成功才清掉暂存，失败会保留并提示「刷新页面重新登录后再交卷」；开始新考试前，如果有没交卷的考试会先确认；成绩页百分比与学习记录用同一种取整，不再差 1；提示条不再被底部翻页栏挡住；长英文串不再撑出横向滚动条；没有 `<dialog>` 的旧 Safari 改用系统确认框；长按字母键不会反复切换答案。
  - 镜像和 CI：健康检查不走代理并跟随 `XEXAM_PORT`；`.dockerignore` 改成任意层级生效；`.gitignore` 加了备份压缩包和 `.env.*`；只从 `main` 发布，`main` 上排队不取消；冒烟测试里容器内路径在 Windows 的 Git Bash 里也能跑。
  - 这版在本机用 Docker 真的构建了镜像（amd64），并用 CI 同一份冒烟测试跑通：非 root、Host/Origin 拦截、删除容器后数据还在、`docker stop` 1 秒。arm64 和发布仍要看 GitHub Actions 的结果。
- 名称统一成 **xexam**：环境变量 `XEXAM_DATA` / `XEXAM_HOST` / `XEXAM_PORT` / `XEXAM_ALLOWED_HOSTS` / `XEXAM_ALLOWED_ORIGINS` / `XEXAM_ALLOW_ANY_HOST`，数据库文件 `xexam.db`，浏览器里保存的键名 `xexam.*`；文档里也不再写死你自己的 Excel 文件名。
- ⚠️ **从 v0.1 升级时注意：** 数据库文件的名字变了，新版只读 `xexam.db`。如果你在更早的版本里已经导入过题库，请先停止程序，把数据目录里那个 `.db` 文件改名为 `xexam.db` 再启动，否则会看到空题库（旧文件不会被删，只是不会被读取）。浏览器里暂存的考试和语言选择用的键名也变了，升级前请先交卷，语言会恢复成日语。在自己改过的 compose 里设置过旧环境变量名的，要改成上面的 `XEXAM_*`，否则会被忽略。
- `docker-compose.yml` 去掉了 `pull_policy: always`：`docker compose up -d` 不再每次联网检查，更新请用 `docker compose pull && docker compose up -d`。
- `.env`（本机配置）加进 `.gitignore` 和 `.dockerignore`，不会被提交，也不会进镜像。

### v0.1 — 2026-11-10

第一个版本。

- 浏览器答题：单选、多选、热点题；**交卷后才计分**，错题显示答案、解析、记忆口诀。
- 日语 / 中文对照切换；答题卡；标记；暂存退出再继续。
- 错题本：答错的题连续答对 2 次后移出；成绩历史。
- 题库和学习记录改存 **SQLite**（Docker 命名卷 `xexam_data`），不再依赖 Excel。
- 管理页**上传 Excel 更新题库**：先预览（新增 / 更新 / 恢复 / 下架 / 读不出来），确认后才写入；下架的题保留答题记录，传回来自动恢复。
- 旧版的 `progress.json` 第一次启动时自动迁移。
- 支持 **iPhone**：触控区域不小于 44 px、安全区、底部固定翻页栏、可添加到主屏幕。
- Docker 镜像同时支持 **x86（amd64）和 ARM（arm64）**，由 GitHub Actions 构建并测试。
- ⚠️ v0.1 的「发布」步骤没有跑成，所以 `ghcr.io/yumin-space/xexam` 上**没有** `v0.1` 标签；镜像从 v0.2 起发布（`latest` / `v0.2`）。
- 应用名改为 **Xexam**。

<!-- 新版本写在这一行的上面，格式：### vX.Y — 日期 -->

---

## 自动构建与发布

[`.github/workflows/docker.yml`](.github/workflows/docker.yml)，流程图见上面 [CI/CD](#6-自动构建与发布cicd)。

- **触发：** 推送到 `main`，或在 Actions 页面手动运行；拉取请求只构建和测试，不发布。
- **每次更新必须：** 改 `VERSION`，并在本文件「更新日志」最上面写一条同版本的记录，否则检查步骤会失败。
- **两种架构各测一遍：** `amd64` 在 GitHub 的 x86 机器上原生跑；`arm64` 用 QEMU 模拟运行（比原生慢，但这个镜像很小）。
- **都通过才发布：** 发布成一个多架构镜像，并回头检查镜像清单里确实有 `amd64` 和 `arm64`。测试失败时 `latest` 仍是上一个能用的版本。
- **发布的标签：** `latest`、`v<VERSION>`、`sha-<提交>`。
- **Releases 页：** 镜像发布并核对成功之后，流水线会自动建（或更新）一个叫 `v<VERSION>` 的 Release，说明文字取自本文件「更新日志」里这个版本的那一段（[`scripts/release-notes.py`](scripts/release-notes.py)），并自动打上同名的 git 标签。镜像没发布成功，就不会有这个 Release。同一个版本重新跑只会更新说明，不会再建一个。
- **Releases 和镜像是两个地方：** 镜像在仓库的 **Packages** 里（ghcr.io），Release 在 **Releases** 页；两者现在由同一次构建一起产生。

镜像从私有仓库发布，默认也是私有的。想免登录拉取，到 GitHub 的包设置里把它改成公开。

---

## 不用 Docker

需要 Python 3.8 或更新版本。

```bash
python server.py                       # 数据放在 ./data，自动打开浏览器
python server.py --data D:\xexam-data  # 指定数据目录
python server.py --import 题库.xlsx    # 不启动服务，直接把 Excel 导入数据库
```

也可以用环境变量代替命令行参数：`XEXAM_DATA`（数据目录，等于 `--data`）、`XEXAM_HOST`（监听地址，等于 `--host`，默认 `127.0.0.1`）、`XEXAM_PORT`（端口，等于 `--port`，默认 `8765`）。Docker 镜像里它们已经设成 `/data`、`0.0.0.0`、`8765`。

Windows 上也可以双击 `start.bat`（会带上 `--allow-shutdown`，页面右下角有「关闭程序」按钮；Docker 里没有这个按钮，用 `docker compose stop`）。

## 让手机访问

**这个程序没有登录**，默认只绑定本机（`127.0.0.1`）。想让同一个家庭 Wi-Fi 里的 iPhone 访问，**只在你信任的网络里**这样做，不要在咖啡厅或公司网络里开：

1. 在 `docker-compose.yml` 旁边建 `.env` 文件，写两行：`XEXAM_BIND=0.0.0.0:8765` 和 `XEXAM_ALLOWED_HOSTS=你的电脑的局域网IP`（例如 `192.168.1.20`）。
2. `docker compose up -d`，iPhone 的 Safari 打开 `http://你的电脑IP:8765`。

要在外面（不在家里的 Wi-Fi）用，请看下面的 [通过 Cloudflare Tunnel 从互联网访问](#通过-cloudflare-tunnel-从互联网访问)，不要直接把端口开到互联网上。

## 通过 Cloudflare Tunnel 从互联网访问

适用场景：Xexam 跑在家里的局域网里，`cloudflared`（Cloudflare 的隧道客户端）也在局域网里的某台机器上（不一定是同一台），你想用自己的域名在外面访问，并且在 **Cloudflare Access** 里设置身份验证。

```mermaid
sequenceDiagram
    autonumber
    actor U as 你（在外面）
    participant CF as Cloudflare<br/>（域名 + Access）
    participant T as cloudflared<br/>（局域网）
    participant X as Xexam 容器<br/>（局域网 主机IP:8765）

    U->>CF: 打开 https://xexam.你的域名.com
    CF-->>U: 要求登录（Cloudflare Access）
    U->>CF: 通过验证
    CF->>T: 经隧道转发请求
    T->>X: HTTP 请求（Host 是你的域名，或被改写成 主机IP:8765）
    X->>X: 检查 Host 在白名单里；写请求再检查 Origin
    X-->>U: 页面 / 数据（原路返回）
    Note over U,X: 上传 Excel、交卷等 POST 请求，浏览器带的 Origin 是 https://你的域名<br/>程序只比较主机名，所以不会被拦
```

### 需要做的三件事

**1. 让局域网里的 cloudflared 连得到容器。** 默认容器只绑定本机（`127.0.0.1`）。在 `docker-compose.yml` 旁边建一个 `.env` 文件：

```ini
# 让局域网里别的机器（cloudflared）能连到这台机器的 8765 端口
XEXAM_BIND=0.0.0.0:8765
# 允许用你的域名访问（可以写多个，逗号分隔）
XEXAM_ALLOWED_HOSTS=xexam.你的域名.com
```

然后 `docker compose up -d`。

> 如果 cloudflared 和 Xexam 在**同一台机器**上，可以不改 `XEXAM_BIND`（保持默认的只绑定本机），隧道的目标写 `http://localhost:8765` 就行。

**2. 在 Cloudflare 里把域名指到 Xexam。** Zero Trust 控制台 → Networks → Tunnels → 你的隧道 → Public Hostname：

| 项目 | 填什么 |
|---|---|
| Subdomain / Domain | `xexam` / `你的域名.com` |
| Service Type | `HTTP`（不是 HTTPS） |
| URL | `运行 Xexam 的主机的局域网IP:8765`，例如 `192.168.1.20:8765` |

**3. 在 Cloudflare Access 里加一条应用策略**，只允许你自己的账号（邮箱）访问这个域名。

### 几种常见情况

| 你的情况 | 怎么配 |
|---|---|
| 隧道把 **Host 原样转成你的域名** | 只需要 `XEXAM_ALLOWED_HOSTS=xexam.你的域名.com`，上面第 1 步就够了 |
| 你在 Cloudflare 里设了 **HTTP Host Header**（`httpHostHeader`），把 Host 改成了 `192.168.1.20:8765` | `XEXAM_ALLOWED_HOSTS=192.168.1.20` 再加 `XEXAM_ALLOWED_ORIGINS=https://xexam.你的域名.com`。浏览器发的 `Origin` 仍是公网域名，要单独放行 |
| 不确定 Host 到底是什么 | 先照上面第 1 步配好再打开页面。如果被拒绝（403），页面上的提示会写出它看到的 `Host`（或 `Origin`）：把那个值加进 `XEXAM_ALLOWED_HOSTS`（或 `XEXAM_ALLOWED_ORIGINS`），`docker compose up -d` 重新创建容器即可 |
| 不想列域名，前面已经有 Access 验证 | `XEXAM_ALLOW_ANY_HOST=1`（不再检查 `Host`；写请求的 `Origin` 检查仍然有效） |

我在本机用**真实的 Chrome + 一个 HTTPS 反向代理**模拟过这两种情况（浏览器上传 Excel → 预览 → 确认导入 → 交卷，全程通过）。**没有**用真正的 Cloudflare 测过。

### 不要忘记的事

- ⚠️ **程序自己没有登录。** 一旦把 `XEXAM_BIND` 设成 `0.0.0.0`，或者设了 `XEXAM_ALLOW_ANY_HOST=1`，**能连到这个端口的人就能看到和修改题库**。所以：
  - 只让 cloudflared 连得到这个端口：路由器上**不要**做端口映射。在 Linux 上，主机防火墙（ufw、firewalld）**拦不住** Docker 发布的端口，要限制来源请在 `DOCKER-USER` 链里加规则，或者把 `XEXAM_BIND` 写成具体的局域网 IP（如 `192.168.1.20:8765`）；最稳的是让 cloudflared 和 Xexam 在同一台机器上，保持默认的 `127.0.0.1`。
  - Cloudflare Access 的策略一定要设好，并且**只允许你自己**。没设策略的域名等于对全世界公开。
- 经过 Cloudflare 访问时，登录会过期。过期后上传或交卷会提示「可能是登录已过期，请刷新页面重新登录」，刷新、重新登录就行。**做到一半的考试存在浏览器里，不会丢**。
- 被拒绝（403）时，页面或 `docker compose logs` 里的提示会写明：看到的 `Host` / `Origin` 是什么，应该把哪个值加进哪个选项。

## 目录结构

```
README.md                  本文件（架构、部署、使用、更新日志）
VERSION                    版本号（和更新日志最上面一条保持一致）
server.py                  HTTP 服务：接口、上传检查、导入预览/确认
app/db.py                  SQLite 存储：题库、导入计划、成绩、错题统计
app/xlsx_parser.py         读取 .xlsx 并解析题目（zipfile + ElementTree）
web/                       单页前端：index.html / app.js / style.css / icon.svg / apple-touch-icon.png
docs/screenshots/          README 里的截图（演示题库）
Dockerfile                 非 root 用户运行，/data 是卷
docker-compose.yml         拉取镜像运行（只需要这一个文件）
docker-compose.build.yml   从源码构建运行
.env                       （自己建，不提交）本机配置：端口、域名，见「快速开始」里的表
scripts/release-notes.py    从更新日志生成 GitHub Release 的说明（CI 用，本机也能跑）
scripts/smoke-test.sh      冒烟测试：CI 里每种 CPU 架构各跑一次，装了 Docker 的机器上也能直接运行
.github/workflows/         自动构建、测试、发布
使用说明.txt                面向使用者的简要说明
start.bat                  Windows 上不用 Docker 启动
```

## 安全说明

- 默认只接受发给本机的请求（检查 `Host`）；放在 Cloudflare 等有身份验证的反向代理后面时，把你的域名写进 `XEXAM_ALLOWED_HOSTS`。写请求还会检查 `Origin` 必须和 `Host` 是同一个主机（或在 `XEXAM_ALLOWED_ORIGINS` 里），防止恶意网页通过你的浏览器操作它。
- **程序自己不做登录。** 从互联网访问时必须在前面加身份验证（Cloudflare Access），不要把端口直接暴露到公网。
- 上传的文件会检查格式，拒绝空文件、非 `.xlsx`、损坏的压缩包、含 XML 实体声明的文件，最大 20 MB。
- 上传后的预览在内存里保留 30 分钟，确认时需要一次性的校验码，用过就作废。
- 数据库写入在一个事务里：中途出错整体回滚，不会留下半截数据。
- 容器以非 root 用户（uid 10001）运行。
- 静态文件只从 `web/` 目录提供，不允许 `..` 穿越，网页读不到数据库和代码。
