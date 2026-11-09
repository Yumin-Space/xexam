# xexam —— CCARP 模拟考

一个在浏览器里做题的本地模拟考程序。题目来自你自己的 Excel，上传一次就存进 SQLite；做完题交卷才算分，并列出每道错题的答案、解析和记忆口诀。

- 只用 Python 标准库，没有第三方依赖。
- 题库和学习记录都保存在 Docker 卷里的 `ccarp.db`，更新或删除容器都不会丢。
- 支持日语 / 中文对照切换，错题本，成绩历史。
- 仓库里**没有题库**。题库是你自己的 Excel，不会被提交（`.gitignore` 已排除）。

## 快速开始

只需要仓库里的 [`docker-compose.yml`](docker-compose.yml) 这一个文件，复制到任意机器的任意文件夹即可。

```bash
# 第一次：登录 GitHub 容器仓库（镜像是私有的）
docker login ghcr.io -u Yumin-Space
# 密码处填一个有 read:packages 权限的 GitHub 令牌

docker compose up -d
```

然后浏览器打开 <http://localhost:8765>，点「题库管理 / 上传 Excel」导入题库。

| 操作 | 命令 |
|---|---|
| 更新到最新版本 | `docker compose pull && docker compose up -d` |
| 停止 / 再启动 | `docker compose stop` / `docker compose start` |
| 看日志 | `docker compose logs -f` |
| 从源码构建（改代码时） | `docker compose -f docker-compose.build.yml up -d --build` |

> 镜像由 GitHub Actions 自动构建并发布到 `ghcr.io/yumin-space/xexam`，见下方「自动构建」。

## 题库：上传 Excel

第一次使用和以后更新，都是同一个流程：

1. 在电脑上用 Excel 修改你的题库文件（新增工作表、新增或修改题目都可以）。
2. 打开页面，点「题库管理 / 上传 Excel」，选择整份文件（也可以拖进去）。
3. 先看**检查结果**：新增几题、更新几题、恢复几题、下架几题、哪几题读不出来（会写明工作表、行号和原因）。
4. 确认没问题，点「确认导入」。点之前不会写入任何东西。

上传的文件会被当成**完整的题库**：

| 情况 | 结果 |
|---|---|
| 文件里新出现的题 | 新增 |
| 文件里改过的题（选项、答案、解析、翻译） | 更新，答题记录不受影响 |
| 现有的题在文件里找不到 | **下架**，不再出现在考试里。导入前会弹窗提醒 |
| 下架的题以后又出现在文件里 | 自动恢复，答题记录和错题本也回来 |
| 改了某道题的**题干** | 被当成一道新题（旧题下架，旧题的答题记录不会带到新题上） |
| 整份文件一道可用的题都没有 | 拒绝导入，防止把题库清空 |

### Excel 格式

每个工作表是一组题，一行一道题：

| 列 | 内容 |
|---|---|
| **A** | 题目。选择题：题干后空一行，选项写成 `A. 内容`，一行一个。热点题：第一行写 `ホットスポット`，小项写成 `1. 内容　はい / いいえ` |
| **B** | 中文翻译（可以不写）。要和 A 列保持同样的结构：选项也写成 `A. 内容`，热点题的小项写成 `1. 内容`，这样答题时才能切换成中文 |
| **C** | 以 `【答案】` 开头，后面可以接 `【解析·怎么选】` 和 `【记忆口诀】` |

C 列的答案写法：

- 单选 / 多选：`【答案】C` 或 `【答案】C、E`
- 热点题：`【答案】1 是；2 否；3 是`。小项的答案可以写 `是/否`、`はい/いいえ`，或者直接写选项文字（比如 `Model Stage`）

读不出来的题**不会被猜**：它们会在检查结果里列出原因（多半是 C 列没写 `【答案】…`，或答案字母不在选项里），不会悄悄判错。

## 考试

- 答题时不显示对错，可以跳题、标记不确定的题。点「交卷」后才计分（答对的百分比）。
- 单选、多选要选对全部才算对；热点题每个小项都对才算这题对。
- 题目右上角有「日语 / 中文」按钮，随时对照；切换不影响已选的答案和计时。
- 答错的题进入**错题本**，之后连续 2 次答对自动移出。交卷后可以「只重做错题」。
- 没做完可以「暂存退出」，下次回到首页「继续作答」（暂存放在浏览器里，换浏览器就没有了）。
- 键盘：`←` `→` 切换题目，`A`–`E` 选答案，`M` 标记。

