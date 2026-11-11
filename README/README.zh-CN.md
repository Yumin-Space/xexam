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
| 当前版本 | **v0.9**（见 [更新日志](#更新日志)，也可以在仓库的 [Releases](../../../releases) 页看；在程序里：菜单 → **Xexam v0.9**） |
| 镜像 | 共三个，在 Docker Hub 上（公开，不用登录）：`docker.io/yuminspace/xexam-frontend`、`docker.io/yuminspace/xexam-backend` 和 `docker.io/yuminspace/xexam-ai-service`，标签 `latest` / `v0.9` / `sha-<提交>`（`sha-` 标签用来排查问题）。镜像副本：`ghcr.io/yumin-space/xexam-frontend`、`ghcr.io/yumin-space/xexam-backend` 和 `ghcr.io/yumin-space/xexam-ai-service`（私有，标签相同） |
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
| 固定在某个版本 | 把 `docker-compose.yml` 里三个镜像的 `:latest` 都改成 `:v0.9`，再 `docker compose up -d` |
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

下面各节是为它们升级到的那个版本写的。直接用当前的镜像（v0.9）从更早的版本升级，会一次做完每一步：写一份以文件原来的 schema 命名的备份，数据库最后停在 schema 6（这时日志里会写 `schema 6`，而某一节写的是 `schema 5`；`/api/info` 显示 `"version": "0.9"`）。

### 从 v0.8 升级到 v0.9

v0.9 新增了学习时的 AI 帮助（见更新日志）。三个镜像、卷和 `docker-compose.yml` 都不变：**不需要新的 compose 文件**。先备份两个数据卷，然后拉取并启动。

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
- 检查 <http://localhost:8765/api/info> 显示版本（`"0.8"`，用当前的镜像则是 `"0.9"`），并且 `docker compose ps` 列出三个 healthy 的容器（`xexam-frontend`、`xexam-backend` 和 `ai-service`）。

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
5. **之后**：决定谁是 **可管理题库** 的用户、每个用户能看到哪些题库（从现在起新添加的用户，在你选好之前看不到任何题库），并检查 <http://localhost:8765/api/info> 显示 `"version": "0.9"`。在你设置之前 AI 是关闭的（见上一节）。

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

每次更新都要在这里写一条，**版本号和仓库根目录 `VERSION` 文件里的 `CODE_VERSION` 一致**（自动构建会检查，不一致就不让构建）。最新的镜像同时有 `latest` 和对应的 `vX.Y` 标签。

程序里的 **关于 Xexam** 页面会显示这些记录（它们在构建时打包进前端），所以请只用简单的 Markdown：段落、`- ` 列表、缩进的 `  - ` 子列表、`**粗体**` 和 `` `代码` ``。链接在那里显示成普通文字。

### v0.9 — 2026-11-11

**为拥有 AI+ 功能的账号提供学习时的 AI 帮助：练习中的提示和解释、成绩的 AI 解析，以及对答错题目的 AI 总结。三个镜像、数据卷和 docker-compose.yml 都保持不变。数据库会自动升级（主数据库升到 schema 6，AI 数据库升到 schema 3），每个都在写好备份副本之后进行。**

- **谁能看到**：只有拥有 **AI+ 功能** 的账号（AI+ 功能开启时的管理员；橙色的 **AI+** 徽章）。没有它的用户看不到下面任何控件。有这个功能、但现在不能使用 AI 的用户（管理员暂停了它、AI+ 功能已关闭或 ai-service 没有连接）会看到变灰的控件，带 **无法使用** 标记，提示框里写明原因（触屏上点一下就会显示），在学习记录里表格上方还有一句说明。服务器也会拒绝这些请求。考试进行中没有这些；考试只在它的成绩页和它的学习记录里有 AI。
- **练习**：每道题的操作栏里有两个按钮，**AI 提示** 和 **AI 解释**，卡片页头里、语言切换按钮旁边还有同样的两个，分别是灯泡和书本图标。按钮和图标作用相同。
  - **AI 提示**：讲解怎样着手做这道题，以及背后的知识点，但不说哪个选项是对的。发给模型的请求里从不包含答案，如果提示还是把答案泄露了，它会被隐藏（**这个提示透露了答案，已被隐藏。请重新生成。**）；这个检查是一道保险，能拦住大多数、但不是所有的说法。
  - **AI 解释**：先给出正确答案，然后是这道题考查的知识、它为什么对、其他每个选项为什么错。
  - 回答在按钮旁边一个可以关闭的小窗口里逐步显示（Esc、点窗口外面、**关闭** 或换到另一道题都会关掉它），带 **复制**、**重新生成** 和 **在 AI 对话中打开**。再次打开同一个，会马上显示已保存的回答；**重新生成** 写一个新的，新的失败或被隐藏时，上一个回答仍然留着。回答用 **设置** 里的语言（保存的回答是另一种语言时会说明）。
- **成绩**：练习成绩、考试成绩和学习记录详情页上的每张题目卡片都有 **AI 解析**（判定那一行上的一个按钮，卡片页头里的一个书本图标）。它讲解这道题，答错时还会讨论你自己的答案。它的题目文字跟随卡片自己的语言切换按钮。
- 一条记录的 **AI 总结**，在 **学习记录**（新的第一列，每行一个按钮）和 **主页**（在最近一次成绩里，**查看详情** 和 **全部学习记录** 之间）：一个大窗口，里面是薄弱的知识点、需要注意什么、以后怎样做这类题，以及对这些内容的简单总结。只计入答错的题（未作答的不算），最多 70 道，更多时随机抽取；窗口里会写例如 **基于 80 道答错的题中随机抽取的 70 道。** **重新生成** 写一个新的总结，再按一次按钮则显示已保存的那个。没有答错的题的记录，或者由没有保存逐题详情的版本保存的记录，按钮是灰的并写明原因。
- **一次一个回答**：一个回答正在生成时，另一个请求会说明是哪一个（**第 3 题的 AI 提示 还在生成。**），并提供 **查看**（在同一页面上时）和 **停止它，生成这一个**；什么都不会自己停下。学习的回答和 AI 对话共用这一处。
- **在 AI 对话里**：每个练习、考试和总结都有一个对话，标题是 `<开始时间>-练习`、`<开始时间>-考试` 或 `<记录时间>-AI总结`（标题里的文字跟随界面语言，时间用 ai-service 的时区）。每个回答都按顺序写进去，每道题标明是第 n 题的提示、解释还是解析；练习页上的提示和解释，与它的成绩页上的解析，共用这个练习的对话。学习的回答没有 **重试** 和 **记住这个**。删除学习记录会保留它的总结对话。和所有对话一样，学习对话遵循保留期，用户可以删除，管理员可以查看；已删除的对话里的回答不再作为已保存的回答提供。
- **v0.9 之前生成的记录** 没有会话 key，所以在这样的记录的详情页上第一次做 AI 解析时，会开一个自己的对话，标题是记录的时间加上它的类型后缀。
- **发给 AI 服务商的数据**：提示发的是题目和页面上所显示语言的选项；解释或解析还包括题库里的答案和解析，在成绩页上还有你自己的答案；总结对每道答错的题发同样的内容。记忆笔记、以前的对话和自由模式都不参与这些请求。用量按功能统计（AI 设置里 **用量** 下的 **按功能**：AI 对话、AI 提示、AI 解释·解析、AI 总结），从不带内容。
- **限制**：开始一个学习回答，和 AI 对话计入同一个 Nginx 限制（每个登录的浏览器，突发 10 次之后每分钟 20 次；到限制时窗口会说等几秒再试）。每个学习对话算一个对话，每次提问算两条消息，所以默认的每个对话 400 条消息，足够在一次很长的练习里提问约 200 次（管理员可以修改）。
- **布局**：手机上操作栏保持两行（两个 AI 按钮用简短的标签，屏幕很窄时只显示图标），变灰的控件和语言切换按钮一起折行，而不是把页面撑宽，学习记录在窄窗口里保持卡片布局，每张卡片里有 AI 总结按钮。
- **数据库 schema 6**（主数据库）：`attempts.session_key` 和 `attempts.started_at`，可以为空；迁移之前会把一份副本写到 `<数据目录>/backups/xexam-schema5-<日期时间>.db`。**AI 数据库 schema 3**：`chats.kind`、`ref` 和 `record_id`；`messages.kind`、`question_id`、`question_no`、`source`、`lang`、`sample_used`、`sample_usable` 和 `sample_wrong`；`usage.purpose`（AI 对话时为空）；三个索引；先把一份副本写到 AI 卷里的 `backups/xexam-ai-schema2-<日期时间>.db`。备份从不会自动删除，每次升级需要的空闲磁盘空间约等于它的数据库的大小。v0.8 能原样打开 schema 6 的主数据库，但它的 AI 服务不接受 schema 3：回退 AI 数据库就是恢复它的副本。AI 服务现在会分小批删除超过保留期的对话。
- **新 API**：`POST /api/ai/study/ask` 和 `POST /api/ai/summary/ask`（server-sent events，和 `send` 一样），`GET /api/ai/study/saved` 和 `GET /api/ai/summary/saved`；对话路由里，对话新增 `kind` 和 `recordId`，消息新增 `kind`、`questionId`、`questionNo`、`source`、`lang` 和 `sample`，`start` 事件里新增 `chatKind`，`ai.busy` 拒绝里新增 `running`，AI 用量里新增 `byPurpose`；`POST /api/attempt` 接受 `key` 和 `startedAt`，并返回新记录的 `recordId`，`GET /api/record` 返回 `sessionKey` 和 `startedAt`；内部的 `POST /internal/ai/question`、`/record` 和 `/record-info`（只用令牌）。Nginx 现在记录请求的路径，但不再记录它的查询字符串。
- **发布带签名**：发布任务记录已发布版本（`RELEASE_VERSION` 和 README 徽章）时，改用 GitHub API 生成提交，不再用 `git push`，所以 GitHub 会为它签名，显示为 **Verified**。
- 测试：后端 342 → 382，前端 246 → 391，AI 服务 160 → 369

### v0.8 — 2026-11-11

**AI 对话放在一个单独的容器里，叫 ai-service，还有新图标和更紧凑的题库权限控件。管理员设置好之前 AI 是关闭的。请下载新的 docker-compose.yml：它新增了第三个镜像 xexam-ai-service 和两个卷。数据库会先写一份备份副本，再自动升级到 schema v5。**

- **AI 对话**（菜单里）：和 AI 模型对话。
  - 回答边生成边显示（推理模型还在思考时显示 **正在思考…**），关掉页面也会继续生成并保存；有 **停止** 和 **重试**。对话会保存下来，并在最后一条消息 90 天后自动删除（可以调整）。
  - AI 用 **设置** 里选的语言回答（除非你在那条消息里要求用另一种语言）；记忆笔记也用这种语言写。新建的空对话会写明 AI 将用哪种语言回答。
  - 像聊天程序那样的固定高度窗口：消息在窗口里滚动（用键盘也能到达并滚动这个区域），输入框始终在底部。每条消息都显示时间，并带 **复制**；**记住这个** 和 **重试** 在它下面排成一行。
  - 地址会记住打开的对话：对话是 `#/ai/<id>`，新对话是 `#/ai/new`，所以刷新后会重新打开同一个对话。
  - **记忆** 保存一些简短的笔记，AI 会在以后的对话里参考，可以手动添加，也可以用 **记住这个** 添加。页面顶部有一行说明：管理员可以查看和删除你的 AI 对话和记忆，也能看到你的 AI 用量。你随时都能查看和删除自己的对话和记忆，AI+ 功能对你关闭或被暂停时也一样。
  - 当你不能使用 AI、又没有对话或记忆时，**AI 对话** 仍然能打开，并说明原因（已暂停、AI+ 功能暂时不可用，或你的账号没有开通）以及该怎么做。只读的 AI 对话顶部那一行，和深色 **AI+** 徽章的提示框是同一句话。
  - **自由模式** 是管理员自己的一种模式，在 **设置** 的 **AI 对话模式** 里选择（**普通模式** / **自由模式**；只有管理员在 AI+ 功能开启时才看得到）。开启后，任何对话（新的或旧的）里的每条消息，都会不带学习助手的指示、今天的日期和记忆发给模型（只保留回复语言规则；服务商自己的安全规则始终有效），**AI 对话** 标题旁边会有绿色的 **自由模式** 标记。AI 对话页面本身没有开关，也没有分隔线。服务器记住这个选择，所以刷新和换设备后都还在；它从下一条消息起生效，默认是关闭的。每条消息都记录它是在哪种模式下写的，切换之后，模型会用一句简短的话得知当前的模式。限制、用量统计和管理员的查看照常。
  - **更省的请求**：服务商可能缓存每个请求的开头部分（Anthropic：在指示和新问题之前的历史上设缓存点；OpenAI 格式的服务商在支持时自己缓存）。为了让这个开头保持稳定，历史总是用同样的方式缩短：最新的 4 条消息完整保留，更早的回答截到 800 个字符，更早的提问截到 2000 个字符。
- **AI 设置**（管理员）：
  - 页面总是能打开。顶部四个标记显示设置进行到哪一步（**连接已保存**、**模型已保存**、**测试成功**、**AI+ 功能已开启**），一行文字说明 ai-service 有没有连接（版本、最后一次报告）。全新安装时第一个标记是 **连接未设置**，在你保存连接之前不会请求模型列表（只保存对话保留天数不算连接）。没有连接时，**ai-service 连接状态** 面板会写明原因（没有运行、已停止、没有连接、在等它的第一次报告，或者主服务本身），并且每 10 秒重新检查一次；
  - OpenAI 兼容 API、Anthropic 或 Cloudflare AI Gateway（带它的认证方式和密钥）；保存后自动列出模型，放在一个可搜索的列表里。大概不是对话模型的模型（向量嵌入、重排序、安全分类、图像生成、语音、文档解析、仅图像）排在最后，带标签和提示；没有任何模型被隐藏。密钥输入框有 **显示密钥** / **隐藏密钥** 按钮；
  - 只有 **测试** 成功后才能开启 **AI+ 功能**。它开着时，保存改过的连接或模型会先询问，然后对所有人关闭它，并一直保持关闭，直到新的测试通过、你再把它开启（测试通过本身绝不会自动开启它；**测试** 下面那一行跟着开关变化）。密钥加密保存，以后不再显示，已保存的密钥不重新输入就绝不会发到新的地址；
  - 按用户、模型和时间统计的用量（默认 7 天；调用次数、输入和输出 Token、服务商从缓存里提供的 **缓存命中** Token，以及错误，从不包括内容）；
  - **AI+ 用户**（每个有 AI+ 功能的人）：**AI+ 功能** 一列显示 **使用中** 或 **已暂停**，带 **暂停** 和 **恢复** 按钮（它们从不改变用户管理里的开关），以及他们的对话和记忆的只读视图，可以用 **删除** 删掉；每次查看、删除、暂停和恢复都会写进系统日志。窗口窄于 900 px 时两列 Token 会省掉（它们在 **用量** 里）；窄于 560 px 时每个用户变成一张小卡片；
  - 对话保留（1 到 3650 天）和 **安全上限**（同时生成的回答数、对话数、消息数、记忆数、消息长度），它们用来保护服务器，不是用量限制。
- **每个用户两处设置，一个名字**：**AI+ 功能** 是各处统一的名字。管理员在 **AI 设置** 里为所有人开启它，并用 **用户管理** 里的开关把它开通给每个用户（每个用户那一行的开关，**添加用户** 里的勾选框）。只有 ai-service 已连接、功能已开启、用户有这个功能、而且没有被暂停时，用户才能使用 AI。**暂停** 会让正在生成的回答立即停止，用户仍然列在那里，数据也都保留。
  - 用户管理里的开关只在 ai-service 已连接、而且 AI+ 功能已开启时才出现。否则这些行里没有 AI+ 这一行，页面开头下面有一句说明原因，服务器也会拒绝开启或关闭它，提示 **请先在 AI 设置里开启 AI+ 功能。**（没有发生的修改绝不会提示成功）。已保存的开关保持不变。被暂停的用户，开关旁边会显示 **已暂停** 标记。
  - 每个现在能使用 AI 的人，名字旁边都有橙色的 **AI+** 徽章。深色的 **AI+** 徽章表示用户有这个功能，但现在不能使用 AI（被暂停、功能已关闭或 ai-service 没有连接；提示框里写着是哪一种，触屏上点一下就会显示；**设置** → **账户** 里会把原因写出来）；没有徽章表示没有这个功能。
  - 用户永远看不到容器的名字：对他们来说，所有故障都显示 **AI 暂时无法使用，请稍后再试或联系管理员。** 管理员看到的是 **ai-service 未连接。**
- **ai-service**：AI 作为第三个容器单独运行（文件夹 `ai-service/`，镜像 `xexam-ai-service`），有自己的数据库，放在自己的卷 `xexam_ai_data` 里。它从不接收登录 Cookie：Nginx 先问后端，然后只把一份简短的、签过名的「是谁在请求」的声明传给它。它是唯一能上网的容器；后端仍然不能上网，也从不调用它。ai-service 把自己的状态报告给后端（启动时、有变化时马上、每 25 秒，以及停止时）；超过 60 秒没有报告，后端就认为它没有连接。它停止时 Xexam 照常可用。系统日志在每次重新连接时写一条 `AI: AI link: connected to the main service`，连接断开时写 `AI: AI link: lost the main service`。
- **用户管理**：在 **添加用户** 和 **更改** 里，**管理题库**、**全选** 和 **全部不选** 排成一行，放在题库列表上方，**包括以后导入的题库** 是列表的第一项，说明都改成了提示框。**管理题库** 从 **编辑** 移到了这里。
- **学习记录**：每一行的 **删除** 按钮在窄窗口里不再折成两行。
- **新图标**：用于程序、浏览器标签页和手机主屏幕。
- **配置**：可选的 `XEXAM_AI_SECRET_KEY`（加密已保存的 AI 密钥所用的密钥；默认是 AI 卷里的一个密钥文件）、`XEXAM_INTERNAL_TOKEN`（后端和 ai-service 之间的令牌；默认是卷 `xexam_link` 里的一个随机令牌）、`XEXAM_AI_LOG` 和 `XEXAM_AI_SQLITE_JOURNAL`。从现在起请备份两个数据卷。
- **数据库 schema v5**：新增列 `users.ai_enabled` 和 `users.uid`；迁移前会把副本写到 `<数据目录>/backups/xexam-schema4-<日期时间>.db`。ai-service 有自己的数据库 `xexam-ai.db`（schema 2，带 `users.function_enabled`、`users.free_mode`、`messages.free` 和 `usage.tokens_cached`）；早期预发布版本的数据库里缺少的 schema 2 的列，会在启动时自动补上。两个服务都不会打开对方的数据库。
- **新 API**：`/api/ai/*`，由 ai-service 响应（`send` 和 `retry` 以 server-sent events 流式返回回答，并接受回复语言 `lang`；请求体里的 `free` 会被忽略）；`POST /api/ai/free`（管理员）和 `GET /api/ai/status` 里的 `freeMode`；`POST /api/ai/admin/suspend` 和 `resume`；已登录用户里新增 `aiAllowed`，`/api/auth/state` 里新增 `aiAvailable`（管理员还有 `aiService`），`/api/users`、`/api/users/create` 和 `/api/users/update` 里新增 `ai`、`aiSuspended` 和 `aiAvailable`；用量里新增 `tokensCached`；内部的 `POST /internal/ai/heartbeat`。
- 测试：后端 316 → 342，前端 139 → 246，AI 服务 160。

### v0.7.5 — 2026-11-11

**v0.7 之后的修正：多人同时使用时不再出现「连不上服务」，所有删除操作都用真正的红色，所有正确答案都是绿色，README 变短，另有一份单独的使用说明。数据库不变。**

- **请求太频繁**：Nginx 现在允许同一个地址每秒 30 个 API 请求通过，突发 120（以前是 10，突发 30）。在 Cloudflare Tunnel 这样的隧道后面，所有用户都来自隧道的同一个地址，共用这个额度，而光是系统日志页面每秒就要读取两次。Nginx 仍然拒绝请求（429）时，页面会自己把读取重试两次，然后显示 **请求太频繁**，而不再显示「连不上服务」和请你重新登录的提示。上传仍有自己的、更小的额度。
- **删除用红色**：删除按钮、每个删除对话框的确认按钮、错误提示、表单检查和系统日志里的错误，都用 GitHub 的红色，浅色和深色主题都一样。答错的答案仍是橙色，警告也保留自己的橙色。
- **所有正确答案都是绿色**：**答对** 标记、成绩条上的勾、热点题的 **答对** 标签，以及 **学习记录** 里 80% 及以上的正确率，都和正确选项一样是绿色。在 **你的答案** 那一行，多选题答案的每个字母单独显示绿色或橙色，所以部分答对的答案两种颜色都有。
- **公开信息里不再有题数**：`/api/info` 不再说有多少道题（`bankCount` 去掉了）；这个路由是公开的，登录后的页面会自己数它们能看到的题。它仍然会读取数据库，所以健康检查能发现坏掉的数据库。
- **更短的 README**：安装、配置、升级和这份更新日志。怎样使用 Xexam、Excel 格式和导入规则移到了 `docs/zh-CN/usage.md`；其他内容都在 `docs/` 下的其他文档里。
- **截图工具**：`python tools/screenshots/retake.py` 用一个装着虚构演示数据、用完即弃的实例，重新拍 README 和文档里三种语言的所有截图，最后自己清理干净。
- **审查 v0.7 后的修正**：
  - `docker/.env` 里空的 `XEXAM_LOG=` 不再把所有 info 和 warning 日志都藏起来：为空表示 `info`，无效的值退回 `info` 并记一条警告；
  - 由更早的版本加进 v0.7 预发布版数据库的用户，现在在转换这个数据库时也会得到所有题库，和升级时的用户一样；
  - 在主页上，题库里只有已下架题目的可管理题库的用户会被引到 **题库管理**，在那里可以更新自己的题库；
  - 系统日志页面的一次读取超过 10 秒就放弃，稍后再试；
  - 用户管理里 **更改** 和 **解锁** 按钮的无障碍名称包含了它们显示的文字；
  - 题库管理页上的 Excel 格式说明和解析器更一致了（`HOTSPOT` 是建议写的，不是必须的）。
- 测试：后端 314 → 316，前端 129 → 139。

### v0.7 — 2026-11-11

**每个用户能看到的题库可以单独设置，新增题库管理权限、练习模式，只能导入六列 Excel 格式，以及管理员专用的系统日志。数据库会先写一份副本，再自动升级到 schema v4，并改用 WAL 模式。**

- **按用户设置题库**：管理员在 **用户管理** 里为每个用户勾选他能看到的已有题库（带 **全选**），并单独设置 **包括以后导入的题库**，让他得到从那以后导入的每个题库。新用户在选好之前看不到任何题库；v0.7 之前就有的每个用户都得到每个已有题库和 **包括以后导入的题库**，管理员始终能看到全部题库。收回某个题库后，该用户的学习记录仍作为历史保留。
- **题库管理权限**：有新权限 **可以管理题库** 的用户（在 **用户管理** 里设置）可以导入新题库，并更新或删除自己导入的题库，但不能动管理员或其他用户的题库。没有这个权限的用户在菜单里看不到 **题库管理**，直接打开该页面会显示 **没有权限**。管理员能看到每个题库是谁上传的；删除上传过题库的用户时，这些题库会转给执行删除的管理员。
- **练习模式**：主页 **开始考试** 旁边新增 **开始练习**，题库、工作表、题数和顺序的设置与考试相同。每道题会马上核对（单选题选中选项时，其他题型点 **核对答案**），显示对错、正确答案、解析和记忆口诀。练习会作为单独类型的学习记录保存，并和考试一样计入错题本。**学习记录** 里每条记录都标有 **考试** 或 **练习**，也可以只看其中一种；主页的 **最近一次成绩** 显示两种记录里最新的一条；错题复习里的 **练习这些题** 现在会开始一次练习。
- **只能导入六列 Excel 格式**：A 英文题目、B 中文题目、C 日文题目；D 英文、E 中文、F 日文的答案、解析和记忆口诀。以英文为准：题型、选项、热点题的小项、题目标识和答案都取自 A 列和 D 列。中文或日文答案和英文不一致时，这一行仍按英文答案导入，并在预览里列出警告。三列和四列格式的文件会被拒绝，并有提示说明原因；以前从这些格式导入的每道题在每次启动时都会被下架，题库、学习记录和统计保留。请用工作簿的六列格式版本更新这样的题库：它的题会按英文文字重新识别，从没有答题记录开始。题库管理页上的说明只讲六列格式。
- **三种语言的答案和解析**：结果页、考试记录和错题复习里的每张题目卡片都有自己的 **English** / **中文** / **日本語** 切换，同时切换题目、选项、解析和记忆口诀；原来单独的中文翻译开关去掉了。所有题目语言切换在任何界面语言下都统一显示 **English** / **中文** / **日本語**。
- **绿色和橙色的答案**：在答案解析和练习中，正确选项是实心绿色，选错的选项是实心橙色，文字为白色，浅色和深色外观都一样。已评分的热点题小项改为中性底色。主页上 **开始考试** 是绿色，**开始练习** 是橙色。
- **系统日志**（菜单里，只有管理员可用）：后端启动以来记录的最后 2000 行，每半秒读取一次，可以 **暂停**、**跳到最新**、按级别筛选和搜索。日志里用用户名指代用户，从不写没有账户的、输入的名字。成批出现的被拒请求和登录失败每分钟汇总成一行，不会把其他日志挤掉。更早的日志：`docker compose logs backend`。
- **SQLite WAL 模式**：数据库改用 WAL 模式（仍然是 `synchronous=FULL`，已提交的修改不会丢失），后端正常停止时会做检查点。后端运行时复制备份，要连同 `xexam.db-wal` 和 `xexam.db-shm` 一起复制，或者先停止后端。网络文件系统等特殊存储可以在 `docker/.env` 里设 `XEXAM_SQLITE_JOURNAL=delete`，继续使用原来的模式。
- **时区**：时间戳默认跟随宿主机的时区（compose 文件会挂载宿主机的 `/etc/localtime`）；在 `docker/.env` 里设置 `TZ` 可以覆盖。
- **配置**：compose 文件也会把 `docker/.env` 里的 `XEXAM_COOKIE_SECURE` 和 `XEXAM_LOG` 传进去。日志和命令行程序的输出都是英语。
- **登录和用户**：密码输错后的锁定按账户记录，猜别的用户名也绕不过去；修改密码后，你在其他地方的登录会被登出；用户还有剩余次数时也会显示 **解锁**，锁定说明显示在该用户那一行下面；用户管理里统计的是每个用户的学习记录（考试和练习）。
- **版本文件**：`VERSION` 现在有两项：`CODE_VERSION`（代码的版本，和这份更新日志一起手动修改）和 `RELEASE_VERSION`（最近一次正式发布的版本）。发布任务在镜像和 GitHub Release 都发布成功之后，才会写入 `RELEASE_VERSION` 并更新 README 顶部的版本徽章。
- **数据库 schema v4**：新增表 `user_banks` 和列 `users.new_banks`、`users.can_manage_banks`、`banks.owner_id`、`attempts.kind`；迁移前会把副本写到 `<数据目录>/backups/xexam-schema3-<日期时间>.db`。v0.7 预发布版写入的数据库在打开时会被转换，转换前先写一份副本 `xexam-schema4-<日期时间>.db`。
- **新 API**：`GET /api/logs`（只有管理员可用）；`/api/users`、`/api/users/create` 和 `/api/users/update` 新增 `banks`、`newBanks` 和 `canManageBanks`；`/api/attempt`、`/api/records` 和 `/api/record` 新增 `kind`（`exam` 或 `practice`）。`POST /api/upload` 和 `POST /api/import/confirm` 以 400 拒绝三列或四列格式的工作簿。只有管理员能看到题库的上传者。
- 测试：后端 232 → 314，前端 59 → 129。

### v0.6 — 2026-11-11

**账户和登录：每个用户有自己的学习记录，用户和题库由管理员管理。数据库会先写一份副本，再自动升级到 schema v3。**

- **登录**：一个标准的登录页，和 v0.5.1 一样简洁柔和。第一次启动（新装或升级后）用 `admin` / `admin` 登录，接着会要求你设置自己的密码；在改掉之前，登录页和启动日志都会提醒你。
- **用户和角色**：管理员管理题库和用户；普通用户参加考试，只能看到自己的学习记录、错题本、没交卷的考试和设置。题库由所有用户共用。v0.6 之前的数据归第一个管理员所有。
- **用户管理**（菜单里，只有管理员可用）：添加、编辑、重置密码、停用 / 启用、删除和解锁用户，并显示每个用户的角色、状态（正常、已停用、已锁定）、创建时间、最近登录时间和考试场数。管理员新建或重置过密码的用户，下次登录时要设置自己的密码。
- **页头**：菜单按钮左边显示你的用户名（管理员带 **管理员** 标签）；菜单只列出你的角色能用的页面，最后是 **退出登录**。
- **界面设置**：新增 **账户** 部分，可以 **修改密码**。
- **密码输错**：先有 5 次机会，用完锁 1 分钟；之后每轮 3 次机会，锁定时间依次是 5 分钟、15 分钟、30 分钟、1 小时和 1 天。登录页会显示还要等多久，管理员可以解锁。
- **安全**：密码用 Argon2id 哈希保存；会话 Cookie 是 `HttpOnly`、`SameSite=Strict`（数据库只存它的 SHA-256），有效期 30 天，密码、角色或状态变化时会被登出。纯 HTTP 照样能用（只有 HTTPS 或设了 `XEXAM_COOKIE_SECURE=1` 时才加 `Secure`）。
- **进不了管理员账户时**：`docker compose exec backend xexam-backend reset-password admin` 会打印一个新的临时密码。
- **数据库 schema v3**：新增表 `users`、`sessions`、`user_stats`、`user_settings` 和列 `attempts.user_id`；迁移前会把副本写到 `<数据目录>/backups/xexam-schema2-<日期时间>.db`。
- **新 API**：`/api/auth/state`、`/api/auth/login`、`/api/auth/logout`、`/api/auth/password`、`/api/users`、`/api/users/create`、`/api/users/update`、`/api/users/password`、`/api/users/delete` 和 `/api/users/unlock`；其他路由都要求已登录（`/api/info` 仍然公开）。
- **仓库结构**：日语和中文的 README 移到 `README/`，compose 文件和 `.env.example` 移到 `docker/`（克隆的仓库里 `.env` 现在放在 `docker/.env`）。README 的徽章改成圆角，版本徽章会显示版本号。
- 测试：后端 181 → 232，前端 44 → 59。

### v0.5.1 — 2026-11-11

**更柔和、更简洁的外观，改用对色盲友好的配色，菜单按钮移到右上角，题库管理页和主页一样紧凑。**

- **更柔和、更简洁的外观**：配色改为参照 GitHub 的 Primer 设计系统里对色盲友好的浅色和深色配色（Light colorblind / Dark colorblind）：答对用蓝色、答错用橙色，不再是绿色和红色；浅色主题是柔和的灰白底配白色区块和藏蓝色的主要按钮，深色主题接近纯黑。页面铺在一整张平静的底上；外框和边线更少，改用圆角，鼠标移上去时显示柔和的背景色，提示改用带浅色底色的色块显示，菜单更轻，带一层柔和的阴影。变的只是外观，位置和用法并没有因此改变。
- **菜单移到右边**：三条横线的菜单按钮现在在页头的右上角；Xexam 标志还在左上角。菜单在按钮下方打开，用法和以前一样。
- **更紧凑的题库管理页**：
  - **导入 Excel**：**新建题库** 和 **更新已有题库** 是标题旁边的两个小按钮（悬停提示里有说明）；选更新时，下面会出现 **要更新的题库**。只有一行提示，末尾是 **查看 Excel 写法** 链接；选文件也只占一行：点 **选择 Excel 文件**，或者把文件拖到这个面板的任意位置。
  - **预览**：各项数字是几个小标签（为 0 的不显示），每个工作表一行，工作表里读不出来的行收在它下面。和以前一样，确认和取消按钮紧跟在数字和提示后面，在各个工作表之前。
  - **已有的题库**：每个题库占紧凑的一行，有名字、**更新** 和 **删除** 两个小按钮，以及一行概况（文件、题数、工作表数、最近更新时间）。工作表收在这一行右边的箭头下面。
  - **导入记录** 每次导入一行（最近 8 次），**Excel 题库的写法和导入说明** 在你打开之前一直是收起的。
  - 在触摸屏上，这些小按钮的点按区域仍有约 44 px；在手机上，题库名字很长时，按钮会换到单独的一行。
- **镜像发布到 Docker Hub**：每个版本现在也发布到 Docker Hub，名字是 `yuminspace/xexam-frontend` 和 `yuminspace/xexam-backend`（公开，同一个标签下有 `amd64` 和 `arm64`）。`docker-compose.yml` 改为从那里拉取，所以不再需要 `docker login`。同样的标签仍保留在 `ghcr.io/yumin-space` 上，作为私有镜像副本。如果你还在用 v0.5 的旧 `docker-compose.yml`，请换成新的（数据卷会保留）。三份 README 现在以 Docker Hub 的徽章开头。
- README 和文档都按新布局改写了，截图也重新截了。

### v0.5 — 2026-11-10

**新界面：可以有多个题库，有学习记录和界面设置，界面有英语、中文、日语三种。数据库会在写好一份备份副本之后自动升级到 schema v2。**

- **新布局**：除了考试页，每个页面都有一个窄窄的页头，上面有三条横线的菜单按钮。点它会打开一个小菜单，里面有主页、题库管理、学习记录和界面设置；选好一项、点菜单外面或按 Esc 键，菜单就会关上。在宽屏上，页面内容居中显示（最宽 1120 px）。考试本身是一个专注的全屏页面。
- **关于页面**：菜单的最后一项 Xexam v0.5（在分隔线下面）会打开它。它显示版本、服务器的 CPU 架构、许可证，以及每个版本的更新内容：也就是这份更新日志，用界面语言显示，最新的版本展开，更早的版本折叠起来。
- **更紧凑的主页**：没交卷的考试是一行横幅，带继续作答和放弃这场。开始考试面板右上角的两个小按钮选择从哪里出题：全部和错题本，各自显示题数（所选工作表里没有要复习的错题时，错题本不能选，并用一行字说明原因）。每个题库是一张卡片，里面一行一个工作表；题库的复选框会勾选它的全部工作表，工作表也可以收起来（题库超过两个时，一开始是收起的）。
- **随机抽题**：题数比所选的题少时，现在会从整个所选范围里随机抽题（以前不打乱顺序时，取的是按题库顺序排在最前面的那些题）。打乱题目顺序只决定这些题按随机顺序出，还是按题库里的顺序出。
- **多个题库**：每份导入的工作簿都是一个独立的题库。可以把文件导入成新题库，也可以更新 / 替换已有的题库；文件里没有的题只在那个题库里下架。题库列表显示每个题库的工作表和题数。删除题库前会先确认，并可以选择同时删除这个题库的学习记录。
- **学习记录页**：考试列表，可以按题库、日期范围、正确率筛选；每场考试的详情，显示每道题你的答案、正确答案和解析（v0.5 起考的场次）；错题复习和练习；重做错题或重考一场；删除一条记录（错题本不变）或全部记录（可以选择同时清空错题本）。Xexam 保留最近 500 场考试（以前是 60 场）。
- **界面设置页**：主题（跟随系统 / 浅色 / 深色）、界面语言（默认 English，另有中文、日本語）、答完自动跳到下一题（单选题，以及每个小项都答完的热点题；多选题永远不会自动跳转）、恢复默认设置。设置和题目语言保存在服务器的数据目录里，所以重启、更新镜像、清除浏览器缓存、换别的浏览器或设备之后都还在。没交卷的考试仍然只保存在当前浏览器里。
- **整个界面都有英语、中文、日语三种**，包括答题页和服务器返回的错误提示（后端仍然用中文发送，由页面翻译）。
- **更适合手机**：主页上的开始考试按钮固定在屏幕底部，列表更紧凑，答题卡把已答题数和交卷按钮放在题目小方块上面，触控区域至少 44 px。
- **数据库 schema v2**：新增 `banks`、`attempt_items`、`settings` 三张表，以及 `questions.bank_id`、`attempts.banks`、`imports.bank_id` 三个列。v0.5 第一次启动时会自动迁移现有的数据库；迁移之前，先把一份副本写到 `<数据目录>/backups/xexam-schema1-<日期时间>.db`（副本写不出来时，后端不会启动，也不会改动任何东西）。现有的数据变成一个题库；题目 id、错题本和成绩历史都保留。回退 = 停止、恢复那份副本、运行 v0.4。不要在有多个题库的数据库上运行 v0.4。步骤见 [docs/zh-CN/migration.md](../docs/zh-CN/migration.md)。
- **新接口**：`GET /api/banks`、`POST /api/banks/delete`、`GET /api/records`、`GET /api/record?id=`、`POST /api/records/delete`、`GET` / `POST /api/settings`、`POST /api/settings/reset`，以及 `POST /api/upload` 上可选的请求头 `X-Bank-Target`（不带它时，上传的行为和 v0.4 完全一样）。`GET /api/bank` 里的每道题多了一个键 `bank`。见 [docs/zh-CN/api.md](../docs/zh-CN/api.md)。
- **文档按语言分开**：`docs/en`、`docs/ja` 和 `docs/zh-CN`；只有中文的历史文档移到了 `docs/zh-CN/history`。每个语言文件夹都有自己的截图，截图里的界面就是那种语言。CI 会检查这个结构和每一个相对链接。
- **推送到 `dev` 分支时也会跑 CI**（只检查和测试，从不发布）。
- README、文档和四列格式解析测试里的 Excel 示例都换成了虚构的示例；`tests/baseline/` 里的旧版测试快照没有改动。较早的更新日志条目里，只改了链接（指向新的文档文件夹）和一组示例词。
- **新增 MIT 许可证**（`LICENSE`）。
- 测试：后端 151 → 181（迁移和备份、题库、记录、设置、新接口上的守卫），前端 27 → 44（界面文字、服务器提示的翻译、关于页面上的更新日志、随机抽题）。

### v0.4 — 2026-11-10

- **三种语言对照：英语 / 中文 / 日语。** Excel 新增一个格式：A 列英语、B 列中文、C 列日语（题目本身）、D 列答案·解析·口诀；A、B、C 三列是同一道题的三种语言，一一对应，答案只写一份。答题页的语言按钮变成「英语 / 中文 / 日语」，成绩页的错题解析也用你选的语言显示。
- **热点题的选项也跟着语言走**：每个小项末尾用本语言写选项（日语 `はい / いいえ`，中文 `是 / 否`，英语 `Yes / No`；执行层之类的三选一同理），切换语言时选项按钮一起换。某一列没写选项时，这个语言仍可用，选项按钮显示日语的。
- **热点题的答案用任何一种语言的选项词来写都认**（`調理`、`烹饪`、`Cooking`，也可以混着写）；三种语言的选项按位置一一对应，个数对不上的那一列不参与，不会猜。
- **旧的三列格式（A 日语、B 中文、C 答案）不用改，照样能上传**，只是没有英语。程序看内容自动判断是哪种格式；四列格式里题目 id 仍然来自 C 列的日语，所以同一道题在两种格式里 id 相同，学习记录不受影响。
- 首页页脚的说明文字改为：Xexam 仅提供题目与答案的导入、整理及练习功能，软件本身不提供或收录考试题目、答案及相关资料；用户应确保导入和使用的资料来源合法，并拥有相应使用权限。
- **README 拆成三份**：`README.md`（英语，默认）、`README.ja.md`（日语）、`README.zh-CN.md`（中文），页面顶部的语言栏可以互相跳转；各自有自己语言的更新日志，GitHub Release 说明取自英语那份。
- **`docs/` 里的文档也分成三种语言**：现行的七份（架构、API、数据库、安全、部署、迁移、开发）各有 `X.md`（英语）、`X.ja.md`（日语）、`X.zh-CN.md`（中文），顶部有语言栏；`openapi.yaml` 改为英语。Phase 0 的审计和计划文档是历史资料，仍只有中文。CI 会检查三份是否齐全，以及所有相对链接是否都指向存在的文件。
- **换了新截图**：用 v0.4 的界面和虚构的四列演示题库重新截的，同一道题有英语、中文、日语三种显示，热点题的选项按钮也跟着语言走。（界面本身，按钮和标签，仍然只有中文。）
- **语言约定**：代码注释和程序、工具、CI 的输出一律用英语；界面和 README 用各自对应的语言。这次没有改动任何代码（已和上一个提交做过机械比对）。
- 测试：后端 136 → 151（四列格式、三种语言的选项与答案、对不上时的退回），前端 19 → 27（语言规则）。

### v0.3 — 2026-11-10

**整个程序重写成 Rust + TypeScript + Nginx。数据库文件和 schema 没有变，API 向后兼容；旧的 Python 实现已从仓库里移除。**

- **后端改成 Rust**（Axum 0.8 + Tokio + rusqlite，工具链固定 1.99.1）：REST 接口、XLSX 解析、导入（预览不写库 → 一次性确认码 → 一个事务写入）、成绩与错题统计、Host/Origin 守卫、`xexam-backend import / healthcheck / migrate-progress` 子命令。行为与 Python 版一一对应：172 份旧版响应逐项对照没有未解释的差异；49 个合成工作簿的解析输出、85 条题目 id 向量全部一致，所以题目 id 与旧版逐字节相同、学习记录不会脱钩。
- **前端改成 TypeScript**（strict）+ **Vite**，原生 DOM、没有框架；没有内联样式，所以 CSP 可以很严。构建目标 `safari15`，iPhone 需要 iOS 15.4 以上。
- **两个镜像、两个容器**：`xexam-frontend`（Nginx + 前端静态文件，唯一对外）和 `xexam-backend`（Rust，在 `internal` 私有网络里不发布端口）。非 root、`cap_drop: ALL`、只读根文件系统；Host 原样透传、`X-Forwarded-*` 清空；20 MB 上限、安全头、限流、超时；后端不可达时返回 JSON 502。两个镜像都同时有 `amd64` 和 `arm64`。
- **更严的上传检查（有意的差异）**：任何编码 / 扩展名里的 DOCTYPE 和实体声明都拒绝；重复的 ZIP 条目名拒绝；ZIP 条目数、XML 深度 / 节点数 / 每元素属性数、单元格总字节数有上限；解析有协作式的超时（到了解析器自己停下）且同时最多 2 个；错误文案不再回显库的异常文本；不支持的 HTTP 方法回 405（以前是 501）。完整差异表见 [docs/zh-CN/api.md](../docs/zh-CN/api.md)。
- **一个流水线**（`.github/workflows/ci.yml`）：检查版本和更新日志 → 前端 / 后端 / Nginx 与整栈 / arm64 四组检查 → 都通过才发布两个镜像并建 Release。
- **测试**：后端 136 个（对着已提交的旧版快照：172 份黄金响应、49 个解析夹具、85 条 id 向量）、前端 19 个；`tests/integration/` 里 Nginx 38 项、整栈 34 项、`progress.json` 迁移 10 项。
- **文档**：`docs/` 下的架构、API（含 OpenAPI）、数据库、安全、部署、迁移、开发，以及迁移前的审计、风险和计划。
- 判分仍在浏览器里（服务端只收 `{题目 id, 对错}`），理由见 [docs/zh-CN/history/migration-plan.md](../docs/zh-CN/history/migration-plan.md)。
- ⚠️ **升级注意**：新旧版容器名和端口相同，**不能同时运行**，升级前先 `docker compose stop`；数据卷 `xexam_data` 是同一个，数据不用迁移。旧的单镜像 `ghcr.io/yumin-space/xexam` 不再更新（最后是 `v0.2.2`），`docker-compose.yml` 现在一次拉取 `xexam-frontend` 和 `xexam-backend` 两个。回滚步骤见 [docs/zh-CN/migration.md](../docs/zh-CN/migration.md)。
- **没有验证的东西（如实写）**：真正的 Cloudflare 隧道、iPhone 真机、在真正的 ARM 硬件上运行（只在 QEMU 下构建并启动验证过）、用你自己真实数据库的副本演练。

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
