[English](../en/migration.md) | [日本語](../ja/migration.md) | **简体中文**

# 迁移与回滚

> 本文覆盖每一条升级路径，最新的在前：
>
> - **v0.9 → v0.9.1**（没有单独的一节）：除了 `docker compose pull` 和 `docker compose up -d` 之外不用做什么；数据库没有变化，没有新设置，也不需要新的 `docker-compose.yml`。回退到 v0.9 只要把三个镜像的 `:latest` 改成 `:v0.9`，再 `docker compose up -d`；数据库保持原样。
> - **v0.8 → v0.9**（第 0 节）：不需要新的 `docker-compose.yml`；主数据库升级到 schema v6，AI 数据库升级到 schema 3，各自都在自己的备份副本写好之后自动进行。主数据库回退不需要恢复；AI 数据库需要把它的副本放回去。（这一节编号为 0，这样其他各节的编号，也就是别的文档链接到的那些，保持不变。）
> - **v0.7.5 或 v0.7 → v0.8**（第 1 节）：带 AI 服务 `ai-service` 的新 `docker-compose.yml`（第三个镜像 `xexam-ai-service` 和两个卷）；先写一份备份副本，然后数据库自动升级到 schema v5；管理员设置之前 AI 保持关闭。
> - **v0.6 → v0.8**（第 2 节）：先写一份备份副本，然后数据库自动升级到 schema v5，并改用 WAL 模式；然后旧的三列和四列 Excel 格式的题被下架，你再用六列格式的工作簿更新题库。
> - **v0.5 或 v0.5.1 → v0.8**（第 3 节）：登录和之后的各个 schema 一步到位。
> - **v0.4 → v0.5 及以后**（第 4 节）：题库、考试详情和设置（schema v2），现在和之后的所有变化一起完成。
> - **Python v0.2.x → Rust + TypeScript + Nginx（v0.3）**（第 5–11 节）：当时数据库文件和 schema 都没变；那次切换是「换程序」，不是「搬数据」。决定的依据：[migration-plan.md](history/migration-plan.md)。风险：[risk-assessment.md](history/risk-assessment.md)。
> - **v0.1 → 以后的版本**（第 12 节）：数据库的文件名。
>
> 每次改变 schema 的升级都会先写**一份**数据库副本，以它原来的 schema 命名：`xexam.db` 旁边的 `backups/xexam-schema<N>-<YYYYmmdd-HHMMSS>.db`（Docker 里：卷 `xexam_data` 里的 `/data/backups/`）。回退就是**恢复那份副本**，然后运行较早的镜像，只有一个例外：v0.8 能原样打开 v0.9 的 schema 6 主数据库（第 0.4 节）。AI 数据库有它自己的副本，在卷 `xexam_ai_data` 的 `/data/backups/` 里，名字是 `xexam-ai-schema<N>-<YYYYMMDD-HHMMSS>.db`；v0.8 的 AI 服务打不开 v0.9 的 schema 3 数据库，所以回退到 v0.8 就要在那里恢复 schema 2 的副本。背景见 [database.md](database.md) 第 6 节。回退主数据库从不会碰到 AI 数据库，反过来也一样。
>
> **从更早的版本直接升级到 v0.9.1**，用当前的镜像会一次做完下面各节的每一步：写一份以数据库原来的 schema 命名的备份，然后迁移到当前的 schema（主数据库是 6，AI 数据库是 3）。下面各节是为它们升级到的那个版本写的，所以某一节写 schema 5 的地方，v0.9.1 最后停在 schema 6，日志里会写 `Upgraded the database to schema 6`，`/api/info` 显示 `0.9.1`，ai-service 创建的数据库是 schema 3。
>
> **从 v0.6 或更早的版本升级，都会下架你所有的题。** 从 v0.7 起只能导入六列 Excel 格式，每次启动都会下架从以前版本的三列和四列格式导入的题；v0.6 及更早版本的每道题都是这两种格式之一。题库、学习记录和统计保留。升级之前请为每份工作簿准备好六列格式的版本（见 [usage.md](usage.md) 第 12 节），升级之后用它更新每个题库（第 2.4 节）。

## 0. 从 v0.8 升级到 v0.9

这一节编号为 0，这样第 1 到 12 节的编号（别的文档链接到它们）保持不变。

### 0.1 会有什么变化

v0.9 新增了学习时的 AI 帮助（练习里的 AI 提示和 AI 解释、成绩页上的 AI 解析、一条记录的 AI 总结；见 [usage.md](usage.md) 第 5.1、6.1 和 7.1 节）。它**不需要新的 `docker-compose.yml`**：还是同样的三个镜像，只是标签换成新的，卷和网络都一样，也没有新的设置。

| | v0.8 | v0.9 |
|---|---|---|
| 镜像、容器、卷、网络 | `xexam-frontend`、`xexam-backend`、`xexam-ai-service`；`xexam_data`、`xexam_ai_data`、`xexam_link` | 相同 |
| 主数据库（`xexam.db`） | schema 5 | **schema 6** |
| AI 数据库（`xexam-ai.db`） | schema 2 | **schema 3** |
| Nginx（在前端镜像里） | 流式转发 `send` 和 `retry` | 也流式转发新的 `study/ask` 和 `summary/ask`，并且记录请求的路径时不带查询字符串 |

有两个数据库要迁移，各由自己的服务在自己的第一次启动时迁移，各在自己的备份副本之后：

1. **后端** 把数据库复制到 `/data/backups/xexam-schema5-<YYYYmmdd-HHMMSS>.db`，这是一个普通的 schema 5 数据库，v0.8 能打开它，然后在一个事务里把数据库迁移到 schema v6：`attempts` 表新增两列，`session_key`（练习或考试那一次的 32 位 key）和 `started_at`（它开始的时间，Unix 秒），你已有的每条记录里这两列都是空的（`NULL`）。没有索引，也没有约束，其他都不变：题库、记录、账户、设置和会话都保留，没有人会被退出登录。日志里会写 `Upgraded the database to schema 6. A copy of the previous version was saved first: /data/backups/xexam-schema5-….db (…)`。副本写不出来时，什么都不改变，后端也不启动。从现在起，保存下来的练习或考试会带着它的 key 和开始时间（不发这两样、或发了无效值的浏览器，保存下来的就是空的）。
2. **ai-service** 把它的数据库复制到卷 `xexam_ai_data` 里的 `/data/backups/xexam-ai-schema2-<YYYYMMDD-HHMMSS>.db`，然后在一个事务里通过添加列和索引把它迁移到 schema 3，从不删除任何东西：`chats` 上新增 `kind`（`chat`、`practice`、`exam` 或 `summary`；已有的每个对话都成为 `chat`）、`ref`（这个对话所属的会话 key 或记录）和 `record_id`（记录，只用于显示）；`messages` 上新增 `kind`（`chat`、`hint`、`explain` 或 `summary`；已有的每条消息都成为 `chat`）、`question_id`、`question_no`、`source`、`lang`，以及总结的三个计数 `sample_used`、`sample_usable` 和 `sample_wrong`；`usage` 上新增 `purpose`（`hint`、`explain` 或 `summary`；你已有的每一行里它是空的，统计时把它们当作 AI 对话）；还有三个索引，用来找一个会话的对话、一道题已保存的回答，以及被重启截断的回答。设置、加密的密钥、上限、用户、对话、消息和记忆都保持原样。日志里会写 `Upgraded the AI database to schema 3. A copy of the previous version was saved first: …`。副本写不出来时，ai-service 不启动，也什么都不改变；主程序照常工作，**AI 设置** 会写明 ai-service 没有运行。

关于这些副本要知道的：

- **空闲磁盘空间**：每份副本都是它的数据库的完整副本（SQLite `VACUUM INTO`），所以每个服务在它的卷里需要的空闲空间，约等于它的数据库现在的大小。磁盘满了会让那个服务无法启动，数据库保持不变。
- **它们从不会自动删除**，而且它们保存着写下它们时存在的一切，包括你或管理员之后删除的对话和学习记录。不再需要时请自己删除旧副本（`docker run --rm -v xexam_data:/data alpine rm /data/backups/<文件名>`），并且要像数据库一样妥善保管它们（[database.md](database.md)）。
- 已经迁移过的数据库再次打开时，不会写新的副本，也不会写入任何东西。

