[English](../en/deployment.md) | [日本語](../ja/deployment.md) | **简体中文**

# 部署

> 新架构：`nginx`（唯一对外）+ `backend`（Rust，私有网络）+ SQLite（命名卷 `xexam_data`）。结构图见 [architecture.md](architecture.md)，安全边界见 [security.md](security.md)。

## 1. 前提

- 装了 Docker（含 Compose v2）的机器，x86（amd64）或 ARM（arm64）都行。
  - arm64 镜像在 CI 里用 QEMU **构建并启动**验证过（`/api/info` 报 `aarch64` 和正确的版本，Nginx `/healthz` 正常）；**没有在真正的 ARM 硬件上跑过**。
  - 后端是在 x86 构建机上用 zig 交叉编译出 aarch64 版本的（冷缓存下 CI 约 5 分钟，不需要在 QEMU 里编 Rust）。
    因为 zig 的链接器不接受 rustc 默认给 aarch64 加的 `--fix-cortex-a53-843419`，aarch64 产物**不带**这个针对 Cortex-A53 核心的勘误规避。
    Oracle Cloud 的 Ampere A1 用 Ampere Altra（据 Wikipedia 是 Arm Neoverse N1 核心），不是 Cortex-A53，所以不受影响；
    如果你要在 Cortex-A53 核心的板子上跑（部分老款开发板），请别用这个镜像，先告诉我。
- 宿主机上有 `/etc/localtime`（常见的 Linux 发行版都有）。没有的话，见第 3.1 节。

## 2. 启动

### 拉取现成的镜像（推荐）

只需要仓库 `docker/` 文件夹里的 [`docker-compose.yml`](../../docker/docker-compose.yml) 这一个文件（下载到单独的文件夹里，例如 `curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml`，然后在那个文件夹里运行命令）。它从 Docker Hub 拉取两个镜像（`docker.io/yuminspace/xexam-backend` 和 `docker.io/yuminspace/xexam-frontend`），这两个镜像是公开的，所以不需要 `docker login`。同样的标签也在 `ghcr.io/yumin-space/` 上，作为私有镜像副本（用它要先 `docker login ghcr.io`，密码填一个有 `read:packages` 权限的经典令牌，即 Tokens (classic)）。

```bash
# 不需要 docker login：镜像在 Docker Hub 上是公开的
docker compose up -d
```

`docker compose up -d` 拉取并启动**两个容器**：`xexam-frontend`（Nginx，唯一对外）和 `xexam-backend`（Rust，私有网络）。然后用浏览器打开 <http://localhost:8765>，用 `admin` / `admin` 登录；Xexam 会马上请你设置自己的密码（第 10 节）。打开菜单（右上角三条横线的按钮），选 **Question banks**（中文界面里叫 **题库管理**），导入你的 Excel。界面一开始是英语；可以在 **Settings**（中文界面里是 **设置**）里，或者在登录页上换成中文或日语。

### 从源码构建（改代码时用）

```bash
git clone git@github.com:Yumin-Space/xexam.git
cd xexam
docker compose -f docker/docker-compose.build.yml up -d --build
```

第一次构建要几分钟（编译 Rust 和前端），之后会用缓存。克隆下来的仓库里，compose 文件和 `.env` 都在 `docker/` 里；`docker/docker-compose.build.yml` 以仓库根目录为构建上下文。下面的命令要在仓库根目录运行，并加上 `-f docker/docker-compose.build.yml`（例如 `docker compose -f docker/docker-compose.build.yml logs -f`）。

前端镜像需要三份 README：**关于 Xexam** 页面显示它们的更新日志，这些更新日志是构建进前端的。所以 `nginx/Dockerfile` 把 `README.md`、`README/README.ja.md` 和 `README/README.zh-CN.md` 复制进它的构建阶段，`.dockerignore` 也把 `README.md` 和 `README/` 留在构建上下文里。最终镜像里只有构建出的静态文件。

下面的命令用的是 Compose 的**服务名** `backend` 和 `frontend`。容器的名字是 `xexam-backend` 和 `xexam-frontend`；普通的 `docker` 命令（`docker logs`、`docker exec`）要用容器名。

| 操作 | 命令 |
|---|---|
| 查看状态 | `docker compose ps`（两个容器都应该是 `healthy`） |
| 看日志 | `docker compose logs -f`（只看后端：`docker compose logs -f backend`；见第 9 节） |
| 停止 / 再启动 | `docker compose stop` / `docker compose start` |
| 更新到最新版本 | `docker compose pull && docker compose up -d`（源码构建：`git pull && docker compose -f docker/docker-compose.build.yml up -d --build`）。从较早的版本升级时先看第 6 节 |
| 版本和架构 | <http://localhost:8765/api/info>，或者在程序里：菜单 → **Xexam v0.7**（关于页面） |