## 数据放在哪里

数据保存在一个 **Docker 命名卷**里，不是你电脑上看得见的文件夹。

- 卷名是 `xexam_data`（项目名 `xexam` + 卷名 `data`），里面是 `ccarp.db`：题库、成绩、错题记录都在这一个文件里。
- 更新镜像、`docker compose down`、删除并重建容器，**数据都还在**。
- ⚠️ **不要运行 `docker compose down -v`**，`-v` 会把卷一起删掉，数据就没了。

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

重启时会自动把里面的成绩和错题记录迁移进数据库，只迁移一次，日志里会写「已把旧的学习记录迁移进数据库」（`docker compose logs` 能看到）。数据库里已经有学习记录时不会覆盖。

> 这里没有用 `docker cp`：它默认以 root 身份写入，文件属于 root，而容器里的程序是非 root 用户，可能读不了。上面的做法把文件的所有者设成了程序用户（uid 10001）。

## 不用 Docker

需要 Python 3.8 或更新版本。

```bash
python server.py            # 数据放在 ./data，自动打开浏览器
python server.py --data D:\xexam-data
python server.py --import 题库.xlsx   # 不启动服务，直接把 Excel 导入数据库
```

Windows 上也可以双击 `start.bat`。

## 让手机 / 平板也能用（可选）

**这个程序没有登录**，默认只绑定本机（`127.0.0.1`）。只在你信任的家庭 Wi-Fi 里这样做，不要在咖啡厅或公司网络里开：

1. 把 `docker-compose.yml` 里的 `"127.0.0.1:8765:8765"` 改成 `"8765:8765"`。
2. 在 `environment` 里加一行 `CCARP_ALLOWED_HOSTS: "你的电脑的局域网IP"`（例如 `192.168.1.20`）。
3. `docker compose up -d`，手机浏览器打开 `http://你的电脑IP:8765`。

## 自动构建（GitHub Actions）

[`.github/workflows/docker.yml`](.github/workflows/docker.yml)：

- 推送到 `main`、打 `v*` 标签、或在 Actions 页面手动运行，都会**构建镜像并运行冒烟测试**：容器不是 root、全新卷时题库为空、陌生 Host 被拒绝、数据库写进卷里、重启后服务正常、健康检查变 `healthy`、删除容器后用同一个卷重建数据仍在。
- **测试全部通过才会发布**到 `ghcr.io/yumin-space/xexam`（标签 `latest`、`sha-<提交>`，打标签时还有版本号）。测试失败时 `latest` 仍是上一个能用的版本。
- 拉取请求只构建和测试，不发布。
- 只改文档也会触发一次。

镜像从私有仓库发布，默认也是私有的。想免登录拉取，到 GitHub 的包设置里把它改成公开。

## 目录结构

```
README.md            本文件
server.py            HTTP 服务：上传检查、导入预览/确认、成绩记录
app/db.py            SQLite 存储：题库、导入计划、成绩、错题统计
app/xlsx_parser.py   读取 .xlsx 并解析题目（zipfile + ElementTree）
web/                 单页前端（index.html / app.js / style.css）
Dockerfile           非 root 用户运行，/data 是卷
docker-compose.yml        拉取镜像运行（只需要这一个文件）
docker-compose.build.yml  从源码构建运行
.github/workflows/   自动构建、测试、发布
使用说明.txt          面向使用者的说明
```

## 安全说明

- 服务只接受发给本机的请求（检查 `Host`），写请求还会检查 `Origin`，防止恶意网页通过你的浏览器操作它。
- 上传的文件会检查格式，拒绝空文件、非 `.xlsx`、损坏的压缩包、含 XML 实体声明的文件，最大 20 MB。
- 上传后的预览在内存里保留 30 分钟，确认时需要一次性的校验码，用过就作废。
- 数据库写入在一个事务里：中途出错整体回滚，不会留下半截数据。
- 容器以非 root 用户（uid 10001）运行。
