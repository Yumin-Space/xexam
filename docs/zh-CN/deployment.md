[English](../en/deployment.md) | [日本語](../ja/deployment.md) | **简体中文**

# 部署

> 新架构：`nginx`（唯一对外）+ `backend`（Rust，私有网络）+ SQLite（命名卷 `xexam_data`）。结构图见 [architecture.md](architecture.md)，安全边界见 [security.md](security.md)。

## 1. 前提

- 装了 Docker（含 Compose v2）的机器，x86（amd64）或 ARM（arm64）都行。
  - arm64 镜像在 CI 里用 QEMU **构建并启动**验证过（`/api/info` 报 `aarch64`，Nginx `/healthz` 正常）；**没有在真正的 ARM 硬件上跑过**。
  - 后端是在 x86 构建机上用 zig 交叉编译出 aarch64 版本的（冷缓存下 CI 约 5 分钟，不需要在 QEMU 里编 Rust）。
    因为 zig 的链接器不接受 rustc 默认给 aarch64 加的 `--fix-cortex-a53-843419`，aarch64 产物**不带**这个针对 Cortex-A53 核心的勘误规避。
    Oracle Cloud 的 Ampere A1 用 Ampere Altra（据 Wikipedia 是 Arm Neoverse N1 核心），不是 Cortex-A53，所以不受影响；
    如果你要在 Cortex-A53 核心的板子上跑（部分老款开发板），请别用这个镜像，先告诉我。

## 2. 启动

### 拉取现成的镜像（推荐）

只需要仓库里的 [`docker-compose.yml`](../../docker-compose.yml) 这一个文件：

```bash
docker login ghcr.io -u Yumin-Space      # 第一次；镜像是私有的，密码填有 read:packages 权限的令牌
docker compose up -d
```

浏览器打开 <http://localhost:8765>，点左上角三条横线的菜单按钮，选 **Question banks**（题库管理），导入你的 Excel 文件。界面默认是英文，可以在 **Settings**（界面设置）里切换成中文或日文。上面的命令会拉取并启动**两个容器**：`xexam-frontend`（Nginx，唯一对外）和 `xexam-backend`（Rust，私有网络）。

### 从源码构建（改代码时用）

```bash
git clone git@github.com:Yumin-Space/xexam.git
cd xexam
docker compose -f docker-compose.build.yml up -d --build
```

第一次构建要几分钟（编译 Rust 和前端），之后有缓存。

构建 `xexam-frontend` 时，`nginx/Dockerfile` 会把仓库根目录的 `README.md`、`README.ja.md` 和 `README.zh-CN.md` 复制进构建环境（`COPY README.md README.ja.md README.zh-CN.md /`）：「关于 Xexam」页（菜单底部的「Xexam v0.5」）里的「更新内容」，就是在构建时从这三份 README 的更新日志生成的。所以 `.dockerignore` 不再排除 `README.md`。最终镜像里只有生成的静态文件，没有 README 本身。

| 操作 | 命令 |
|---|---|
| 看状态 | `docker compose ps`（两个容器都应是 `healthy`） |
| 看日志 | `docker compose logs -f` |
| 停止 / 再启动 | `docker compose stop` / `docker compose start` |
| 更新到最新版本 | `docker compose pull && docker compose up -d`（源码构建用：`git pull && docker compose -f docker-compose.build.yml up -d --build`）。从 v0.4 升级：先看第 6 节 |
| 版本和架构 | <http://localhost:8765/api/info> |

> ⚠️ 不要运行 `docker compose down -v`：`-v` 会删除数据卷 `xexam_data`，题库和学习记录都在里面。

**`docker-compose.yml` 和 `docker-compose.build.yml` 用同一个项目名（`xexam`）、同一个卷（`xexam_data`）和同样的容器名**，所以两者不能同时运行；切换前先 `docker compose stop`。数据互通，见 [migration.md](migration.md)。

## 3. 配置（`.env`）

在 `docker-compose.yml` 旁边建 `.env`（不会被提交，也不会进镜像；模板是 [`.env.example`](../../.env.example)）：