> ⚠️ 不要运行 `docker compose down -v`：`-v` 会删掉数据卷 `xexam_data`，题库、账户和所有学习记录都在里面。

**`docker/docker-compose.yml` 和 `docker/docker-compose.build.yml` 用的是同一个项目名（`xexam`）、同一个卷（`xexam_data`）和同样的容器名**，所以两者不能同时运行；切换前先 `docker compose stop`。数据是两者共用的；见 [migration.md](migration.md)。

## 3. 配置（`.env`）

在 `docker-compose.yml` 旁边建一个 `.env` 文件；克隆的仓库里是 `docker/.env`，在仓库根目录用 `-f docker/...` 运行时 Compose 也会读它（以前克隆的仓库如果在根目录有 `.env`，请把它移到 `docker/`）。它不提交，也不进镜像；模板是 [`.env.example`](../../docker/.env.example)：

| 变量 | 默认 | 作用 |
|---|---|---|
| `XEXAM_BIND` | `127.0.0.1:8765` | 在宿主机上发布的「地址:端口」，给 Nginx 用。必须写成 `IP:端口`。`0.0.0.0:8765` = 局域网能连。⚠️ 只写端口号 = 所有网卡 |
| `XEXAM_ALLOWED_HOSTS` | 空 | 允许用哪些域名 / IP 访问，用逗号、空格或分号分隔 |
| `XEXAM_ALLOWED_ORIGINS` | 空 | 代理把 Host 改写成源站地址时：填公网域名（带 `https://`） |
| `XEXAM_ALLOW_ANY_HOST` | `0` | `1` = 不检查 Host（只在前面已经有身份验证时用） |
| `XEXAM_PARSE_TIMEOUT_SECS` | `30` | 解析一份 Excel 的最长时间 |
| `XEXAM_SQLITE_JOURNAL` | （不设：WAL） | `delete` 让 SQLite 继续用以前的回滚日志，而不是 WAL。只在数据卷放在网络文件系统上，或者是绑定挂载进 Docker Desktop 的 Windows 文件夹时才设置；其他任何值都表示 WAL。见 [database.md](database.md) 第 2.1 节 |
| `TZ` | （不设：宿主机的时区） | 数据库里时间的时区；compose 文件挂载了宿主机的 `/etc/localtime`，所以只有想用别的时区时才设置 `TZ`（例如 `UTC`） |

改了 `.env` 之后运行 `docker compose up -d`（它会按需重新创建容器）。只有 `.env` 里设置了 `TZ` 和 `XEXAM_SQLITE_JOURNAL` 时，它们才会传进容器。

后端还读取两个 compose 文件不传递的变量：`XEXAM_COOKIE_SECURE`（`1` = 会话 Cookie 总是带 `Secure`，见第 10 节）和 `XEXAM_LOG`（日志过滤，默认 `info`，例如 `debug`）。把它们写在 `.env` 里没有效果；要用的话，请把它加到你的 `docker-compose.yml` 里 `backend` 服务的 `environment:` 下面，例如 `XEXAM_LOG: "debug"`。`xexam-backend --help` 列出所有变量。

### 3.1 时区

后端写入不带时区标记的本地时间（[database.md](database.md) 第 4.6 节）。compose 文件用长格式的卷语法，把宿主机的 `/etc/localtime` 以只读方式挂载进后端容器：

```yaml
      - type: bind
        source: /etc/localtime
        target: /etc/localtime
        read_only: true
        bind:
          create_host_path: false
```

所以时间跟随宿主机的时区；`.env` 里的 `TZ` 优先于它。`create_host_path: false` 让 Compose 在没有 `/etc/localtime` 的宿主机上报错停下（「bind source path does not exist」），而不是在那里建一个空文件夹（它挂载不到镜像里的文件上）。**在这样的宿主机上，请从 `docker-compose.yml` 里删掉这六行，并在 `.env` 里设置 `TZ`**（例如 `TZ=UTC`），否则后端容器起不来。既没有这个挂载、也没有 `TZ` 时，镜像用 UTC。