你还会注意到：

- 有 **AI+ 功能** 的账号：练习里的 AI 按钮和图标、成绩页和记录详情页上的 **AI 解析**、学习记录和主页上的 **AI 总结**，以及 AI 对话里新增的练习、考试和总结的对话。没有这个功能的用户什么新东西都看不到。AI 被暂停或暂时不能用的用户会看到这些控件变灰。什么都不用设置：AI+ 功能、服务商的设置和用户的开关都和以前一样。
- **v0.9 之前生成的记录** 没有会话 key。它们在详情页上第一次做 AI 解析时，会为这条记录开一个自己的对话（usage.md，第 16.1 节）；AI 总结对它们和对任何记录一样可用，除非记录太旧、没有保存逐题详情。
- AI 用量统计新增了 **按功能** 表（AI 对话、AI 提示、AI 解释·解析、AI 总结）。
- **对话上限会把新的对话计算在内**：每次练习、每场考试和每条总结过的记录都是一个对话，一次请求是两条消息，所以一次很长的练习在大约 200 次请求之后就会用完默认的每个对话 400 条消息（usage.md，第 17 节）。

### 0.2 升级

```bash
# 1. 备份两个卷（先停止；备份文件里是你的整个数据库，不要提交它们）
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v08.tgz -C /data .
docker run --rm -v xexam_ai_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-ai-data-v08.tgz -C /data .

# 2. 拉取三个镜像并启动（不要用 down -v）
docker compose pull
docker compose up -d

# 3. 查看两条升级信息
docker compose ps
docker compose logs backend
docker compose logs ai-service
```

你的 `docker-compose.yml` 和 `.env` 保持不变。**源码构建**：在仓库根目录运行 `git pull && docker compose -f docker/docker-compose.build.yml up -d --build`。想有意留在 v0.8，就让三个镜像都保持 `:v0.8` 标签。

### 0.3 升级后的检查

- [ ] `docker compose ps` 列出 `xexam-frontend`、`xexam-backend` 和 `ai-service`，都是 `healthy`。
- [ ] `/api/info` 里的 `version` 是 `0.9`。
- [ ] 后端日志里有 `Upgraded the database to schema 6. A copy of the previous version was saved first: /data/backups/xexam-schema5-…`，并且 `docker run --rm -v xexam_data:/data alpine ls -l /data/backups` 列出了那份副本。
- [ ] ai-service 日志里有 `Upgraded the AI database to schema 3. A copy of the previous version was saved first: …`，并且 `docker run --rm -v xexam_ai_data:/data alpine ls -l /data/backups` 列出了 `xexam-ai-schema2-…`。
- [ ] 你可以像以前一样登录，学习记录能打开，**AI 对话** 仍然列着你的对话。**AI 设置** 写着 **ai-service 已连接（版本 0.9；…）**，AI+ 功能和你离开时一样。
- [ ] 想试试新功能：用有 **AI+ 功能** 的账号登录，开始一次练习，按 **AI 提示**（usage.md，第 5.1 节）。
- [ ] 把容器再重启一次，不会写新的副本，也没有升级的日志行。

### 0.4 回退到 v0.8

两个数据库的回退方式不同。

- **主数据库不需要恢复。** v0.8 原样打开 schema 6 的数据库：它忽略那两个新列，v0.8 保存的记录只是 key 和开始时间为空。这一点测试过：v0.8 的后端打开了迁移过的数据库，保存了一条记录，并让 `meta.schema` 仍是 6；v0.9 之后在它上面再次启动，没有写新的副本。
- **AI 数据库确实需要它的副本。** v0.8 的 ai-service 不接受 schema 3 的数据库，并以 `The AI database … has schema 3, written by a newer ai-service; this version (schema 2) cannot open it. Run the newer version, or restore a backup.` 退出，文件保持不变。在恢复它之前，v0.8 没有 AI 运行。恢复副本会**丢失升级之后对 AI 数据库做的每一项改变**：那段时间所有的对话（包括所有练习、考试和总结的对话）、记忆、设置的修改、用量数字和用户的暂停。密钥文件 `secret.key` 不在数据库里，会保留下来，所以恢复后的数据库里的密钥仍然能解开。

```bash
# 1. 停止新版本（不要用 down -v）
docker compose stop

# 2. 找到 AI 的副本（名字里有它原来的 schema：xexam-ai-schema2-…）
docker run --rm -v xexam_ai_data:/data alpine ls -l /data /data/backups

# 3. 把它放回去，作为 xexam-ai.db（用第 2 步看到的文件名）；先删掉残留的 -wal / -shm 文件
docker run --rm -v xexam_ai_data:/data alpine sh -c 'rm -f /data/xexam-ai.db-wal /data/xexam-ai.db-shm && cp /data/backups/xexam-ai-schema2-20261112-140501.db /data/xexam-ai.db'

# 4. 在 docker-compose.yml 里把三个镜像固定在 v0.8（在 Docker Hub 上，不用登录），然后启动：
#      image: docker.io/yuminspace/xexam-frontend:v0.8
#      image: docker.io/yuminspace/xexam-backend:v0.8
#      image: docker.io/yuminspace/xexam-ai-service:v0.8
docker compose up -d
```

源码构建时，请检出那个版本：`git fetch --tags && git checkout v0.8 && docker compose -f docker/docker-compose.build.yml up -d --build`。

如果你宁愿暂时不用 AI，就跳过第 2 和第 3 步：v0.8 的主程序可以单独运行，你把 ai-service 停掉（`docker compose stop ai-service`），免得它不断重启；不要动它的卷。想让主数据库回到升级之前的样子（不是必需的），就像第 1.4 节那样恢复 `/data/backups/xexam-schema5-….db`，这会丢失在主程序里做过的一切。要回退到比 v0.8 更早，就用更早那一节的回退（1.4、2.6……），用当时写的副本；那些副本仍然在 `backups/` 里。

以后要再升级，就把 v0.9 的镜像换回来。仍然是 schema 6 的主数据库不会再次迁移，也不会写新的副本；你恢复成 schema 2 的 AI 数据库会再次迁移，先写一份新的副本。

### 0.5 测试过什么

只用合成数据（`backend/tests/v09_ai_study.rs`、`db_golden.rs`、`ai-service/tests/` 和 `tests/integration/`）：

- v0.8 写的 schema 5 数据库迁移到 schema 6，正好写一份 `xexam-schema5-…` 备份（`migrate_check.py`，情形 J）；旧记录的 key 和开始时间是空的；结果的 `sqlite_master` 和新建的数据库一样；再次打开时什么都不写。更早的数据库（schema 3 和 4）一步迁移到 schema 6，写一份以它们原来的 schema 命名的备份。
- schema 1 或 2 的 AI 数据库迁移到 schema 3，写一份 `xexam-ai-schema<N>-…` 副本，最后的 `sqlite_master` 和新建的一样；查找已保存的回答用到了它们的索引（没有临时排序）；旧的用量行按 AI 对话读取；保留期的清理分小批删除，让其他写入者能在批与批之间插进来。
- v0.8 的后端能打开并写入 schema 6 的数据库，v0.8 的 ai-service 拒绝 schema 3 并保持文件不变（上面的回退）。
- 用三个镜像和一个虚构的服务商跑整个栈（`stack_check.py`）：提示、解释和解析及它们保存的回答和 `Regenerate`，泄露答案的提示被隐藏，最多 70 道答错的题的总结，一次一个回答的拒绝和它正在进行的回答，没有 AI+ 功能的用户和被暂停的用户的访问规则，会话和记录的对话，以及任何容器的日志里（包括 Nginx 的访问日志）都没有题目、答案、会话 key 或记录时间。无头浏览器的走查检查了控件、窗口和手机布局。