| 变量 | 默认 | 作用 |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | 宿主机上发布的「地址:端口」，对应 Nginx。必须写成 `IP:端口`。`0.0.0.0:8765` = 局域网可连。⚠️ 只写端口号 = 所有网卡 |
| `XEXAM_ALLOWED_HOSTS` | 空 | 允许用哪些域名 / IP 访问，逗号 / 空格 / 分号分隔 |
| `XEXAM_ALLOWED_ORIGINS` | 空 | 代理把 Host 改写成源站地址时：填公网域名（含 `https://`） |
| `XEXAM_ALLOW_ANY_HOST` | `0` | `1` = 不检查 Host（只在前面已有身份验证时） |
| `XEXAM_PARSE_TIMEOUT_SECS` | `30` | 解析一份 Excel 的最长时间 |
| `TZ` | `Asia/Tokyo` | 数据库里时间戳用的时区 |

改了 `.env` 之后：`docker compose up -d`（会按需重建容器）。

## 4. 让手机 / 外部访问

### 同一个 Wi-Fi 里的 iPhone

只在**信任的网络**里做。`.env`：

```ini
XEXAM_BIND=0.0.0.0:8765
XEXAM_ALLOWED_HOSTS=192.168.1.20      # 电脑的局域网 IP
```

iPhone 的 Safari 打开 `http://192.168.1.20:8765`（需要 iOS 15.4 以上）。

### 通过 Cloudflare Tunnel 从互联网访问

后端本来就不对外，隧道只需要指向 **Nginx**（`XEXAM_BIND` 发布的端口）。三件事：

1. cloudflared 连得到 Nginx：同一台机器就保持默认并指向 `http://localhost:8765`；在局域网别的机器上就把 `XEXAM_BIND` 设成 `0.0.0.0:8765`（或具体的局域网 IP）。
2. Public Hostname：`xexam.你的域名.com` → Service `HTTP`（不是 HTTPS）→ `主机IP:8765`。
3. Cloudflare Access 加一条策略，**只允许你自己**。没设策略的域名等于对全世界公开。

Host 的两种常见情形（`.env`）：

| 隧道的行为 | 设置 |
|---|---|
| Host 原样保留，就是你的域名 | `XEXAM_ALLOWED_HOSTS=xexam.你的域名.com` |
| 设了 `httpHostHeader`，Host 被改成 `192.168.1.20:8765` | `XEXAM_ALLOWED_HOSTS=192.168.1.20` + `XEXAM_ALLOWED_ORIGINS=https://xexam.你的域名.com` |
| 不确定 | 先配第一种，被拒（403）时页面上会写出它看到的 Host / Origin，加进对应选项再 `up -d` |

Nginx 把浏览器（或隧道）发来的 `Host` **原样**转给后端，并清空 `X-Forwarded-*`；后端只读真实的 `Host`。

⚠️ 程序自己没有登录：只让 cloudflared 连得到这个端口，路由器上不要做端口映射；Linux 上主机防火墙拦不住 Docker 发布的端口，用 `DOCKER-USER` 链或具体的局域网 IP。

验证到什么程度（如实写）：

- 迁移时用真实 Chrome + HTTPS 反向代理，对**旧的 Python 版**做过两种 Host 情形（保留 Host / 改写 Host）的整套浏览器流程；那套脚本依赖旧 Python 代码，已随它移除。
- 对**新栈**：`tests/integration/stack_check.py` 用请求头级别的检查验证「经过 Nginx 之后，Host / Origin 守卫仍然生效、`X-Forwarded-*` 不被信任」；`nginx_check.py` 验证 Host 原样透传。
- **没有**用真正的 Cloudflare 测过。第一次上线时请按上表的「不确定」一行走，看页面给出的 403 提示。

## 5. 数据