在 Docker Desktop（Windows、macOS）上，宿主机的 `/etc/localtime` 是 Docker 自己的虚拟机里的文件，不是你电脑上的，而那台虚拟机通常用的是 UTC。挂载本身没问题，但不管你的电脑用什么时区，时间都会是 UTC：请在 `.env` 的 `TZ` 里设置你想用的时区。

以后再改时区不会改写已经保存的时间；它们保留写入时的时区。

## 4. 让手机 / 外面连进来

### 同一个 Wi-Fi 里的 iPhone

只在**信任的网络**里这样做。`.env`：

```ini
XEXAM_BIND=0.0.0.0:8765
XEXAM_ALLOWED_HOSTS=192.168.1.20      # 你电脑的局域网 IP
```

在 iPhone 的 Safari 里打开 `http://192.168.1.20:8765`（需要 iOS 15.4 或更新）。

### 经 Cloudflare Tunnel 从互联网访问

后端本来就不对外，所以隧道只需要指向 **Nginx**（`XEXAM_BIND` 发布的端口）。要做三件事：

1. 确保 cloudflared 连得到 Nginx：在同一台机器上就保持默认，指向 `http://localhost:8765`；在局域网里的另一台机器上，把 `XEXAM_BIND` 设成 `0.0.0.0:8765`（或者一个具体的局域网 IP）。
2. Public Hostname：`xexam.你的域名.com` → Service `HTTP`（不是 HTTPS）→ `主机IP:8765`。
3. 在 Cloudflare Access 里加一条策略，**只允许你自己**（以及你邀请的人）。没有策略的域名等于对全世界公开。

两种常见的 Host 情况（`.env`）：

| 隧道怎么做 | 设置 |
|---|---|
| Host 原样转成你的域名 | `XEXAM_ALLOWED_HOSTS=xexam.你的域名.com` |
| 设了 `httpHostHeader`，Host 被改写成 `192.168.1.20:8765` | `XEXAM_ALLOWED_HOSTS=192.168.1.20` + `XEXAM_ALLOWED_ORIGINS=https://xexam.你的域名.com` |
| 不确定 | 先照第一行做。请求被拒绝（403）时，页面会写出它看到的 Host / Origin；把它们加进对应的选项，再运行一次 `up -d` |

Nginx 把浏览器（或隧道）发来的 `Host` **原样**转给后端，并清掉 `X-Forwarded-*`；后端只读真实的 `Host`。

⚠️ 从 v0.6 起 Xexam 有了自己的登录，但前面仍然要保留 Cloudflare Access：只让 cloudflared 连得到这个端口，路由器上不要做端口映射。在 Linux 上，主机防火墙拦不住 Docker 发布的端口；请用 `DOCKER-USER` 链，或者写一个具体的局域网 IP。

验证到什么程度（如实说明）：

- 迁移期间，两种 Host 情况（Host 保留 / Host 被改写）下完整的浏览器流程，是在**旧的 Python 版**上，用真实的 Chrome 和一个 HTTPS 反向代理跑通的。那个脚本依赖旧的 Python 代码，已经随它一起删掉了。
- 对**新的技术栈**：`tests/integration/stack_check.py` 用请求头层面的检查验证「经过 Nginx 之后，Host / Origin 守卫仍然有效，`X-Forwarded-*` 不被信任」；`nginx_check.py` 验证 Host 原样透传。
- **没有**用真正的 Cloudflare 测过。第一次上线时，请按上表「不确定」那一行做，并看页面给出的 403 提示。

## 5. 数据

- 位置：命名卷 `xexam_data` → 容器里的 `/data/xexam.db`。更新、重建或删除容器都不会丢。数据库里有题库、账户、学习记录和设置。
- 后端运行时，数据库旁边有 SQLite 的 `xexam.db-wal` 和 `xexam.db-shm`（WAL 模式，从 v0.7 起）。正常停止时，后端把所有内容复制进 `xexam.db`，并清空 `xexam.db-wal`；这两个文件可能空着留在旁边，没有害处。
- 卷里还可能有一个 `/data/backups/` 文件夹，里面是 schema 迁移前写的副本（第 6 节）。程序永远不会删除这些文件。查看它们：`docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups`。
- **备份**：先停止后端（`docker compose stop`），打包卷，再启动：命令见 [database.md](database.md) 第 5 节。后端运行时只复制 `xexam.db`，可能会漏掉还在 `xexam.db-wal` 里的最新修改；不能停止时，请把 `xexam.db`、`xexam.db-wal` 和 `xexam.db-shm` 一起复制。备份卷时也会备份 `backups/`。
- 不用 Docker、直接运行后端（开发时）：见 [development.md](development.md)。