没有测试过：迁移**你自己的**数据库，以及真正的 AI 服务商（CI 用的是虚构的）。想要确定的话，先在一份副本上试一下升级。

## 1. 从 v0.7.5（或 v0.7）升级到 v0.8

### 1.1 会有什么变化

v0.8 新增了 AI 对话，它在**第三个容器** `ai-service`（镜像 `xexam-ai-service`）里运行，有自己的数据库。这需要**新的 `docker-compose.yml`**：

| | v0.7.5 | v0.8 |
|---|---|---|
| 镜像 | `xexam-frontend`、`xexam-backend` | 同样的两个，再加上 **`xexam-ai-service`** |
| 容器（Compose 服务） | `xexam-frontend`、`xexam-backend`（`frontend`、`backend`） | 同样的两个，再加上 **`ai-service`**（`ai-service`） |
| 卷 | `xexam_data` | `xexam_data`（不变）、**`xexam_ai_data`**（AI 数据库和它的密钥文件）、**`xexam_link`**（只有 `internal.token`，后端和 AI 服务共用的令牌） |
| 网络 | `internal`（不能上网）、`public` | 同样的两个，再加上 **`egress`**，只给 `ai-service` 向外访问互联网；后端仍然不能上网 |

用旧的 compose 文件，v0.8 也能启动，除了 AI 以外一切照常：菜单里没有 AI 对话，**AI 设置** 提示 ai-service 没有运行（还带一条下载新 compose 文件的提示），后端记录 `AI link off: cannot create /link/internal.token (…)`。

后端**第一次启动**时，在接受任何请求之前做这些事：

1. **备份**：把数据库复制到 `/data/backups/xexam-schema4-<YYYYmmdd-HHMMSS>.db`，这是一个普通的 schema 4 数据库，v0.7.5 能打开它。副本写不出来时，什么都不改变，后端也不启动。
2. **迁移到 schema v5**，在一个事务里完成：`users` 表新增两列，`ai_enabled`（**AI+**，所有人都是关闭的）和 `uid`（每个用户一个随机的身份标记）。其他都不变：题库、记录、设置、账户和会话都保留，没有人会被退出登录。日志里会写 `Upgraded the database to schema 5. A copy of the previous version was saved first: /data/backups/xexam-schema4-….db (…)`。
3. **内部令牌**：后端把一个随机令牌写到 `/link/internal.token`（在 `xexam_link` 里），并记录 `AI link: /link/internal.token`。设置了 `XEXAM_INTERNAL_TOKEN` 时，它不写文件，并记录 `AI link: using XEXAM_INTERNAL_TOKEN`。

ai-service 第一次启动时，用默认设置创建它的数据库 `xexam-ai.db`（v0.8 里是 schema 2，v0.9 里是 schema 3），没有设置 `XEXAM_AI_SECRET_KEY` 时还会创建密钥文件 `secret.key`（`Created a new AI key file /data/secret.key`）。然后它马上向后端报告，之后每 25 秒报告一次，所以主服务知道它已连接（超过 60 秒的报告算作没有连接）；后端从不调用它。**AI 是关闭的**，直到管理员在 **AI 设置** 里把它设置好（见 [usage.md](usage.md) 第 17 节）；AI 没有需要迁移的东西。（由 v0.8 的预发布版写入的 AI 数据库，即 schema 1，会在 ai-service 第一次启动时升级，先在 AI 卷里它旁边写一份副本 `backups/xexam-ai-schema1-<YYYYMMDD-HHMMSS>.db`；副本写不出来时，ai-service 不启动，也什么都不改变。来自更晚的预发布版、但缺少 schema 2 的某些列（`users.function_enabled`、`users.free_mode`、`messages.free`、`usage.tokens_cached`）的 schema 2 数据库，每次启动时都会把它们补上，不写副本：这一步只添加缺少的列，从不删除任何一列，所以那种版本留下的旧的 `chats.free` 列只是闲置在那里。）

你还会注意到：

- 菜单里有 **AI 对话** 和 **AI 设置**（AI 对话只在 AI 开启后出现，对管理员和有 **AI+ 功能** 的用户）。**AI 设置** 总是能打开，并显示 ai-service 有没有连接。AI 开启后，管理员能在 **设置** 里找到 **AI 对话模式**（**普通模式** / **自由模式**）。
- **用户管理**：每个用户一个 **AI+ 功能** 开关（只在 ai-service 已连接、而且 AI 设置里的 **AI+ 功能** 已开启时显示；某个用户的 **暂停** 和 **恢复** 在 AI 设置的 **AI+ 用户** 下面）；**更改** 和 **添加用户** 里选择题库的部分更紧凑了（**管理题库**、**全选** 和 **全部不选** 排成一行放在列表上方，**包括以后导入的题库** 是列表的第一项，说明都改成了提示框），**管理题库** 从 **编辑** 移到了这里。
- 页头、浏览器标签页和手机主屏幕上的新图标（以前添加到主屏幕的图标可能还是旧的样子，重新添加一次就好）。

从 v0.7（而不是 v0.7.5）升级时，v0.7.5 的变化也会一起带来（见 [更新日志](changelog.md)）；数据库的步骤相同。

由 **v0.7 的预发布版** 写入的数据库（仍然有 `users.all_banks` 这一列），会在同一次启动时、在同一个事务里、在写完同一份副本（`xexam-schema4-…`）之后被转换：能看到所有题库的用户得到每个已有题库和 **包括以后导入的题库**，其他用户保留他们的题库，在这期间由更早的版本（v0.6 或之前）加进来的用户也会得到所有题库和 **包括以后导入的题库**。这时日志里会写 `Converted the question bank access of a database written by a v0.7 pre-release to the released format and upgraded the database to schema 5. A copy of the previous version was saved first: <path>.`（见 [database.md](database.md) 第 6.5 节）。

### 1.2 升级

```bash
# 1. 备份卷（先停止；备份文件里是你的整个数据库，不要提交它）
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v075.tgz -C /data .

# 2. 换上新的 docker-compose.yml（你的 .env 保持不变；如果你自己改过 docker-compose.yml，请把那些改动搬过去）
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

# 3. 拉取三个镜像并启动（不要用 down -v）
docker compose pull
docker compose up -d

# 4. 查看升级信息和 AI 服务的启动
docker compose ps
docker compose logs backend
docker compose logs ai-service
```

- **可选，在第 3 步之前**：要让 AI 加密密钥和 AI 卷分开存放，现在就把 `XEXAM_AI_SECRET_KEY=<openssl rand -base64 32 的输出>` 写进 `docker/.env`。以后再设置也可以，但那样的话，在那之前保存的 AI 密钥就无法再解密，必须重新输入。`XEXAM_INTERNAL_TOKEN` 也是可选的；不设置时，后端会创建令牌（见 [deployment.md](deployment.md) 第 3 节）。
- **没有 `/etc/localtime` 的宿主机**：新的 compose 文件也把它挂载进 AI 服务；请在两个服务里都删掉那六行，并在 `docker/.env` 里设置 `TZ`。
- **AI 服务不能上网**（宿主机在严格的防火墙后面）：Xexam 照常可用；只是 AI 设置里的测试会以「连不上 AI 服务商」失败，直到 AI 服务能连到服务商为止（或者把它指向你网络里的服务商）。
- **源码构建**：在仓库根目录运行 `git pull && docker compose -f docker/docker-compose.build.yml up -d --build`；它把 `ai-service/Dockerfile` 构建成第三个镜像 `xexam-ai-service:local`。
- **从 v0.8 的预发布版升级**（compose 文件里 AI 服务还叫 `ai`）：用新文件运行 `docker compose up -d --remove-orphans`，会删掉旧的容器；卷 `xexam_ai_data` 和 `xexam_link` 会继续使用。没有哪个正式发布的版本用过那个名字，所以从 v0.7.5 正常升级时没有什么要删的。

### 1.3 升级后的检查

