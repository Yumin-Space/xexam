[English](deployment.md) | [日本語](deployment.ja.md) | **简体中文**

# 部署

> 新架构：`nginx`（唯一对外）+ `backend`（Rust，私有网络）+ SQLite（命名卷 `xexam_data`）。结构图见 [architecture.zh-CN.md](architecture.zh-CN.md)，安全边界见 [security.zh-CN.md](security.zh-CN.md)。

## 1. 前提

- 装了 Docker（含 Compose v2）的机器，x86（amd64）或 ARM（arm64）都行。
  - arm64 镜像在 CI 里用 QEMU **构建并启动**验证过（`/api/info` 报 `aarch64`，Nginx `/healthz` 正常）；**没有在真正的 ARM 硬件上跑过**。
  - 后端是在 x86 构建机上用 zig 交叉编译出 aarch64 版本的（冷缓存下 CI 约 5 分钟，不需要在 QEMU 里编 Rust）。
    因为 zig 的链接器不接受 rustc 默认给 aarch64 加的 `--fix-cortex-a53-843419`，aarch64 产物**不带**这个针对 Cortex-A53 核心的勘误规避。
    Oracle Cloud 的 Ampere A1 用 Ampere Altra（据 Wikipedia 是 Arm Neoverse N1 核心），不是 Cortex-A53，所以不受影响；
    如果你要在 Cortex-A53 核心的板子上跑（部分老款开发板），请别用这个镜像，先告诉我。

## 2. 启动

### 拉取现成的镜像（推荐）

只需要仓库里的 [`docker-compose.yml`](../docker-compose.yml) 这一个文件：

```bash
docker login ghcr.io -u Yumin-Space      # 第一次；镜像是私有的，密码填有 read:packages 权限的令牌
docker compose up -d
```

浏览器打开 <http://localhost:8765>，「题库管理」里上传 Excel。会拉取并启动**两个容器**：`xexam-frontend`（Nginx，唯一对外）和 `xexam-backend`（Rust，私有网络）。

### 从源码构建（改代码时用）

```bash
git clone git@github.com:Yumin-Space/xexam.git
cd xexam
docker compose -f docker-compose.build.yml up -d --build
```

第一次构建要几分钟（编译 Rust 和前端），之后有缓存。

| 操作 | 命令 |
|---|---|
| 看状态 | `docker compose ps`（两个容器都应是 `healthy`） |
| 看日志 | `docker compose logs -f` |
| 停止 / 再启动 | `docker compose stop` / `docker compose start` |
| 更新到最新版本 | `docker compose pull && docker compose up -d`（源码构建用：`git pull && docker compose -f docker-compose.build.yml up -d --build`） |
| 版本和架构 | <http://localhost:8765/api/info> |

> ⚠️ 不要运行 `docker compose down -v`：`-v` 会删除数据卷 `xexam_data`，题库和学习记录都在里面。

**`docker-compose.yml` 和 `docker-compose.build.yml` 用同一个项目名（`xexam`）、同一个卷（`xexam_data`）和同样的容器名**，所以两者不能同时运行；切换前先 `docker compose stop`。数据互通，见 [migration.zh-CN.md](migration.zh-CN.md)。

## 3. 配置（`.env`）

在 `docker-compose.yml` 旁边建 `.env`（不会被提交，也不会进镜像；模板是 [`.env.example`](../.env.example)）：

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
| Host 原样转成你的域名 | `XEXAM_ALLOWED_HOSTS=xexam.你的域名.com` |
| 设了 `httpHostHeader`，Host 被改成 `192.168.1.20:8765` | `XEXAM_ALLOWED_HOSTS=192.168.1.20` + `XEXAM_ALLOWED_ORIGINS=https://xexam.你的域名.com` |
| 不确定 | 先配第一种，被拒（403）时页面上会写出它看到的 Host / Origin，加进对应选项再 `up -d` |

Nginx 把浏览器（或隧道）发来的 `Host` **原样**转给后端，并清空 `X-Forwarded-*`；后端只读真实的 `Host`。

⚠️ 程序自己没有登录：只让 cloudflared 连得到这个端口，路由器上不要做端口映射；Linux 上主机防火墙拦不住 Docker 发布的端口，用 `DOCKER-USER` 链或具体的局域网 IP。

验证到什么程度（如实写）：

- 迁移时用真实 Chrome + HTTPS 反向代理，对**旧的 Python 版**做过两种 Host 情形（保留 Host / 改写 Host）的整套浏览器流程；那套脚本依赖旧 Python 代码，已随它移除。
- 对**新栈**：`tests/integration/stack_check.py` 用请求头级别的检查验证「经过 Nginx 之后，Host / Origin 守卫仍然生效、`X-Forwarded-*` 不被信任」；`nginx_check.py` 验证 Host 原样透传。
- **没有**用真正的 Cloudflare 测过。第一次上线时请按上表的「不确定」一行走，看页面给出的 403 提示。

## 5. 数据

- 位置：命名卷 `xexam_data` → 容器内 `/data/xexam.db`。更新、重建、删除容器都不丢。
- 备份与恢复：见 [database.zh-CN.md](database.zh-CN.md) 的第 5 节。
- 不用 Docker 直接运行后端（开发）：见 [development.zh-CN.md](development.zh-CN.md)。

## 6. 健康检查与停止

- `xexam-frontend`：`GET /healthz`（不经 Host 白名单）。`xexam-backend`：`xexam-backend healthcheck`（容器内自己带 `Host: 127.0.0.1` 访问自己）。后端 healthy 之后 Nginx 才启动（`depends_on`）。
- 后端挂了或重启时，Nginx 不会退出，只是 `/api/` 暂时回 JSON 的 502（「后端暂时连不上」），后端恢复后自动好。
- `docker compose stop`：后端在 8 秒内优雅退出；导入的事务要么提交完要么回滚。

## 7. 资源和镜像

| | |
|---|---|
| `xexam-frontend` | nginx-unprivileged + 静态文件；监听 8080；只读根文件系统 |
| `xexam-backend` | 静态链接的单个可执行文件 + tzdata；uid 10001；只读根文件系统，只有 `/data` 和 `/tmp` 可写 |
| 网络 | `internal`（后端 + Nginx，不能访问外网）和 `public`（只有 Nginx） |

## 8. 发布

推送到 `main` 时，[`.github/workflows/ci.yml`](../.github/workflows/ci.yml) 先跑完前端、后端、Nginx 与整栈、arm64 四组检查，都通过才把 `xexam-frontend` 和 `xexam-backend` 两个镜像（amd64 + arm64）发布到 `ghcr.io/yumin-space/`，标签为 `latest`、`v<VERSION>`、`sha-<提交>`，并建对应的 GitHub Release。拉取请求和其它分支只检查、不发布。

第一次发布一个新镜像名时，要在 GitHub 的包设置里把本仓库加进 **Manage Actions access**，角色选 **Admin**，否则推送会被拒绝（`permission_denied: write_package`）。
