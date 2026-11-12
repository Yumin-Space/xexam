[English](../README.md) | [日本語](README.ja.md) | **简体中文**

<!-- The release badge shows RELEASE_VERSION from VERSION, the last published release. Do not edit it by hand: the release job of .github/workflows/ci.yml writes it (scripts/set-release-version.py) after the images and the GitHub Release are published. GitHub rounds an image's corners only when it has both width and height, so every badge carries its exact size (in px, from shields.io). -->
<p>
  <a href="../../../releases/latest"><img width="132" height="28" alt="发布版本 v0.9" src="https://img.shields.io/badge/release-v0.9-green?style=for-the-badge"></a>
</p>

<p align="center">
  <img width="120" height="120" alt="Xexam 标志" src="../frontend/public/icon.svg">
</p>

<p align="center">
  <a href="https://hub.docker.com/r/yuminspace/xexam-frontend"><img width="360" height="28" alt="Docker image yuminspace/xexam-frontend" src="https://img.shields.io/badge/Docker%20Image-yuminspace%2Fxexam--frontend-blue?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-frontend"><img width="150" height="28" alt="Docker stars" src="https://img.shields.io/docker/stars/yuminspace/xexam-frontend?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-frontend"><img width="148" height="28" alt="Docker pulls" src="https://img.shields.io/docker/pulls/yuminspace/xexam-frontend?style=for-the-badge"></a>
</p>
<p align="center">
  <a href="https://hub.docker.com/r/yuminspace/xexam-backend"><img width="352" height="28" alt="Docker image yuminspace/xexam-backend" src="https://img.shields.io/badge/Docker%20Image-yuminspace%2Fxexam--backend-blue?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-backend"><img width="150" height="28" alt="Docker stars" src="https://img.shields.io/docker/stars/yuminspace/xexam-backend?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-backend"><img width="148" height="28" alt="Docker pulls" src="https://img.shields.io/docker/pulls/yuminspace/xexam-backend?style=for-the-badge"></a>
</p>
<p align="center">
  <a href="https://hub.docker.com/r/yuminspace/xexam-ai-service"><img width="369" height="28" alt="Docker image yuminspace/xexam-ai-service" src="https://img.shields.io/badge/Docker%20Image-yuminspace%2Fxexam--ai--service-blue?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-ai-service"><img width="150" height="28" alt="Docker stars" src="https://img.shields.io/docker/stars/yuminspace/xexam-ai-service?style=for-the-badge"></a>
  <a href="https://hub.docker.com/r/yuminspace/xexam-ai-service"><img width="148" height="28" alt="Docker pulls" src="https://img.shields.io/docker/pulls/yuminspace/xexam-ai-service?style=for-the-badge"></a>
</p>

# Xexam

Xexam 是一个自己部署、在浏览器里用的刷题程序。你导入自己的 Excel 题库，用它们考试或练习，再复习答错的题。如果管理员接入了 AI 服务商并为用户开通了 **AI+ 功能**，用户还可以在 **AI 对话** 里向 AI 模型提问，并在学习时得到 AI 帮助：练习中的提示和解释、成绩页上对每道题的 AI 解析，以及对答错题目的 AI 总结。电脑和手机（iPhone）都能用。

![答题页：一道显示成中文的热点题、English / 中文 / 日本語 切换按钮，以及带交卷按钮的答题卡](../docs/zh-CN/screenshots/desktop-2-quiz.png)

仓库里**没有题目**。Xexam 只提供导入、整理和练习题目的工具；题库是你自己的 Excel 文件，不会被提交（`.gitignore` 已排除）。截图里用的是虚构的演示题库，不是任何真实考试的内容。

| | |
|---|---|
| 当前版本 | **v0.9.1**（见 [更新日志](../docs/zh-CN/changelog.md)，也可以在仓库的 [Releases](../../../releases) 页看；在程序里：菜单 → **Xexam v0.9.1**） |
| 镜像 | 共三个，在 Docker Hub 上（公开，不用登录）：`docker.io/yuminspace/xexam-frontend`、`docker.io/yuminspace/xexam-backend` 和 `docker.io/yuminspace/xexam-ai-service`，标签 `latest` / `v0.9.1` / `sha-<提交>`（`sha-` 标签用来排查问题）。镜像副本：`ghcr.io/yumin-space/xexam-frontend`、`ghcr.io/yumin-space/xexam-backend` 和 `ghcr.io/yumin-space/xexam-ai-service`（私有，标签相同） |
| CPU 架构 | `linux/amd64`、`linux/arm64`（同一个标签里自动选对的） |
| 端口 | `8765`（默认只绑定本机） |
| 许可证 | [MIT](../LICENSE) |