- [ ] `docker compose ps` 列出 `xexam-frontend`、`xexam-backend` 和 `ai-service`，都是 `healthy`。
- [ ] `/api/info` 里的 `version` 是 `0.8`。
- [ ] 你可以像以前一样登录；没有人需要重新登录。**用户管理** 显示同样的用户，每个普通用户的 **AI+ 功能** 都是关闭的（等 ai-service 已连接、AI 设置里的 **AI+ 功能** 已开启之后，才会出现这些开关）。
- [ ] **系统日志** 里有一条 `AI: AI link: connected to the main service`（AI 服务的日志行以 `AI: ` 开头；每次重新连接后会再出现一条，连接断开时会有 `AI: AI link: lost the main service`）。
- [ ] **AI 设置** 能打开，显示 **ai-service 已连接（版本 0.8；…）** 和 **AI+ 功能已关闭**；在你保存连接之前，第一个标记是 **连接未设置**。想试试 AI：设置一个服务商，**测试**，开启 **AI+ 功能**，然后在 **AI 对话** 里问点什么。如果它显示的是 **ai-service 连接状态**，请看它写明的原因（见 [usage.md](usage.md) 第 17 节）。
- [ ] 从现在起备份两个数据卷（见 [database.md](database.md) 第 5 节和第 8.4 节）。

### 1.4 回退到 v0.7.5

回退就是**恢复迁移前写的那份副本**，换回旧的 `docker-compose.yml`，然后运行 v0.7.5 的镜像。迁移之后在主程序里做的一切都会丢失（记录、账户、设置、导入）；AI 数据留在它自己的卷里，v0.7.5 不使用这个卷。

```bash
# 1. 停止新版本（不要用 down -v）
docker compose stop

# 2. 找到那份副本。正常停止之后，/data 里即使留着 xexam.db-wal，也是空的。
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. 把副本放回去，作为 xexam.db（用第 2 步看到的文件名）；先删掉残留的 -wal / -shm 文件
docker run --rm -v xexam_data:/data alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && cp /data/backups/xexam-schema4-20261111-140501.db /data/xexam.db'

# 4. 换回 v0.7.5 的 docker-compose.yml，并固定它的两个镜像（在 Docker Hub 上，不用登录），然后启动：
#      image: docker.io/yuminspace/xexam-backend:v0.7.5
#      image: docker.io/yuminspace/xexam-frontend:v0.7.5
#    并删掉旧文件已经不认识的、已停止的 `ai-service` 容器（它的卷保留）：
docker compose up -d --remove-orphans
```

源码构建时，请检出那个版本：`git fetch --tags && git checkout v0.7.5 && docker compose -f docker/docker-compose.build.yml up -d --build --remove-orphans`。

v0.7.5 也能直接打开迁移过的数据库（它忽略那两个新列），但这只留作应急用：它创建的用户没有身份标记，AI+ 是关闭的，v0.8 下次启动时会给他们一个标记（[database.md](database.md) 第 6.6 节）。卷 `xexam_ai_data` 和 `xexam_link` 可以留着以后升级时用；`docker volume rm xexam_ai_data xexam_link` 会删掉它们（这会永久删除 AI 对话、记忆和设置）。

以后要再升级，就换回新的 `docker-compose.yml`，运行 `docker compose pull && docker compose up -d`。迁移会再执行一次，并写一份新的副本；`xexam_ai_data` 里的 AI 设置和对话都还在，已经不存在的用户的 AI 数据会被 AI 服务删除。

### 1.5 测试过什么

只用合成数据（`backend/tests/v08_ai_link.rs`、`db_golden.rs`、`ai-service/tests/` 和 `tests/integration/`）：

- v0.7.5 写的 schema 4 数据库迁移到 schema v5，正好写一份 `xexam-schema4-…` 备份；每个用户的 AI+ 都是关闭的，并得到一个唯一的 32 位十六进制标记；结果的 `sqlite_master` 和新建的数据库一样，再次打开时什么都不写。
- schema 3 的数据库一步迁移到 schema v5，写一份 `xexam-schema3-…` 备份；预发布版 v4 数据库在一个事务里被转换并迁移。
- 回退后由 v0.7.5 创建的用户在下次启动时得到一个标记，不写备份。
- 后端在第一次 `serve` 时创建 `internal.token`（权限 0600），重启时继续使用它，设置了 `XEXAM_INTERNAL_TOKEN` 时不写文件，对不符合规则的值用一行日志关闭连接。
- 用三个镜像和一个虚构的服务商跑整个栈（`stack_check.py`）：登录、AI 设置、测试、开启、带 keep-alive 注释的流式回答、回答过程中关闭 **AI+ 功能**（它在几秒内结束）和暂停某个用户（它立即结束）、ai-service 没有连接或功能关闭时用户管理里的 AI+ 开关被拒绝、对 ai-service 执行 `docker stop` 和 `docker start` 之后的连接状态、回复语言、管理员的自由模式（在设置里选择、按用户保存、发送时带 `free` 会被忽略、旧的按对话设置的路由返回 404）、重新创建容器后 AI 设置和数据仍在、`ai-service` 停止时主程序照常回答、每个卷里有哪些文件，以及任何日志或回应里都没有密钥、题目或回答。停止、重试、上限、向后端的报告、AI 数据库升级到 schema 2 和管理员的访问在 `ai-service/tests/` 里测试。

没有测试过：迁移**你自己的**数据库，以及真正的 AI 服务商（CI 用的是虚构的）。想要确定的话，先在一份副本上试一下升级。

## 2. 从 v0.6 升级到 v0.8

### 2.1 会有什么变化

新版本在你的数据卷上**第一次启动**时，在接受任何请求之前做这些事：

1. **备份。** 把当前的数据库复制到 `/data/backups/xexam-schema3-<YYYYmmdd-HHMMSS>.db`。这份副本是一个普通的 schema 3 数据库，v0.6 能打开它。**副本写不出来时**（磁盘满、没有权限），后端拒绝启动，数据库保持原样；日志里写明原因。
2. **迁移到 schema v5**（经过 v4），在一个事务里完成：
   - **已有的每个用户仍然能看到所有题库**：每个人都得到每个已有题库和 **包括以后导入的题库**，在用户管理里显示为 **题库：全部，包括以后导入的**。之后添加的用户，在你选好之前一个题库都没有。
   - **已有的每个题库都归第一个管理员**（id 最小的已启用管理员）。还没有人 **可管理题库**。
   - **已有的每条学习记录都是考试。** 练习记录从现在起才会出现。
   - **AI+** 对所有人都是关闭的，每个用户都得到一个身份标记（v5 这一步，第 1.1 节）。
   - 题目 id、统计、记录、设置、账户和会话都不变：没有人会被退出登录。
3. **WAL 模式。** 只有迁移成功之后，后端才把数据库切换到 WAL。后端运行时，`xexam.db` 旁边有 `xexam.db-wal` 和 `xexam.db-shm`；正常停止时它们会被并回 `xexam.db`，`xexam.db-wal` 被清空（这两个文件可能空着留下，没有害处）。
4. **下架旧格式的题**，在另一个事务里完成：从三列或四列格式导入的每一道上架的题（v0.6 的话就是每一道题）都被下架，和导入时下架一道从工作簿里消失的题完全一样。题库、它们的导入记录、学习记录、统计和错题本都保留；这些题不再出题，题库列表里会写明这个题库的题来自旧的 Excel 格式。日志里每个题库有一条警告，例如 `retired 12 questions of bank "Kitchen basics": they use the old 3- / 4-column layout; import the six-column version of the workbook`。这项检查在每次启动时运行（命令行命令也是），没有要下架的题时什么都不写。

你还会注意到：

- **只能导入六列格式的工作簿。** 三列或四列格式的文件会被拒绝，并有一句提示说明原因（见 [usage.md](usage.md) 第 12 节）。
- **用户管理** 里每一行都显示这个用户的题库，带 **更改** 用来选择（里面有 **管理题库**，即可管理题库的权限），还有 **AI+** 开关。收回某个题库后，这个用户在其中的记录仍作为历史保留。
- 主页上 **开始考试** 旁边有 **开始练习**；**学习记录** 里练习记录带 **练习** 标签。
- 管理员的菜单里多了 **系统日志** 和 **AI 设置**，AI 设置好之后还有 **AI 对话**（第 1.1 节）。
- 答案与解析的每张卡片都有自己的 **English** / **中文** / **日本語** 切换按钮。