## 6. 升级

每次升级都是 `docker compose pull && docker compose up -d`（注明需要时，先换上新的 `docker-compose.yml`）。新的后端第一次启动时，会先往 `/data/backups/` 写一份副本，再把数据库迁移到它的 schema；副本写不出来（磁盘满、没有权限）时，它报错退出，数据库不变。Docker 会不断重启它，每次启动都再试一次；Nginx 不会启动，因为它要等后端健康。`docker compose logs backend` 里写着原因。也请先自己备份一次卷（第 5 节）：卷以外的副本更保险。

逐步的检查清单、升级后的检查和回退见 [migration.md](migration.md)：

- **从 v0.6**（migration.md 第 1 节）：schema v3 → v4，备份 `xexam-schema3-<日期时间>.db`，WAL 模式。也请换上新的 `docker-compose.yml`：它不再设置固定的默认时区（而是跟随宿主机，第 3.1 节），限制容器日志的大小（第 8 节），并传递 `XEXAM_SQLITE_JOURNAL`。如果你以前的时间是用和宿主机不同的时区写入的，请在 `.env` 里把那个时区设成 `TZ`，这样新的记录会接着用它。不需要新增任何变量。
- **从 v0.5 或 v0.5.1**（migration.md 第 2 节）：schema v2 → v4 一步到位，备份 `xexam-schema2-<日期时间>.db`。登录是新加的：用 `admin` / `admin` 登录，以前的所有数据现在都归它。
- **从 v0.4**（migration.md 第 3 节）：schema v1 → v4 一步到位，备份 `xexam-schema1-<日期时间>.db`。从 v0.5.1 起，镜像来自 Docker Hub（`docker.io/yuminspace/…`，公开，不用登录），不再来自 `ghcr.io`：把你的 `docker-compose.yml` 换成新的那份（或者把里面的两行 `image:` 改成 `docker.io/yuminspace/xexam-backend:latest` 和 `docker.io/yuminspace/xexam-frontend:latest`）。源码构建时，compose 文件现在在 `docker/` 里：把仓库根目录的 `.env` 移到 `docker/.env`。

升级后，`/api/info` 显示 `"version": "0.7"`。以后再启动不会再迁移，也不会多写任何东西。**回退**就是恢复 `backups/` 里的副本，然后运行较早的镜像；除了应急，不要在迁移过的数据库上长时间运行较早的版本。具体的命令见 [migration.md](migration.md)；背景见 [database.md](database.md) 第 6 节。

## 7. 健康检查和停止

- `xexam-frontend`：`GET /healthz`（不经过 Host 白名单）。`xexam-backend`：`xexam-backend healthcheck`（在容器里用 `Host: 127.0.0.1` 请求自己）。后端健康之后 Nginx 才启动（`depends_on`）。
- 后端停了或正在重启时，Nginx 不会退出；`/api/` 只是暂时返回一个 JSON 的 502（`后端暂时连不上`），后端回来后自动恢复。
- `docker compose stop`：后端在 8 秒内平稳退出（Docker 最多等 15 秒）；导入的事务要么提交完成，要么回滚。然后它执行 WAL 检查点，所以所有内容都在 `xexam.db` 里，`xexam.db-wal` 是空的（日志里写着 `WAL checkpoint at shutdown: everything is in xexam.db, the -wal file is empty`）。崩溃或 `docker kill` 之后 `-wal` 文件会留下；SQLite 在下次启动时套用它，已经提交的东西不会丢。

## 8. 资源和镜像

| | |
|---|---|
| `xexam-frontend` | nginx-unprivileged + 静态文件（关于页面的更新日志也在其中）；监听 8080；根文件系统只读 |
| `xexam-backend` | 一个静态链接的可执行文件 + tzdata（UTC 的 `/etc/localtime` 作为后备）+ `/VERSION`；uid 10001；根文件系统只读，只有 `/data` 和 `/tmp` 可写 |
| 网络 | `internal`（后端 + Nginx，不能访问外部网络）和 `public`（只有 Nginx） |
| 容器日志 | 每个容器的日志驱动都是 `json-file`，`max-size: 10m`、`max-file: 3`，所以 Docker 为每个容器最多保留约 30 MB 日志（不加限制时，Docker 会一直保留容器日志） |

## 9. 日志：系统日志页面和 `docker compose logs`

有两个地方可以看后端记录的日志。两者显示同样的行，用同样的 `XEXAM_LOG`（默认 `info`）过滤。

