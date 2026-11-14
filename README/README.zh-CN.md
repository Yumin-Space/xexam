[English](../README.md) | [日本語](README.ja.md) | **简体中文**

<!-- The release badge shows RELEASE_VERSION from VERSION, the last published release. Do not edit it by hand: the release job of .github/workflows/ci.yml writes it (scripts/set-release-version.py) after the images and the GitHub Release are published. GitHub rounds an image's corners only when it has both width and height, so every badge carries its exact size (in px, from shields.io). -->
<p>
  <a href="../../../releases/latest"><img width="132" height="28" alt="发布版本 v1.1" src="https://img.shields.io/badge/release-v1.1-green?style=for-the-badge"></a>
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

![答题页：一道显示成中文的热点题、English / 中文 / 日本語 切换按钮，以及带「完成考试」按钮的答题卡](../docs/zh-CN/screenshots/desktop-2-quiz.png)

仓库里**没有题目**。Xexam 只提供导入、整理和练习题目的工具；题库是你自己的 Excel 文件，不会被提交（`.gitignore` 已排除）。截图里用的是虚构的演示题库，不是任何真实考试的内容。

| | |
|---|---|
| 当前版本 | **v1.2**（见 [更新日志](../docs/zh-CN/changelog.md)，也可以在仓库的 [Releases](../../../releases) 页看；在程序里：菜单 → **Xexam v1.2**） |
| 镜像 | 共三个，在 Docker Hub 上（公开，不用登录）：`docker.io/yuminspace/xexam-frontend`、`docker.io/yuminspace/xexam-backend` 和 `docker.io/yuminspace/xexam-ai-service`，标签 `latest` / `v1.2` / `sha-<提交>`（`sha-` 标签用来排查问题）。镜像副本：`ghcr.io/yumin-space/xexam-frontend`、`ghcr.io/yumin-space/xexam-backend` 和 `ghcr.io/yumin-space/xexam-ai-service`（私有，标签相同） |
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
- **学习时的 AI 帮助**（给有 **AI+ 功能** 的用户；考试进行中没有）：练习时，**AI 提示** 和 **AI 解释** 都在题目下面的一个面板里；成绩页和学习记录的详情页上，每道题都有 **AI 解析**，在 **学习记录** 和 **主页** 上，可以对一条记录里答错的题生成 **AI 总结**。每个回答都保存在 **AI 对话** 里，面板里的任何东西都不会离开页面。每个用户对 AI 的请求都有每分钟、每小时和每天的上限，由管理员设置。
- **手机优先**：每个页面在手机宽度下都不会横向滚动；只有图标的按钮按住就能看到名字，触控区域至少 44 px；每个页面末尾有一行居中的、带项目链接的小字，问题和回答获得大部分空间。宽屏上页面居中显示。

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
| 固定在某个版本 | 把 `docker-compose.yml` 里三个镜像的 `:latest` 都改成 `:v1.2`，再 `docker compose up -d` |
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

每次升级端口和卷 `xexam_data` 都保持不变：先 `docker compose pull`，再 `docker compose up -d`。请先备份；各版本的步骤、备份副本和所有回滚方法见 [docs/zh-CN/migration.md](../docs/zh-CN/migration.md)。

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