新的 `docker-compose.yml` 和 v0.6 的不同：它有 AI 服务和它的两个卷以及网络 `egress`（第 1.1 节）；它跟随宿主机的时区，而不是固定的默认时区（它挂载 `/etc/localtime`；`docker/.env` 里的 `TZ` 优先于它）；它把每个容器的日志限制在 3 个 10 MB 的文件；`docker/.env` 里设置了 `XEXAM_SQLITE_JOURNAL`、`XEXAM_COOKIE_SECURE`、`XEXAM_LOG` 和 AI 的变量时，它会把它们传进去。v0.6 的文件仍然能启动新版本，只是没有这些改动，也没有 AI。`.env` 里不需要新增任何变量。

### 2.2 升级之前

- [ ] 为你用的每份工作簿准备好**六列格式的版本**：A–C 列是英语、中文和日语的题目，D–F 列是同样三种语言的答案和解析，以英语为准（见 [usage.md](usage.md) 第 12 节）。没有中文或日语的地方可以空着。
- [ ] 记下每个题库的题数（**题库管理** 页面），以便升级后对照。
- [ ] 把没交卷的考试交卷或放弃：它的题会在升级时被下架，所以之后不能再接着考。

### 2.3 升级

```bash
# 1. 备份卷（先停止；备份文件里是你的整个数据库，不要提交它）
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v06.tgz -C /data .

# 2. 换上新的 docker-compose.yml（你的 .env 保持不变；如果你自己改过 docker-compose.yml，请把那些改动搬过去）
curl -fLO https://raw.githubusercontent.com/Yumin-Space/xexam/main/docker/docker-compose.yml

# 3. 拉取新版本并启动（不要用 down -v）
docker compose pull
docker compose up -d

# 4. 查看升级信息、副本的文件名、日志模式和被下架的题
docker compose logs backend
```

日志里应该有 `Upgraded the database to schema 5. A copy of the previous version was saved first: /data/backups/xexam-schema3-20261111-140501.db (…)`、`SQLite journal mode: wal`，以及每个题库一条 `retired … of bank "…"` 警告。

- **时区。** v0.6 的 compose 文件在 `.env` 没有写别的时区时，会设置一个固定的时区。如果宿主机的时区和你以前的时间写入时用的时区不同，请在 `docker/.env` 里把那个时区设成 `TZ`（例如 `TZ=UTC`），这样新的记录会接着用它。已经保存的时间永远不会被改写。
- **没有 `/etc/localtime` 的宿主机**：Compose 会报「bind source path does not exist」并停下。从 `docker-compose.yml` 里删掉这个挂载的六行（后端和 AI 服务里各有一处），并在 `docker/.env` 里设置 `TZ`（[deployment.md](deployment.md) 第 3.1 节）。
- **特殊的存储**：如果卷放在网络文件系统上，或者是绑定挂载进 Docker Desktop 的 Windows 文件夹，请在第 3 步之前把 `XEXAM_SQLITE_JOURNAL=delete` 写进 `docker/.env`。
- **源码构建**：在仓库根目录运行 `git pull && docker compose -f docker/docker-compose.build.yml up -d --build`，然后 `docker compose -f docker/docker-compose.build.yml logs backend`。不用 Docker 时，副本写到数据目录（`--data` 或 `XEXAM_DATA`）里的 `backups/`。

### 2.4 让题库重新可用

每个题库都这样做：**题库管理** → **导入 Excel** → **更新已有题库**，在 **要更新的题库** 里选好题库，选择它的六列格式文件，然后确认。

- 题库的名字、所有者和能看到它的用户都不变。
- 六列格式按英语文字识别题目，以前的格式按日语文字识别，所以文件里的每道题都作为新题**新增**，没有答题记录，旧题仍保持下架。以前的学习记录仍然显示旧题，带 **已下架** 标签；错题本从新题重新开始。
- 也可以把六列格式的文件作为 **新建题库** 导入，以后再删掉旧题库（删不删它的学习记录都可以）。

在这之前打开主页的用户，会看到一句说明：题库里没有可用的题。

### 2.5 升级后的检查

- [ ] `/api/info` 里的 `version` 是 `0.8`，并且 `docker compose ps` 列出三个 healthy 的容器。
- [ ] 你可以像以前一样登录；没有人需要重新登录。
- [ ] **用户管理**：以前的每个用户都显示 **题库：全部，包括以后导入的**。
- [ ] **题库管理** 列出同样的题库，每个都是第一个管理员**上传**的。在第 2.4 节的更新之前，每个题库都显示它的题已下架（例如 **0 题（另有 25 题已下架）**，就是你记下的那个数）；更新之后，显示六列格式文件里的题数。
- [ ] **学习记录** 列出以前的记录，每条都带 **考试** 标签。
- [ ] **系统日志** 显示启动时的几行，其中有 `SQLite journal mode: wal` 和被下架的题。
- [ ] 做一次简短的练习，把它在 **学习记录** 里的时间和时钟对一下。如果差了整数个小时，请设置 `TZ`（第 2.3 节）。
- [ ] 然后为每个用户决定他能看到哪些题库，以及谁可以管理题库；在你设置之前 AI 是关闭的（第 1.1 节）。

### 2.6 回退到 v0.6

回退就是**恢复迁移前写的那份副本**，然后运行 v0.6 的镜像。迁移之后做的一切都会丢失：六列格式的导入、新题库、记录（包括练习记录）、账户、题库权限和设置。AI 数据留在它自己的卷里，v0.6 不使用这个卷。

```bash
# 1. 停止新版本（不要用 down -v）
docker compose stop

# 2. 找到那份副本。正常停止之后，/data 里即使留着 xexam.db-wal，也是空的。
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. 把副本放回去，作为 xexam.db（用第 2 步看到的文件名）。残留的 -wal / -shm 文件属于被替换掉的那个
#    数据库，不能套用到副本上，所以先删掉它们。
docker run --rm -v xexam_data:/data alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && cp /data/backups/xexam-schema3-20261111-140501.db /data/xexam.db'

# 4. 在 docker-compose.yml 里固定 v0.6 的镜像（在 Docker Hub 上，不用登录），从文件里删掉 ai 服务（和它的卷），
#    或者换回 v0.6 的 docker-compose.yml，然后启动：
#      image: docker.io/yuminspace/xexam-backend:v0.6
#      image: docker.io/yuminspace/xexam-frontend:v0.6
docker compose up -d --remove-orphans
```

源码构建时，请检出那个版本：`git fetch --tags && git checkout v0.6 && docker compose -f docker/docker-compose.build.yml up -d --build --remove-orphans`。

第 3 步也可以换成恢复你在第 2.3 节自己做的 `.tgz`（恢复命令见 [database.md](database.md) 第 5 节）；里面是同样的数据。不管哪种方式，第 4 步都需要。

v0.6 能直接打开迁移过的数据库，也会自己把文件从 WAL 切回去，但这只留作应急用：v0.6 不懂 v0.7 及以后版本的规则，所以每个用户又能看到所有题库，只有管理员能管理题库，练习记录看起来和考试一样，它也完全不知道六列格式的题。之后再启动新版本时，它会修好 v0.6 留下的东西（[database.md](database.md) 第 6.6 节）：v0.6 导入的题库会得到所有者，并给到设置了 **包括以后导入的题库** 的用户；v0.6 创建的用户会得到所有题库和 **包括以后导入的题库**；v0.6 导入的题会再次被下架。

以后要再升级，就把新版本的镜像放回去（标签 `:latest`），运行 `docker compose pull && docker compose up -d`。迁移会再执行一次，并写一份新的副本。

### 2.7 测试过什么