| | **系统日志**（菜单里，只有管理员） | `docker compose logs backend` |
|---|---|---|
| 在哪里 | 程序里的 `#/logs` 页面，每 1.5 秒更新一次 | 在服务器上 |
| 有多少 | 后端上次启动以来的最后 **2000** 行 | Docker 还保留着的全部：最多 3 个 10 MB 的文件，跨重启保留 |
| 重启之后 | 又是空的（只在内存里，从不写进数据库） | 保留 |
| 另外 | 级别筛选、搜索、**暂停** / **继续**、**跳到最新** | Nginx 的访问日志和错误日志：`docker compose logs frontend` |

不用 Compose 时：`docker logs xexam-backend`。页面上的说明写的是 `docker compose logs xexam-backend`；用本仓库的 compose 文件时，服务名是 `backend`，所以请用 `docker compose logs backend`（或 `docker logs xexam-backend`）。

日志里有什么：启动时的几行（版本、数据目录、日志模式、允许的主机、默认密码的提醒）、登录和账户变更（按用户 id：`user 3 signed in`）、被拒绝的上传和写入、警告和错误。它从不包含密码、会话令牌、Cookie 的值、确认码或题目文字。被 Host / Origin 检查拒绝的请求、失败的登录，以及没有账户的用户名被锁定，都先记一次，之后最多每分钟汇总一次，所以没人能不登录就把其他日志挤出这 2000 行。详见 [security.md](security.md) 第 8 节。

## 10. 第一次启动、账户和找回

- 安装或升级之后，打开 Xexam，用 `admin` / `admin` 登录，然后马上设置自己的密码；在那之前，能打开这个页面的人都能用默认密码。只要还有管理员在用它，登录页就会写出那个账户，后端每次启动也会打印一条提醒（`docker compose logs backend`）。
- 在 **用户管理**（菜单里，只有管理员）里添加其他人，并选择每个人能看到哪些题库。有 **可管理题库** 权限的用户可以导入题库，并更新或删除自己的题库。
- **忘了密码**：`docker compose exec backend xexam-backend reset-password admin`（不用 Compose 时：`docker exec xexam-backend xexam-backend reset-password admin`）会打印一个新的临时密码，启用这个账户，让它在所有地方退出登录，并在下次登录时要求设置新密码。服务器运行时也能用。
- **密码输错被锁定**：登录锁定在后端的内存里，所以 `reset-password`（另一个进程）解不开它。可以让另一个管理员在用户管理里点 **解锁**，或者重启后端：`docker compose restart backend`。
- 在 Cloudflare Access 后面要登录两次（先 Access，再 Xexam）。在自己的网络里用纯 HTTP 也能用：只有用 `https://` 打开页面时，会话 Cookie 才带 `Secure`。`XEXAM_COOKIE_SECURE=1`（第 3 节）强制总是带 `Secure`；只有浏览器总是通过 HTTPS 访问 Xexam 时才设置它。

## 11. 发布

推送到 `main` 时，[`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) 先做版本、更新日志和文档的检查，再跑四组检查（前端、后端、Nginx 和整栈、arm64）。全部通过才把 `xexam-frontend` 和 `xexam-backend` 两个镜像（amd64 + arm64）发布到 Docker Hub（`docker.io/yuminspace/`，公开），并作为私有镜像副本发布到 `ghcr.io/yumin-space/`，标签都是 `latest`、`v<CODE_VERSION>` 和 `sha-<提交>`，并建立对应的 GitHub Release（说明里最先列出从 Docker Hub 拉取的命令）。最后记录这次发布：`VERSION` 里的 `RELEASE_VERSION` 和 README 的发布徽章，由机器人提交到 `main`（能快进时也提交到 `dev`）。推送到 `dev` 和向 `main` 提的拉取请求跑同样的检查，从不发布。详见 [development.md](development.md) 第 7 节。

Docker Hub 需要两个仓库机密 `DOCKERHUB_USERNAME` 和 `DOCKERHUB_TOKEN`（一个带 **Read & Write** 权限的访问令牌），另外可以设置仓库变量 `DOCKERHUB_NAMESPACE`（可选）；没有这两个机密，或者 `docker/docker-compose.yml` 里写的不是这个命名空间下的 Docker Hub 镜像，发布这一步会在推送任何东西之前停下。在 ghcr.io 上第一次发布一个新镜像名时，要在 GitHub 的包设置里把本仓库加进 **Manage Actions access**，角色选 **Admin**，否则推送会被拒绝（`permission_denied: write_package`）。