- 位置：命名卷 `xexam_data` → 容器内 `/data/xexam.db`。更新、重建、删除容器都不丢。数据库里存着题库、学习记录，从 v0.5 起还有设置。
- 从 v0.5 起，这个卷里还可能有一个文件夹 `/data/backups/`，放的是 schema 迁移前写下的副本（见第 6 节）。程序从不删除这些文件。查看方法：`docker run --rm -v xexam_data:/data alpine ls -l /data/backups`。
- 备份与恢复：见 [database.md](database.md) 的第 5 节。备份卷时 `backups/` 也包含在内。
- 不用 Docker 直接运行后端（开发）：见 [development.md](development.md)。

## 6. 从 v0.4 升级到 v0.5

对部署来说，v0.5 什么都没变：还是那两个镜像、同一个 `docker-compose.yml`、同一个端口、同一个卷，也没有新的环境变量。变的是界面（菜单按钮和下拉菜单、题库管理、学习记录、界面设置、「关于」页）和数据库 schema。

1. 先备份卷，做法见 [database.md](database.md) 的第 5 节。程序自己也会写一份副本，但放在卷外面的副本更保险。
2. 照常更新：`docker compose pull && docker compose up -d`。
3. 第一次启动时，后端会把数据库升级到 schema v2：
   - 先把旧数据库复制一份到 `/data/backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db`。如果这份副本写不出来（磁盘满了、没有权限），后端会报错退出，数据库不会被改动。Docker 会一直重启它，每次启动都会再试一次；Nginx 不会启动，因为它要等后端 healthy。原因可以用 `docker compose logs backend` 查看。
   - 然后在一个事务里完成迁移。你现有的题会变成一个题库，题目 id 不变，所以错题本、统计和考试历史都保持原样。
   - 日志里 `Upgraded the database to schema 2. A copy of the previous version was saved first: …` 这一行会写出副本的文件名。
4. 检查：`/api/info` 显示 `"version": "0.5"`，`bankCount` 和升级前一样；**Question banks**（题库管理）里只有一个题库，包含你所有的题；**Study records**（学习记录）里列出你以前的考试。

之后再启动不会再迁移，也不会多写任何东西。

**回退**到 v0.4，就是恢复那份副本，再运行 v0.4 的镜像（标签 `v0.4`）。一旦有了不止一个题库，就不要在迁移过的数据库上运行 v0.4：v0.4 会把所有题库当成一个，下一次导入会把其他题库的题下架。具体命令见 [migration.md](migration.md) 的第 1.4 节，背景见 [database.md](database.md) 的第 6 节。

## 7. 健康检查与停止

- `xexam-frontend`：`GET /healthz`（不经 Host 白名单）。`xexam-backend`：`xexam-backend healthcheck`（容器内自己带 `Host: 127.0.0.1` 访问自己）。后端 healthy 之后 Nginx 才启动（`depends_on`）。
- 后端挂了或重启时，Nginx 不会退出，只是 `/api/` 暂时回 JSON 的 502（`后端暂时连不上`），后端恢复后会自动恢复正常。
- `docker compose stop`：后端在 8 秒内优雅退出；导入的事务要么提交完要么回滚。

## 8. 资源和镜像

| | |
|---|---|
| `xexam-frontend` | nginx-unprivileged + 静态文件；监听 8080；只读根文件系统 |
| `xexam-backend` | 静态链接的单个可执行文件 + tzdata；uid 10001；只读根文件系统，只有 `/data` 和 `/tmp` 可写 |
| 网络 | `internal`（后端 + Nginx，不能访问外网）和 `public`（只有 Nginx） |

## 9. 发布

推送到 `main` 时，[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) 先跑版本、更新日志和文档检查，再跑四组检查（前端、后端、Nginx 与整栈、arm64）。全部通过后，才把 `xexam-frontend` 和 `xexam-backend` 两个镜像（amd64 + arm64）发布到 `ghcr.io/yumin-space/`，标签为 `latest`、`v<VERSION>` 和 `sha-<提交>`，并建对应的 GitHub Release。推送到 `dev` 和指向 `main` 的拉取请求会跑同样的检查，但从不发布。详见 [development.md](development.md) 的第 7 节。

第一次发布一个新镜像名时，要在 GitHub 的包设置里把本仓库加进 **Manage Actions access**，角色选 **Admin**，否则推送会被拒绝（`permission_denied: write_package`）。