只用合成数据（`backend/tests/v065_bank_access_and_practice.rs`、`v07_bank_access_and_layouts.rs`、`v07_wal_and_logs.rs`、`v08_ai_link.rs`、`db_golden.rs` 和 `tests/integration/`）：

- schema 3 的数据库带着一份备份迁移到 schema v5；每个用户都保留所有题库，包括以后导入的，题库都归第一个管理员。
- 备份写不出来时，数据库不迁移，也不改变。
- 旧格式的题只下架一次，题库、记录和错题本都保留；第二次启动什么都不写。命令行程序也一样，每个题库记一行英语日志。题已被下架的题库用六列格式的文件更新：新题被加入，旧题仍保持下架。
- 上传、导入确认和命令行的 `import` 会在暂存或写入任何东西之前拒绝三列和四列格式的工作簿。
- v0.7 预发布版的数据库在备份之后被转换，在这期间由更早的版本添加的用户也一样。
- 新的数据库是 WAL 模式；`XEXAM_SQLITE_JOURNAL=delete` 保留 DELETE，之后不带它启动时又回到 WAL，所有记录都还在。
- 迁移一个 WAL 模式的数据库时，还在 `-wal` 文件里的修改也会备份进去；备份是一个独立的文件。
- 打开一个已经是最新的数据库什么都不写（数据库文件和它的 `-wal` 文件逐字节不变）。
- `docker stop` 之后，卷里是 `xexam.db`，没有 `-wal` 的内容。
- 服务器打开着数据库时，`reset-password` 也能用。
- 没有所有者的题库、回退后由旧版本创建的题库和用户，以及过时的题库分配，下次启动时都会修好。

没有测试过：迁移**你自己的**数据库。想放心的话，请先在副本上做（像第 6 节的「在副本上试新版本」那样）。

## 3. 从 v0.5 或 v0.5.1 升级到 v0.8

### 3.1 会有什么变化

v0.6 引入了账户和登录（schema v3）；v0.7 又加上了按用户设置题库（schema v4），v0.8 加上了 AI+ 开关和身份标记（schema v5）。从 v0.5 升级时，这些步骤都在**第一次启动**时、在写完一份副本之后，在一个事务里完成：

1. **备份**到 `/data/backups/xexam-schema2-<YYYYmmdd-HHMMSS>.db`，一个普通的 schema 2 数据库，v0.5 能打开它。写不出来时什么都不变，后端也不启动。
2. **账户。** 创建默认管理员 `admin`，密码 `admin`，它**得到你原有的一切**：每条学习记录、错题本和设置。
3. **schema v4 和 v5**，和第 2.1 节一样：`admin` 能看到并拥有所有题库；AI 是关闭的（第 1.1 节）。
4. **WAL 模式**，然后**下架旧格式的题**，和第 2.1 节一样：请先准备好六列格式的工作簿（第 2.2 节），之后用它们更新题库（第 2.4 节）。

从现在起，每个页面都需要登录的账户。每个人都有自己的记录、错题本和设置，所以请在 **用户管理** 里添加其他人，而不是大家共用 `admin`。

### 3.2 升级

用第 2.3 节的命令，备份的名字写成 `xexam-data-v05.tgz`。v0.5 的 `docker-compose.yml` 仍然从 `ghcr.io` 拉取；新的那份从 Docker Hub（`docker.io/yuminspace/…`，公开，不用登录）拉取。项目名和卷 `xexam_data` 都一样，所以数据原样保留。

日志里会写出副本的名字（`…/backups/xexam-schema2-….db`），写着 `Created the default administrator 'admin' (password 'admin'); sign in and choose a new password.`（意思是：已创建默认管理员 admin，密码 admin；请登录并设置新密码），并按题库列出被下架的题。

### 3.3 升级后的检查

- [ ] 打开 Xexam：出现登录页，上面写着账户 `admin`。用 `admin` / `admin` 登录，并马上设置自己的密码；在那之前，能打开这个页面的人都能用默认密码。
- [ ] `/api/info` 里的 `version` 是 `0.8`。
- [ ] **题库管理** 列出同样的题库，在你更新它们之前（第 2.4 节）它们的题都是已下架的；**学习记录** 列出你以前的考试；**设置** 里是你的主题和语言。
- [ ] 添加其他用户，并选择他们的题库。

### 3.4 回退到 v0.5

和第 2.6 节一样，用副本 `xexam-schema2-….db` 和镜像 `docker.io/yuminspace/xexam-backend:v0.5.1`、`docker.io/yuminspace/xexam-frontend:v0.5.1`（v0.5 本身只在私有的镜像副本 `ghcr.io/yumin-space/` 上）。升级之后做的一切都会丢失，包括账户。

不要在迁移过的数据库上运行 v0.5：它不认识账户，所以它的记录不属于任何人（新版本下次启动时会把它们给第一个管理员），而且所有人又会共用一个错题本。

## 4. 从 v0.4 升级到 v0.5 及以后

现在这种升级会直接到 v0.8：下面的步骤，再加上第 3.1 节和第 2.1 节的步骤，在写完**一份**副本 `xexam-schema1-<YYYYmmdd-HHMMSS>.db` 之后，在**一个**事务里完成，然后旧格式的题被下架（第 2.1 节第 4 步）。然后你用 `admin` / `admin` 登录（第 3.3 节），你的所有数据都归 `admin`。请先准备好六列格式的工作簿（第 2.2 节）。

### 4.1 你的数据会怎样

v0.5 给数据库加上了题库、每道题的考试详情和设置（schema v2）。这个变化纯粹是新增的：不重建任何东西，也不改变任何题目 id。完整的细节：[database.md](database.md) 第 6 节。

新版本在你的数据卷上**第一次启动**时，后端在接受任何请求之前做这些事：

1. **备份。** 把当前的数据库复制到 `xexam.db` 旁边的 `backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db`（Docker 里：卷 `xexam_data` 里的 `/data/backups/`）。这份副本是一个普通的 schema 1 数据库，v0.4 能打开它。已有的文件永远不会被覆盖。**副本写不出来时**（磁盘满、没有权限），后端拒绝启动，数据库保持原样；日志里写明原因。
2. **迁移**，在一个事务里完成：
   - 你现有的所有内容变成**一个题库**（「旧题库」）。它以你最近一次导入的文件名命名，去掉 `.xlsx`（如果从来没导入过，就叫 `Question bank`）。
   - **题目 id 不变。** 所以每道题的统计、错题本和考试历史都还对应着原来的题（这些题随后会被下架，见上文）。
   - **考试记录都保留**，归到这个题库下。它们没有每道题的详情，因为 v0.4 从来没有保存你给出的答案：它们的详情页显示分数，但没有答案，也没有「重考」之类的按钮。
   - 导入记录保留，归到这个题库下。

以后再启动时，只会（只读地）检查有没有需要修正的东西，什么都不写。

你还会注意到：

- **界面一开始是英语。** v0.4 是中文。在 **Settings → Language**（中文界面里是 **设置 → 语言**）里，或者在登录页上修改。这个设置保存在数据库里，所以设一次，所有浏览器和设备都生效。
- 在 **设置** 里选定主题之前，主题跟随系统（浅色或深色）。
- 题目语言（答题页上的日语 / 中文 / 英语）以前由浏览器记住。某个浏览器第一次打开新版本时，会把这个选择转存到服务器上，前提是服务器上还是默认值（日语）。
- 浏览器里暂存的没交卷的考试在升级后不能再接着考，因为它的题和其他题一起被下架了：请在升级之前交卷。
- 数据库现在为每个用户保留最新的 500 条记录，而不是 60 条。

### 4.2 升级

`.env`、端口和卷都不用改（见 [deployment.md](deployment.md) 第 6 节）。从 v0.5.1 起，只有 `docker-compose.yml` 里镜像的来源是新的：它从 Docker Hub（`docker.io/yuminspace/…`，公开，不用登录）拉取，不再从 `ghcr.io` 拉取。把你的 `docker-compose.yml` 换成新的那份，或者把里面的两行 `image:` 改成 `docker.io/yuminspace/xexam-backend:latest` 和 `docker.io/yuminspace/xexam-frontend:latest`。项目名和卷 `xexam_data` 都不变，所以数据原样保留。