---

## 目录

- [能做什么](#能做什么)
- [快速开始](#快速开始)
- [配置](#配置)
- [升级](#升级)
- [文档](#文档)
- [许可证](#许可证)
- [更新日志](#更新日志)

---

## 能做什么

- **考试**：交卷后才计分。然后显示分数，每道错题（也可以看全部题目）都列出你的答案、正确答案、解析和记忆口诀。可以随时跳题、标记想回头再看的题，也可以暂存退出，以后接着考。
- **练习**：每道题作答后马上核对，显示正确答案和解析。
- **你自己的题库**：导入的每一份 Excel 工作簿都是一个独立的题库，里面的每个工作表是一组可以挑着做的题。工作簿用六列格式：题目和答案用英语写，愿意的话再加上中文和日语。保存之前会先显示预览，告诉你导入会带来哪些变化。
- **三种语言**：界面有英语、中文、日语三种，每道题、每个答案和每段解析都可以在 **English**、**中文** 和 **日本語** 之间切换。
- **学习记录和错题本**：每场考试和每次练习都连同作答保存下来；答错的题留在错题本里，直到连续答对两次。
- **多人共用一个 Xexam**：每个人都登录使用，有自己的学习记录和设置。管理员决定谁能看到哪些题库、谁可以导入题库（可管理题库的用户），还可以查看 **系统日志**。
- **AI 对话**（可选，管理员设置好之前是关闭的）：管理员在 **AI 设置** 里接入 OpenAI 兼容 API、Anthropic 或 Cloudflare AI Gateway，测试后开启 **AI+ 功能**，再在 **用户管理** 里为每个可以使用的用户开通 **AI+ 功能**。在 **AI 设置** 里，他们可以 **暂停** 和 **恢复** 某个用户的 **AI+ 功能**，而不用收回它。回答边生成边显示（推理模型工作时显示 **正在思考…**），关掉页面也会继续生成；AI 用 **设置** 里选的语言回答；对话会保存下来（默认在 90 天后自动删除）；**记忆** 保存一些简短的笔记，AI 会在以后的对话里参考；管理员可以在 **设置** 的 **AI 对话模式** 里选 **自由模式**，不带学习助手的指示、直接和模型对话。管理员能看到用量统计，也能查看和删除拥有 AI+ 功能的用户的对话和记忆，AI 对话页面会把这一点告诉每个用户。AI 在自己的容器 `ai-service` 里运行，有自己的数据库；服务商的密钥加密保存。
- **学习时的 AI 帮助**（给有 **AI+ 功能** 的用户；考试进行中没有）：练习时，**AI 提示**（讲解怎样着手做这道题，不给出答案）和 **AI 解释**（答案和原因），可从操作栏或题目上方的灯泡和书本图标打开，回答在一个小窗口里逐步显示；每个练习和考试的成绩页以及学习记录的详情页上，每道题都有 **AI 解析**；在 **学习记录** 和 **主页** 上，可以对一条记录里答错的题生成 **AI 总结**（薄弱的知识点、需要注意什么、以后怎样做这类题）。每个回答都保存在 AI 对话里：它所属练习的对话（`<开始时间>-练习`）、考试的对话（`<开始时间>-考试`）或总结的对话（`<记录时间>-AI总结`）。AI 被暂停或暂时不能用的用户会看到这些控件变灰，标着 **无法使用** 并写明原因；没有 **AI+ 功能** 的用户完全看不到它们。它们会把题目（解释和总结还有题库里的答案和解析，以及你自己的答案）发给你设置的 AI 服务商；提示的请求里从不包含答案，泄露了答案的提示会被隐藏。
- **电脑和手机**：窄窄的页头上有一个菜单，宽屏上页面居中显示，手机上的触控区域至少 44 px。

每个页面怎么用、练习和成绩里的 AI 帮助、AI 对话和 AI 设置、Excel 格式和导入规则：见 [docs/zh-CN/usage.md](../docs/zh-CN/usage.md)。

---

## 快速开始

你需要一台装了 **Docker**（含 Compose v2）的机器，Windows、macOS 或 Linux 都行，**x86 或 ARM 都行**，再加一个浏览器。

**1. 下载 [`docker-compose.yml`](../docker/docker-compose.yml)**（在本仓库的 `docker/` 文件夹里），放进一个新文件夹（例如 `xexam`）。只需要这一个文件。最简单的办法：在 GitHub 上打开这个文件，点 **Download raw file**。也可以用命令行：

```bash
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml
```

仓库是私有的：要用 `curl` 下载这个文件（或者 `git clone` 整个仓库），请所有者把你的 GitHub 账号加为协作者，在 GitHub → **Settings** → **Developer settings** → **Personal access tokens** → **Tokens (classic)** 里创建一个勾选 `repo` 权限的 classic 令牌，并在 `-fLO` 后面加 `-H "Authorization: token 你的令牌"`。也可以请所有者把这个文件发给你。在 Windows PowerShell 里要写 `curl.exe`，而不是 `curl`。镜像本身在 Docker Hub 上是公开的：拉取它们不需要账号，也不需要 `docker login`。

**2. 启动。** 在 `docker-compose.yml` 所在的文件夹里运行：

```bash
docker compose up -d
```

这会拉取三个镜像，并启动**三个容器**：`xexam-frontend`（Nginx，唯一能从外面连到的）、`xexam-backend`（Rust，在不能上网的私有网络里）和 `ai-service`（Rust，AI 服务；唯一能连到互联网的容器，而且只用来调用管理员设置的 AI 服务商）。它每 25 秒向后端报告一次，所以 Xexam 知道 AI 能不能用。没有 `ai-service`，Xexam 也照样能用，只是没有 AI 对话，**AI 设置** 里会写明 ai-service 没有运行。

**3. 登录。** 在同一台机器上用浏览器打开 <http://localhost:8765>，用用户名 `admin`、密码 `admin` 登录。Xexam 会接着要求你设置自己的密码：请马上设置，因为在那之前，能打开这个页面的人都能用默认密码登录。

**4. 导入题库，开始考试。** 打开菜单（右上角三条横线的按钮），选 **Question banks**（中文界面里叫 **题库管理**），导入你的 Excel 工作簿，然后在 **Home**（**主页**）上开始考试：见 [第一场考试](../docs/zh-CN/usage.md#1-第一场考试)。界面一开始是英语；在 **Settings** → **Language**（中文界面里是 **设置** → **语言**）里换成中文或日语。

**5. AI 对话（可选）。** 在你设置之前 AI 是关闭的：菜单 → **AI 设置**（它会显示 ai-service 有没有连接），选择服务商，填好它的地址和密钥，从列表里选一个模型，点 **测试**，再开启 **AI+ 功能**。管理员马上就能使用 AI 对话；其他用户要等你在 **用户管理** 里为他们开启 **AI+ 功能**（只有 ai-service 已连接、而且 AI+ 功能已开启时，那里才会出现这个开关）。具体步骤见 [docs/zh-CN/usage.md 第 17 节](../docs/zh-CN/usage.md#17-ai-设置)。

| 操作 | 命令 |
|---|---|
| 更新到最新版本 | 先 `docker compose pull`，再 `docker compose up -d`（请先看 [升级](#升级)） |
| 固定在某个版本 | 把 `docker-compose.yml` 里三个镜像的 `:latest` 都改成 `:v0.9.1`，再 `docker compose up -d` |
| 查看状态 | `docker compose ps`（三个容器都应该是 `healthy`） |
| 停止 / 再启动 | `docker compose stop` / `docker compose start` |
| 看日志 | `docker compose logs -f backend` 和 `docker compose logs -f ai-service`（管理员也可以在菜单里打开 **系统日志**；那里也显示 AI 服务的日志） |
| 看现在是哪个版本、什么架构 | 浏览器打开 <http://localhost:8765/api/info> |

> ⚠️ **不要运行 `docker compose down -v`**：`-v` 会把两个数据卷一起删掉：`xexam_data`（题库、用户、学习记录和设置）和 `xexam_ai_data`（AI 设置、加密的密钥和解密用的密钥文件、对话和记忆）。

- **从手机或另一台电脑访问**，或者**通过 Cloudflare Tunnel 从互联网访问**：见 [docs/zh-CN/deployment.md](../docs/zh-CN/deployment.md) 第 4 节。
- **从源码构建镜像**（改代码时用）：在克隆的仓库里运行 `docker compose -f docker/docker-compose.build.yml up -d --build`，见 [docs/zh-CN/deployment.md](../docs/zh-CN/deployment.md) 第 2 节。
- **备份**：停止 Xexam，把两个数据卷打包；命令见 [docs/zh-CN/database.md](../docs/zh-CN/database.md) 第 5 节。AI 卷里还有用来解密已保存的 AI 密钥的密钥文件，所以它的备份要像密钥本身一样妥善保管。

---

## 配置

默认设置在一台电脑上直接能用，不需要配置。要改的时候，在 `docker-compose.yml` 旁边建一个 `.env` 文件（克隆的仓库里是 `docker/.env`；模板见 [`.env.example`](../docker/.env.example)），然后再运行一次 `docker compose up -d`。不需要改 compose 文件本身。

| `.env` 里的名字 | 默认 | 作用 |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | 监听的地址和端口，写成 `IP:端口`。默认只有本机能连；`0.0.0.0:8765` = 网络里别的机器也能连。⚠️ 只写端口号（如 `8766`）= 所有网卡都能连 |
| `XEXAM_ALLOWED_HOSTS` | 空 | 允许用哪些域名 / IP 打开 Xexam，用逗号、空格或分号分隔，例如 `xexam.example.com,192.168.1.20` |
| `XEXAM_ALLOWED_ORIGINS` | 空 | 反向代理把 Host 改写成源站地址时才需要：填你的公网域名（带 `https://`） |
| `XEXAM_ALLOW_ANY_HOST` | `0` | `1` = 不再检查 Host（只在前面已经有身份验证时用） |
| `XEXAM_PARSE_TIMEOUT_SECS` | `30` | 解析一份 Excel 的最长时间（秒） |
| `XEXAM_SQLITE_JOURNAL` | 不设（WAL） | `delete` 让 SQLite 继续用以前的 DELETE 日志，只在数据卷放在特殊的存储上时用（网络文件系统，或者绑定挂载进 Docker 的 Windows 文件夹） |
| `XEXAM_COOKIE_SECURE` | 不设 | `1` = 会话 Cookie 总是带 `Secure`；只在总是通过 HTTPS 访问 Xexam 时用 |
| `XEXAM_LOG` | 不设（`info`） | 后端的日志过滤，例如 `debug`；为空 = `info` |
| `TZ` | 不设（宿主机的时区） | Xexam 显示的时间、以及 AI 用量统计里的日期所用的时区，例如 `TZ=UTC` |
| `XEXAM_AI_SECRET_KEY` | 不设（AI 卷里的一个密钥文件） | 加密已保存的 AI 服务商密钥所用的密钥：32 字节，写成 64 个十六进制数字或 base64，例如用 `openssl rand -base64 32` 生成。设置它可以让这个密钥和 AI 卷分开存放，这样只拿到那个卷的副本也解不开密钥。它改了或丢了，就要在 **AI 设置** 里重新输入 AI 密钥 |
| `XEXAM_INTERNAL_TOKEN` | 不设（链接卷里的一个随机令牌） | 后端和 AI 服务共用的令牌；compose 文件把同一个值传给两者。至少 32 个可打印的 ASCII 字符，不能有空格，例如用 `openssl rand -hex 32` 生成。AI 服务自己对后端的调用（例如它的状态报告）也用它来认证。不符合这个规则的值会让 AI 对话无法使用：**AI 设置** 里会写明 ai-service 无法连接，原因写在一行日志里 |
| `XEXAM_AI_LOG` | 不设（`info`） | AI 服务的日志过滤，和 `XEXAM_LOG` 一样 |
| `XEXAM_AI_SQLITE_JOURNAL` | 不设（WAL） | `delete` 用于 AI 数据库，和 `XEXAM_SQLITE_JOURNAL` 一样 |

时间跟随宿主机的时区，因为 compose 文件把宿主机的 `/etc/localtime` 挂载进了后端和 AI 服务。宿主机上没有这个文件时，以及在 Docker Desktop（Windows、macOS）上（它的虚拟机通常用的是 UTC），请看 [docs/zh-CN/deployment.md](../docs/zh-CN/deployment.md) 第 3.1 节。每个选项的说明都在那份文档的第 3 节。AI 服务商、它的地址、密钥和模型不在这里设置，而是在程序的 **AI 设置** 里设置。

---

## 升级

每次升级都还是同样的端口、同样的卷 `xexam_data`，所以数据原样保留：先 `docker compose pull`，再 `docker compose up -d`。v0.8 新增了第三个镜像和两个卷，所以从 v0.7.5 或更早的版本升级时，还需要新的 `docker-compose.yml`；v0.9 不需要新的 compose 文件。数据库 schema 有变化时，新版本第一次启动会先把数据库的一份备份副本写到卷里的 `/data/backups/`（AI 服务对自己的数据库也这样做，写在它自己的卷里），再在一个事务里迁移数据库。副本写不出来（磁盘满、没有权限）时，服务会拒绝启动，数据库保持原样；`docker compose logs backend` 和 `docker compose logs ai-service` 里会写明原因。备份副本和数据库差不多大，所以升级之前请留出这么多空闲空间；备份副本从不会自动删除。

下面各节是为它们升级到的那个版本写的。直接用当前的镜像（v0.9.1）从更早的版本升级，会一次做完每一步：写一份以文件原来的 schema 命名的备份，数据库最后停在 schema 6（这时日志里会写 `schema 6`，而某一节写的是 `schema 5`；`/api/info` 显示 `"version": "0.9.1"`）。

### 从 v0.9 升级到 v0.9.1

不用做什么：先 `docker compose pull`，再 `docker compose up -d`。数据库没有变化，也没有新设置；三个镜像、卷、`docker-compose.yml` 和 `.env` 都保持原样。改动见[更新日志](../docs/zh-CN/changelog.md)（更新日志现在放在 `docs/<lang>/changelog.md`）。

### 从 v0.8 升级到 v0.9

v0.9 新增了学习时的 AI 帮助（见 [更新日志](../docs/zh-CN/changelog.md)）。三个镜像、卷和 `docker-compose.yml` 都不变：**不需要新的 compose 文件**。先备份两个数据卷，然后拉取并启动。

```bash
# 1. 备份两个卷（先停止；备份文件里是你的整个数据库，不要提交它们）
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v08.tgz -C /data .
docker run --rm -v xexam_ai_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-ai-data-v08.tgz -C /data .

# 2. 拉取三个镜像并启动（不要用 down -v），然后查看升级信息
docker compose pull
docker compose up -d
docker compose logs backend
docker compose logs ai-service
```

- **后端的第一次启动** 会写一份副本 `/data/backups/xexam-schema5-<YYYYmmdd-HHMMSS>.db`，并把数据库升级到 schema v6：`attempts` 表新增两列，`session_key` 和 `started_at`，你已有的每条记录里它们都是空的。其他都不变。日志里会写 `Upgraded the database to schema 6`。
- **AI 服务的第一次启动** 会在 AI 卷里写一份副本 `/data/backups/xexam-ai-schema2-<YYYYMMDD-HHMMSS>.db`，并把它的数据库升级到 schema 3（学习对话和它们的消息，以及用量记录上的 `purpose`）。你的对话、记忆、设置和密钥都保留。日志里会写 `Upgraded the AI database to schema 3`。
- **不需要设置什么**：**AI+ 功能** 照旧。现在每个能使用 AI 的人都会看到新的控件；没有 **AI+ 功能** 的用户什么新东西都看不到。v0.9 之前生成的记录没有会话 key，所以它们第一次做 AI 解析时，会开一个自己的对话（[docs/zh-CN/usage.md](../docs/zh-CN/usage.md) 第 16 节）。
- 检查 <http://localhost:8765/api/info> 显示 `"version": "0.9"`，并且 `docker compose ps` 列出三个 healthy 的容器。

回退到 v0.8：主数据库不需要恢复（v0.8 原样打开 schema 6 的数据库，之后保存的记录只是没有 key），但 v0.8 的 AI 服务不接受 schema 3 的 AI 数据库，所以必须恢复它的副本 `xexam-ai-schema2-….db`，这会丢失升级之后产生的所有 AI 对话、设置和用量记录。见 [docs/zh-CN/migration.md](../docs/zh-CN/migration.md) 第 0 节。

### 从 v0.7.5（或 v0.7）升级到 v0.8

v0.8 新增了 AI 服务：第三个镜像 `xexam-ai-service`（容器和 Compose 服务都叫 `ai-service`）；两个新卷，`xexam_ai_data`（AI 数据库和它的密钥文件）和 `xexam_link`（只放后端和 AI 服务共用的令牌）；以及网络 `egress`（只有 AI 服务能通过它上网）。**请下载新的 `docker-compose.yml`**。用旧的文件，除了 AI 以外一切照常，**AI 设置** 里会提示 ai-service 没有运行。

```bash
# 1. 备份卷（先停止；备份文件里是你的整个数据库，不要提交它）
docker compose stop
docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v075.tgz -C /data .

# 2. 换上新的 docker-compose.yml（你的 .env 保持不变；你对旧文件做过的修改要自己搬过去）
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

# 3. 拉取三个镜像并启动（不要用 down -v），然后查看升级信息
docker compose pull
docker compose up -d
docker compose logs backend
```

- **第一次启动** 会写一份副本 `/data/backups/xexam-schema4-<YYYYmmdd-HHMMSS>.db`，并把数据库升级到 schema v5：`users` 表新增两列，即 **AI+** 开关（所有人都是关闭的）和一个身份标记。其他都不变，题库、记录、用户和设置都保留。日志里会写 `Upgraded the database to schema 5`。
- **AI 是关闭的**，直到管理员在 **AI 设置** 里把它设置好（见 [docs/zh-CN/usage.md](../docs/zh-CN/usage.md) 第 17 节）；AI 没有需要迁移的东西。AI+ 功能一开启，管理员就能使用 AI 对话；其他每个用户都需要在 **用户管理** 里开通 **AI+ 功能**，这个开关只有在 ai-service 已连接、而且 AI+ 功能已开启时才能打开。AI 数据库 `xexam-ai.db` 由 ai-service 在第一次启动时创建；如果它来自 v0.8 的某个预发布版本，缺少的列会自动补上。
- **从现在起要备份什么**：两个数据卷，`xexam_data` 和 `xexam_ai_data`。AI 卷里在 AI 数据库旁边放着密钥文件 `secret.key`，所以它的备份能解密已保存的密钥；设置 `XEXAM_AI_SECRET_KEY` 可以把密钥分开存放。`xexam_link` 不用备份：它为空时后端会生成一个新令牌。
- 检查 <http://localhost:8765/api/info> 显示版本（`"0.8"`，用当前的镜像则是 `"0.9.1"`），并且 `docker compose ps` 列出三个 healthy 的容器（`xexam-frontend`、`xexam-backend` 和 `ai-service`）。

回退到 v0.7.5 就是恢复 `backups/` 里的那份副本，并换回旧的 `docker-compose.yml`；见 [docs/zh-CN/migration.md](../docs/zh-CN/migration.md) 第 1 节。

### 从 v0.6 升级到 v0.7 及以后的版本

> ⚠️ 从 v0.7 起**只能导入六列 Excel 格式**，以前版本的三列或四列格式导入的每道题，**在每次启动时都会被下架**。v0.6 的题库全都是这些格式，所以升级之后，要等你用六列格式的工作簿更新过题库，才有题可做。题库、学习记录和统计都保留。

1. **升级之前**，为每份工作簿准备好六列格式的版本（格式见 [docs/zh-CN/usage.md](../docs/zh-CN/usage.md) 第 12 节），并把没交卷的考试交卷或放弃：它的题会被下架，之后不能再接着考。
2. **备份并升级**：

   ```bash
   # 1. 备份卷（先停止；备份文件里是你的整个数据库，不要提交它）
   docker compose stop
   docker run --rm -v xexam_data:/data -v "${PWD}:/backup" alpine tar czf /backup/xexam-data-v06.tgz -C /data .

   # 2. 换上新的 docker-compose.yml（你的 .env 保持不变）
   curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

   # 3. 拉取并启动（不要用 down -v），然后查看升级信息
   docker compose pull
   docker compose up -d
   docker compose logs backend
   ```

   新的 `docker-compose.yml` 跟随宿主机的时区，限制容器日志的大小，把 `.env` 里的 `XEXAM_SQLITE_JOURNAL`、`XEXAM_COOKIE_SECURE` 和 `XEXAM_LOG` 传进去，还加入了 v0.8 的 AI 服务和它的卷。
3. **第一次启动做什么**（在响应任何请求之前）：写一份副本 `/data/backups/xexam-schema3-<YYYYmmdd-HHMMSS>.db`，把数据库迁移到 schema v5（v0.7 和 v0.7.5 停在 v4），并改用 WAL 模式。已有的每个用户都保留已有的每个题库，并设置 **包括以后导入的题库**；每个题库都归第一个管理员；以前的每条记录都是 **考试**；还没有人是可管理题库的用户。然后它下架旧格式的题，日志里每个题库一条警告，例如 `retired 12 questions of bank "Kitchen basics": they use the old 3- / 4-column layout; import the six-column version of the workbook`。
4. **用六列格式的文件更新每个旧题库**：**题库管理** → **导入 Excel** → **更新已有题库**，选好题库，上传文件并确认。题库的名字、所有者和能看到它的用户都不变。它的题现在按英语文字识别，所以从没有答题记录开始；旧题仍保持下架，以前的记录里仍然显示它们。也可以把六列格式的文件作为 **新建题库** 导入。
5. **之后**：决定谁是 **可管理题库** 的用户、每个用户能看到哪些题库（从现在起新添加的用户，在你选好之前看不到任何题库），并检查 <http://localhost:8765/api/info> 显示 `"version": "0.9.1"`。在你设置之前 AI 是关闭的（见上一节）。

回退到 v0.6 就是恢复 `backups/` 里的那份副本；命令见 [docs/zh-CN/migration.md](../docs/zh-CN/migration.md) 第 2.6 节。

### 从更早的版本升级

从 v0.5、v0.4 或 Python 版 v0.2.x 升级，步骤相同，只是多几步（新增了登录，镜像的来源在 v0.5.1 变了，旧的单个容器要删掉）：见 [docs/zh-CN/migration.md](../docs/zh-CN/migration.md)，里面也有每一种回退。

---

## 文档

| 文档 | 内容 |
|---|---|
| [使用说明](../docs/zh-CN/usage.md) | 每个页面、考试和练习、学习记录、设置、Excel 格式、导入、用户和登录、系统日志、AI 对话和 AI 设置、iPhone |
| [部署](../docs/zh-CN/deployment.md) | 启动、配置、时区、从手机和局域网访问、Cloudflare Tunnel、AI 服务商、备份、健康检查、日志、恢复 |
| [迁移与回滚](../docs/zh-CN/migration.md) | 每一条升级路径，以及它的检查和回退 |
| [数据库](../docs/zh-CN/database.md) | 主数据库和 AI 数据库的表、题目 id、备份和恢复、schema 迁移 |
| [安全](../docs/zh-CN/security.md) | 威胁模型、Host / Origin 守卫、登录和锁定、权限、上传检查、Nginx 的限制；AI 服务：身份、加密的密钥、管理员能看到什么 |
| [架构](../docs/zh-CN/architecture.md) | 前端、Nginx、后端和 AI 服务怎样配合、数据边界、目录结构、请求的流程 |
| [API](../docs/zh-CN/api.md)（[OpenAPI](../docs/en/openapi.yaml)，英文） | 每个接口、AI 路由和它们的事件流、谁可以调用、错误 |
| [开发](../docs/zh-CN/development.md) | 构建和测试（后端、AI 服务、前端）、截图和图标工具、CI 和发布 |

---

## 许可证

Xexam 以 [MIT 许可证](../LICENSE) 发布。Copyright (c) 2026 Yumin-Space.

许可证适用于本仓库里的软件。仓库里没有任何考试题目：你导入的题库是你自己的文件，你要自己确认有权使用它们。

---

## 更新日志

每个版本的变化（最新的在前）见 [docs/zh-CN/changelog.md](../docs/zh-CN/changelog.md)。