```bash
# 1. 备份卷（先停止；备份文件里是你的整个数据库，不要提交它）
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v04.tgz -C /data .

# 2. 换上新的 docker-compose.yml（Docker Hub 上的镜像）后，拉取当前版本并启动（不要用 down -v）
docker compose pull
docker compose up -d

# 3. 查看升级信息和副本的文件名
docker compose logs backend
```

日志里应该有这样一行：`Upgraded the database to schema 5. A copy of the previous version was saved first: /data/backups/xexam-schema1-20261110-140501.db (…)`，然后是这个题库被下架的题。那份副本是程序自己写的；第 1 步的 `.tgz` 是你自己在卷以外的第二份副本。

源码构建：在仓库根目录运行 `git pull && docker compose -f docker/docker-compose.build.yml up -d --build`。compose 文件现在在 `docker/` 里：运行那个命令之前，先把仓库根目录的 `.env` 移到 `docker/.env`（Compose 不会再读根目录的 `.env`），之后的命令（例如 `logs backend`）也都要加上 `-f docker/docker-compose.build.yml`。不用 Docker 时，副本写到数据目录（`--data` 或 `XEXAM_DATA`）里的 `backups/`。

### 4.3 升级后的检查

- [ ] 用 `admin` / `admin` 登录，并设置自己的密码（第 3.3 节）。
- [ ] `/api/info` 里的 `version` 是你安装的版本（现在是 `0.8`）。
- [ ] **题库管理** 列出一个题库，包含你所有的工作表；它的题都算作已下架，数量和旧版的一样。
- [ ] **学习记录** 列出你以前的考试。
- [ ] 用你的工作簿的六列格式版本更新这个题库（第 2.4 节），答几道题并交卷。新的记录里每道题都显示你的答案。

### 4.4 回退到 v0.4

回退就是**恢复迁移前写的那份副本**，然后运行 v0.4 的镜像。迁移之后在新版本里做的一切都会丢失：账户、新题库、导入、记录和设置。

```bash
# 1. 停止新版本（不要用 down -v）
docker compose stop

# 2. 找到那份副本。正常停止之后，/data 里没有 xexam.db-journal，即使留着 xexam.db-wal，也是空的。
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. 把副本放回去，作为 xexam.db（用第 2 步看到的文件名；先删掉残留的 -wal / -shm 文件）
docker run --rm -v xexam_data:/data alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && cp /data/backups/xexam-schema1-20261110-140501.db /data/xexam.db'

# 4. v0.4 只在私有的 GHCR 镜像副本上（Docker Hub 从 v0.5.1 开始才有）：先 docker login ghcr.io，再把 docker-compose.yml 里的两个镜像改成
#      image: ghcr.io/yumin-space/xexam-backend:v0.4
#      image: ghcr.io/yumin-space/xexam-frontend:v0.4
docker compose up -d
```

源码构建时，请检出那个标签：`git checkout v0.4 && docker compose -f docker-compose.build.yml up -d --build`（在那个标签里，compose 文件还在仓库根目录，Compose 也从那里读取 `.env`：如果你有 `docker/.env`，先把它复制到仓库根目录）。

第 3 步也可以换成恢复你在第 4.2 节自己做的 `.tgz`（恢复命令见 [database.md](database.md) 第 5 节）；里面是同样的数据。不管哪种方式，第 4 步都需要。

**不要跳过恢复副本、直接在迁移过的数据库上运行 v0.4。** v0.4 能打开这个文件，但它把所有题库当成一个：它的下一次导入会把其他所有题库的题都下架，它会把考试记录删减到 60 条，而且它对账户一无所知。如果还是这样做了，新版本下次启动时会把 v0.4 新增的题挂到旧题库下面（[database.md](database.md) 第 6.6 节），并把它们下架，因为它们是旧格式的。

以后要再升级，就把新的 `docker-compose.yml` 放回去（Docker Hub 上的镜像，标签 `:latest`），运行 `docker compose pull && docker compose up -d`。迁移会再执行一次，并写一份新的副本。

### 4.5 测试过什么

只用合成数据（`backend/tests/v05_banks_records_settings.rs`、`v06_users_and_sign_in.rs` 和 `db_golden.rs`）：

- `sample.db`（旧 Python 实现写的数据库）迁移到当前的 schema，保留每一个题目 id、统计、考试记录和导入记录。它的备份仍然报告 schema 1，行和原来完全一样。
- schema 1 的数据库一步升级到当前的 schema，只有一份 schema 1 的备份。
- 备份写不出来时，数据库不迁移，也不改变。
- 再次打开迁移过的数据库，文件一个字节都不变。
- 旧程序写进迁移过的文件里的行，下次启动时挂到旧题库下面。
- 新的数据库和迁移过来的数据库结构相同。

没有测试过：迁移**你自己的**数据库。想放心的话，请先在副本上做（像第 6 节的「在副本上试新版本」那样）。

## 5. Python 到 Rust（v0.3）：原则

1. **永远不碰原件。** 先备份，先在副本上试，确认一切正常之后再切换。
2. **随时可以回去。** 直到 v0.4，两个实现都用同一个 schema 读写同一个文件（迁移期间用合成数据演练过双向往返，见第 9 节）。从 v0.5 起，schema 有了新的表和列；从较新的版本回去，就是恢复迁移前写的那份副本（第 4.4 节）。
3. **旧的实现没有丢。** 旧代码在 Git 标签 `python-v0.2`；旧镜像 `ghcr.io/yumin-space/xexam:v0.2.2`（以及 `:v0.2` 和 `:v0.2.1`）仍然在 GHCR 上（私有：拉取前先 `docker login ghcr.io`；Docker Hub 上没有它），新的流水线不会覆盖它。仓库的主分支里已经没有任何 Python 代码了。

## 6. Python 到 Rust：升级前的检查清单

按顺序做。没有一步会改动你真正的数据库。

- [ ] **备份数据卷**（先停止旧容器）：

  ```bash
  docker compose stop
  docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data.tgz -C /data .
  docker compose start
  ```

  备份文件里是你的整个数据库，所以**不要提交到 Git**（`.gitignore` 已忽略 `*.tgz`）。
- [ ] **验证备份**：解压到一个临时目录；`sqlite3 work/xexam.db "PRAGMA integrity_check"` 必须输出 `ok`（命令见 [database.md](database.md) 第 5 节）。
- [ ] **在副本上试新版本**：把解压出来的 `xexam.db` 放进一个**空的临时目录**，用它启动后端，不用命名卷：

  ```bash
  cd backend && cargo build --release
  XEXAM_DATA=/tmp/xexam-trial ./target/release/xexam-backend serve --no-browser
  ```

  用浏览器打开 <http://localhost:8765>，用 `admin` / `admin` 登录，检查 **题库管理** 显示一个题库，它已下架的题和旧版的题一样多，并检查 **学习记录** 里有以前的成绩历史。（这也会把副本迁移到当前的 schema，下架它的题（都是旧格式的），并在临时目录里写一个 `backups/`。）**用完删掉这个临时目录。**
- [ ] **记下旧版的数字**：旧版 `/api/info` 里的 `bankCount`（它的题数；当前版本不再报告它）、主页上的总题数，以及成绩历史里的考试场数。升级后逐一核对。
- [ ] **准备好你的工作簿的六列格式版本**（见 [usage.md](usage.md) 第 12 节）：当前版本第一次启动时会下架所有旧格式的题（第 2.1 节）。
- [ ] **检查 `.env`**：`XEXAM_ALLOWED_HOSTS`、`XEXAM_BIND` 等可以保留旧的值；变量名没有变。

## 7. Python 到 Rust：升级

```bash
# 1. 停止旧版（不要用 down -v）
docker compose stop

# 2. 删掉旧容器（只删容器，不删卷）。旧版的服务和容器都叫 xexam，新版是 frontend / backend。
#    不删的话，旧容器会作为「孤儿」留在项目里，它的端口映射还占着 8765，新的 xexam-frontend 会因为端口冲突起不来。
docker rm xexam

# 3. 换上新的 docker-compose.yml（现在有三个服务，从 Docker Hub 拉取，不用登录），然后拉取并启动（同一个卷 xexam_data，同一个端口）
docker compose pull
docker compose up -d

# 4. 检查容器都是 healthy（从 v0.8 起有三个）
docker compose ps
```

`docker rm xexam` 只删**容器**。它不会删数据卷 `xexam_data`（那要 `docker volume rm` 或 `down -v`；两者都不要做）。

从 Python 直接升级到当前版本，也会执行第 4 节的 schema 迁移（带备份副本），创建账户 `admin` / `admin`，你的数据都归它，并下架每一道题，因为它们都是旧格式的；然后用你的六列格式工作簿更新这个题库（第 2.4 节）。

升级后的检查：

- [ ] `/api/info` 里的 `version` 是你安装的版本（现在是 `0.8`）。
- [ ] 登录之后，**题库管理** 显示那一个题库，它已下架的题和旧版的题一样多（旧的 `bankCount`），**学习记录** 里的成绩历史和升级前一样。
- [ ] 用你的六列格式工作簿更新这个题库（第 2.4 节），答几道题并交卷；分数出现在历史里。

> ⚠️ 新旧版本用的是同一个项目名（`xexam`）、同一个卷（`xexam_data`）和同一个端口，所以**不能同时运行**。旧版的容器叫 `xexam`，新版的叫 `xexam-frontend` 和 `xexam-backend`，所以升级时必须像上面第 2 步那样删掉旧容器。

## 8. Python 到 Rust：回退

可以回到 Python 版。旧版的 compose 文件已经不在主分支上了；从标签里取出来：

```bash
# 1. 停止新版本（不要用 down -v）
docker compose stop

# 2. 取出旧版的 compose 文件，启动旧版（它从私有的 GHCR 拉取 ghcr.io/yumin-space/xexam:latest，所以先 docker login ghcr.io；想固定版本就把 :latest 改成 :v0.2.2）
git show python-v0.2:docker-compose.yml > docker-compose.legacy.yml
docker compose -f docker-compose.legacy.yml up -d
```

注意旧 compose 文件里的镜像标签是 `:latest`，**而 `ghcr.io/yumin-space/xexam:latest` 在这次发布之后就不再更新了**（新的镜像用 `xexam-frontend` / `xexam-backend` 两个新名字），所以它仍然指向最后一个 Python 版本。保险起见，请明确写成 `:v0.2.2`。

**从 v0.3 或 v0.4 回退**时，旧版继续用同一个 `xexam_data` 卷，数据不会丢。它能读取这些版本写入的一切（新导入的题、新的分数），因为它们不改 schema、不改 `journal_mode`、不加表或列、不设置 `user_version`，也不留下 `-wal` / `-shm` 文件。

**从 v0.5 或更新的版本回退**时，schema 里有新的表和列（从 v0.7 起文件还是 WAL 模式）。先像第 4.4 节第 1–3 步那样恢复迁移前写的 schema 1 副本，再像上面那样启动旧版。

如果你还想让数据回到升级前的状态（例如你怀疑新版本把它弄坏了）：

```bash
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine sh -c 'rm -f /data/xexam.db-wal /data/xexam.db-shm && tar xzf /backup/xexam-data.tgz -C /data'
docker compose -f docker-compose.legacy.yml up -d
```

> 恢复会用备份覆盖 `/data`，所以备份之后在新版本里做的所有修改都会丢失。只在确定需要时才这样做。

不用 Docker 回退：`git checkout python-v0.2`，运行 `python server.py`（需要 Python 3.8+），数据目录指向同一个 `xexam.db`。

## 9. Python 到 Rust：演练过什么

下面这些是在迁移期间（Python 代码还在仓库里时）做的，**只用虚构的数据**；没有读取、打印或修改任何真实的数据库。依赖于实际运行旧 Python 代码的脚本，已经随那些代码一起删掉了；结论继续由已提交的快照测试守护。

| 演练 | 怎么做的 | 结果 | 现在由什么守护 |
|---|---|---|---|
| 新旧之间的往返 | 旧 Python 建库 → 备份 + `integrity_check` → Rust 在**副本**上读取（和旧版逐字节相同）→ Rust 导入第二份工作簿并记录分数 → 旧 Python 读回来（和 Rust 相同）→ 旧 Python 再写 → Rust 再读 | 19 项检查通过，包括「原件的 sha256 前后不变」 | `db_golden.rs`：`sample.db`（旧实现写的数据库）迁移到当前的 schema 时保留每一行，迁移后的文件之后再打开、读取、关闭都逐字节不变；新建数据库的 `sqlite_master` 是旧实现的文本加上后来新增的部分 |
| 黄金响应对照 | 旧版的 172 份响应和新后端逐项比较 | 0 处无法解释的差异，登记了 23 处 | `api_golden.rs` 在每次 CI 运行时重放它们 |
| 安全探测 | 向新旧两个版本各发 340 个恶意请求，3 种配置 | 0 处无法解释的差异，登记了 51 处（大多是新版本更严格的情况） | 探测脚本已经删掉；守卫和上传检查由 `api_*.rs`、`parser_hardening.rs` 和 `nginx_check.py` 测试 |
| 解析器对照 | 49 份合成工作簿，逐项比较旧解析器和 Rust 解析器的输出；85 条题目 id 向量 | 全部相同 | `parser_golden.rs` 和 `id_vectors.rs` 在每次 CI 运行时运行 |
| 整栈 | `tests/integration/stack_check.py`：启动整栈，上传 / 导入 / 交卷，删除容器并在同一个卷上重建整栈后数据仍在，`docker stop` 的时间，非 root | 当时 34 项检查通过（这个脚本后来又加了检查） | 每次 CI 运行时运行 |

## 10. Python 到 Rust：和旧版有意不同的地方

带理由的完整表格见 [api.md](api.md) 第 7 节。每一条都登记在 `tests/baseline/golden/allowed_differences_rust.json` 和 `allowed_probe_differences_rust.json` 里，没有登记的差异会让测试失败。用户能看到的只有这些：

| 项目 | 变化 |
|---|---|
| 架构 | 一个容器变成两个（nginx + 后端）；镜像叫 `xexam-frontend` / `xexam-backend`；`XEXAM_BIND` 现在指向 Nginx |
| 更严格的上传检查 | 任何编码 / 扩展名里的 DOCTYPE 都拒绝；重复的 ZIP 条目名拒绝；条目数、XML 深度和节点数都有上限 |
| 错误提示 | 其他所有中文提示都原样保留；以前有两处会带出库异常的文字，现在改用一句固定的中文提示 |
| 判分 | 仍然在浏览器里进行，没有搬到服务端（规格建议搬过去；这里有意不搬，理由见 [migration-plan.md](history/migration-plan.md) 的 Q3） |
| iPhone | 需要 iOS 15.4 或更新（构建目标 `safari15`） |
| 不再提供 | 后端不再提供静态文件（由 Nginx 提供）；`Server` 响应头 |

之后各版本的 API 变化列在 [api.md](api.md) 里。

## 11. Python 到 Rust：没有验证过的

- 真正的 Cloudflare Tunnel / Access、真正的 iPhone，以及在真正的 ARM 硬件上运行（arm64 只在 QEMU 下构建并启动过）。
- 用**你自己的真实数据库**的副本演练：我没有它，也不应该读取它；请在你自己的机器上按第 6 节做。

## 12. 从 v0.1 升级

从 v0.2 起，数据库文件叫 `xexam.db`。如果数据目录里有别的 `.db` 文件、却没有 `xexam.db`，后端的启动日志会提醒你；把那个文件改名为 `xexam.db` 再启动（旧文件既不会被删除，也不会被读取）。旧的 `progress.json` 每次启动时都会检查一次，只有数据库里还没有学习记录、以前也没有迁移过时才会迁移；它只迁移一次，永远不会覆盖已有的记录。
