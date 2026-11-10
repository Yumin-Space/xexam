[English](../en/migration.md) | [日本語](../ja/migration.md) | **简体中文**

# 迁移与回滚

> 本文按从新到旧的顺序，介绍每一条升级路径：
>
> - **v0.4 → v0.5**（第 1 节）：先写好一份备份副本，然后数据库自动升级到 schema v2。
> - **Python 版 v0.2.x → Rust + TypeScript + Nginx 版（v0.3）**（第 2–8 节）：当时数据库文件和 schema 都没有变，所以切换是「换程序」，不是「搬数据」。决策依据：[migration-plan.md](history/migration-plan.md)。风险：[risk-assessment.md](history/risk-assessment.md)。
> - **v0.1 → 之后的版本**（第 9 节）：数据库文件名。

## 1. 从 v0.4 升级到 v0.5

### 1.1 你的数据会怎样

v0.5 在数据库里加入了题库、考试的逐题明细和设置（schema v2）。这次改动只做加法：不重建任何东西，也不改任何题目 id。详见 [database.md](database.md) 第 6 节。

v0.5 在你的数据卷上**第一次启动**时，后端在接受任何请求之前先做下面这些事：

1. **备份。** 把当前数据库复制一份，写到 `xexam.db` 旁边的 `backups/xexam-schema1-<YYYYmmdd-HHMMSS>.db`（Docker 里是卷 `xexam_data` 中的 `/data/backups/`）。这份副本是一个普通的 schema 1 数据库，v0.4 能打开。已有的文件永远不会被覆盖。**如果副本写不出来**（磁盘满、没有权限），后端拒绝启动，数据库保持原样；日志里会写明原因。
2. **迁移**，在一个事务里完成：
   - 你现有的全部内容变成**一个题库**（legacy 题库）。它以你最近一次导入的文件名命名，去掉 `.xlsx`（如果从来没导入过，就叫 `Question bank`）。
   - **题目 id 不变。** 所以每道题的统计、错题本和考试历史都还和原来的题对应着。
   - **考试记录都保留**，并归到这个题库下。它们没有逐题明细，因为 v0.4 从来不保存你填的答案：详情页只显示分数，不显示答案，也没有「重做错题」「重考这一场」按钮。
   - 导入历史也保留，并归到这个题库下。

之后每次启动只做只读检查，确认没有需要修正的地方，不写入任何东西。

你还会注意到这些变化：

- **界面默认是英文。** v0.4 是中文界面。可以在 **Settings → Interface language**（中文界面里是「界面设置 → 界面语言」）里修改。这个设置保存在数据库里，所以设一次，所有浏览器和设备都生效。
- 在 **Settings**（界面设置）里选定主题之前，主题跟随系统（浅色或深色）。
- 题目语言（答题页上的日语 / 中文 / 英语）以前由浏览器记住。某个浏览器第一次打开 v0.5 时，如果服务器上还是默认值（日语），就会把这个浏览器里的选择搬到服务器上。
- 保存在浏览器里、还没交卷的考试会保留，可以继续作答。
- 数据库现在保留最近的 500 场考试，不再是 60 场。

### 1.2 升级

`docker-compose.yml`、`.env`、端口和卷都不用改（见 [deployment.md](deployment.md) 第 6 节）。

```bash
# 1. 备份数据卷（先停止；备份文件里是你的整个数据库，不要提交）
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data-v04.tgz -C /data .

# 2. 拉取 v0.5 并启动（不要用 down -v）
docker compose pull
docker compose up -d

# 3. 找到升级信息和副本的文件名
docker compose logs backend
```

日志里应该有一行类似 `Upgraded the database to schema 2. A copy of the previous version was saved first: /data/backups/xexam-schema1-20261110-140501.db (…)` 的内容。这份副本是程序自己写的；第 1 步的 `.tgz` 是你自己另存的第二份副本，放在卷外面。

从源码构建：`git pull && docker compose -f docker-compose.build.yml up -d --build`。不用 Docker 时，副本写到数据目录（`--data` 或 `XEXAM_DATA`）里的 `backups/`。

### 1.3 升级后核对

- [ ] `/api/info` 的 `version` 是 `0.5`，`bankCount`（上架的题数）和升级前一样。
- [ ] **Question banks**（题库管理）里列出一个题库，包含你所有的工作表，题数和升级前一样。
- [ ] **Study records**（学习记录）里列出你以前的考试，**Study records → Wrong-answer review**（学习记录 → 错题复习）里的题数和以前错题本里的一样。
- [ ] （可选）选 **Update / replace an existing bank**（更新 / 替换已有题库），选中这个题库，上传和上次相同的 Excel 文件。预览应该提示这份文件的题目和现在完全一样，不需要导入。然后取消。这证明题目 id 没有变。
- [ ] 答几道题并交卷。新的记录里能看到每道题和你的答案。

### 1.4 回滚到 v0.4

回滚就是**恢复迁移前写下的副本**，并运行 v0.4 的镜像。迁移之后在 v0.5 里做的一切都会丢失：新题库、导入、考试记录和设置。

```bash
# 1. 停止 v0.5（不要用 down -v）
docker compose stop

# 2. 找到副本。正常停止后，/data 里不会有 xexam.db-journal。
docker run --rm -v xexam_data:/data alpine ls -l /data /data/backups

# 3. 把副本放回去，作为 xexam.db（文件名用第 2 步看到的）
docker run --rm -v xexam_data:/data alpine cp /data/backups/xexam-schema1-20261110-140501.db /data/xexam.db

# 4. 在 docker-compose.yml 里，把两个镜像标签从 :latest 改成 :v0.4
#      image: ghcr.io/yumin-space/xexam-backend:v0.4
#      image: ghcr.io/yumin-space/xexam-frontend:v0.4
docker compose up -d
```

从源码构建时，改为检出标签：`git checkout v0.4 && docker compose -f docker-compose.build.yml up -d --build`。

第 3 步也可以换成恢复你在第 1.2 节自己做的 `.tgz`（恢复命令见 [database.md](database.md) 第 5 节），里面是同样的数据。第 4 步无论如何都要做。

**不要跳过恢复副本这一步、直接在迁移后的数据库上运行 v0.4**，至少在你有不止一个题库之后不要这样做。v0.4 能打开这个文件，但会把所有题库当成一个：它的下一次导入会把其他所有题库的题都下架，而且会把考试记录删减回 60 条。如果已经这样做了，v0.5 下次启动时会把 v0.4 新加的题归到 legacy 题库里（[database.md](database.md) 第 6.2 节）。

以后要再升级，把镜像标签改回 `:latest`，然后运行 `docker compose pull && docker compose up -d`。迁移会再执行一次，并写一份新的副本。

### 1.5 测试过什么

只用合成数据（`backend/tests/v05_banks_records_settings.rs` 和 `db_golden.rs`）：

- `sample.db`（旧 Python 实现写出的数据库）迁移到 schema v2 后，每个题目 id、统计、考试记录和导入记录都保留。它的备份仍然报告 schema 1，内容和原来的行完全相同。
- 如果备份写不出来，数据库不会被迁移，也不会被改动。
- 再次打开迁移后的数据库，文件一个字节都不会变。
- 旧版程序写进 schema v2 文件的行，会在下次启动时归到 legacy 题库。
- 新建的数据库和迁移后的数据库结构相同。

没有测试：迁移**你自己的**数据库。想放心的话，先在副本上做一遍（和第 3 节「在副本上试新版」一样）。

## 2. 从 Python 到 Rust（v0.3）：原则

1. **不碰原件。** 先备份，先在副本上试，确认没问题再切。
2. **随时能回去。** 到 v0.4 为止，两个实现读写同一个文件、同一份 schema（迁移时用合成数据演练过双向往返，见第 6 节）。从 v0.5 起 schema 多了新的表和列；要从 v0.5 回去，就得恢复迁移前写下的副本（第 1.4 节）。
3. **旧实现没有丢。** 旧代码在 Git 标签 `python-v0.2`；旧镜像 `ghcr.io/yumin-space/xexam:v0.2.2`（以及 `:v0.2`、`:v0.2.1`）仍在 GHCR，不会被新流程覆盖。仓库的主干上已经没有 Python 代码。

## 3. 从 Python 到 Rust：升级前清单

按顺序做，每一步都不改动真实数据库。

- [ ] **备份数据卷**（先停旧容器）：

  ```bash
  docker compose stop
  docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar czf /backup/xexam-data.tgz -C /data .
  docker compose start
  ```

  备份文件里有你的整个数据库，**不要提交到 Git**（`.gitignore` 已忽略 `*.tgz`）。
- [ ] **验证备份**：解压到一个临时目录，`sqlite3 work/xexam.db "PRAGMA integrity_check"` 必须输出 `ok`（命令见 [database.md](database.md) 第 5 节）。
- [ ] **在副本上试新版**：把解压出来的 `xexam.db` 放进一个**空的临时目录**，用它启动后端，不用命名卷：

  ```bash
  cd backend && cargo build --release
  XEXAM_DATA=/tmp/xexam-trial ./target/release/xexam-backend serve --no-browser
  ```

  浏览器打开 <http://localhost:8765>，确认题数、错题本、成绩历史和旧版一致。（从 v0.5 起，这一步也会把副本迁移到 schema v2，并在临时目录里写出 `backups/`。）**试完删掉这个临时目录。**
- [ ] **记下旧版的数字**：`/api/info` 的 `bankCount`、首页上的总题数、错题本里的题数，升级后逐个核对。
- [ ] **确认 `.env`**：`XEXAM_ALLOWED_HOSTS` / `XEXAM_BIND` 等沿用旧值即可，变量名没变。

## 4. 从 Python 到 Rust：升级

```bash
# 1. 停旧版（不要 down -v）
docker compose stop

# 2. 删掉旧的容器（只删容器，不删卷）。旧版的服务和容器都叫 xexam，新版的是 frontend / backend，
#    如果不删，旧容器会留在项目里成为「孤儿」，而且它的端口映射还占着 8765，新的 xexam-frontend 会因端口冲突起不来。
docker rm xexam

# 3. 换上新的 docker-compose.yml（现在有两个服务），拉取并启动（同一个卷 xexam_data、同一个端口）
docker compose pull
docker compose up -d

# 4. 看两个容器是否 healthy
docker compose ps
```

`docker rm xexam` 只删除**容器**，不会删除数据卷 `xexam_data`（那需要 `docker volume rm` 或 `down -v`，不要做）。

从 Python 版直接升级到 v0.5，也会执行第 1 节的 schema v2 迁移，同样会先写备份副本。

升级后核对：

- [ ] `/api/info` 的 `version` 是你安装的版本（目前是 `0.5`），`bankCount` 和升级前一样。
- [ ] 题数、错题本里的题数、成绩历史都和升级前一样。
- [ ] 随便答几道题、交卷，成绩出现在历史里。
- [ ] （可选）再上传一次同一份 Excel（从 v0.5 起：选 **Update / replace an existing bank**（更新 / 替换已有题库），目标就是你唯一的那个题库）：预览应该显示「新增 0、更新 0、下架 0」——这证明新旧解析器给同一批题算出了**相同的题目 id**。如果这里出现大量「下架 + 新增」，**立刻取消并回滚**（第 5 节）。

> ⚠️ 新旧版的项目名（`xexam`）、卷（`xexam_data`）和端口都相同，所以**不能同时运行**。旧版的容器名是 `xexam`，新版是 `xexam-frontend` 和 `xexam-backend`，所以升级时要像上面第 2 步那样把旧容器删掉。

## 5. 从 Python 到 Rust：回滚

可以回到 Python 版。旧版的 compose 文件已不在主干上，从标签里取：

```bash
# 1. 停新版（不要 down -v）
docker compose stop

# 2. 取出旧版的 compose 文件，起旧版（它拉取 ghcr.io/yumin-space/xexam:latest，如需固定版本请把 :latest 改成 :v0.2.2）
git show python-v0.2:docker-compose.yml > docker-compose.legacy.yml
docker compose -f docker-compose.legacy.yml up -d
```

注意旧 compose 里的镜像标签是 `:latest`，**而 `ghcr.io/yumin-space/xexam:latest` 在本次发布后不再更新**（新镜像用的是 `xexam-frontend` / `xexam-backend` 两个新名字），所以它仍指向最后一个 Python 版本；保险起见请显式写成 `:v0.2.2`。

**从 v0.3 或 v0.4 回滚**时，旧版继续使用同一个 `xexam_data` 卷，数据不丢。这两个版本写进去的数据（新导入的题、新成绩）旧版都读得懂：它们不改 schema、不改 `journal_mode`、不加表和列、不设 `user_version`，也不留 `-wal` / `-shm` 文件。

**从 v0.5 回滚**时，schema 已经多了新的表和列。先按第 1.4 节的第 1–3 步恢复迁移前写下的副本，再像上面那样启动旧版。

如果连数据也要回到升级之前（比如怀疑新版写坏了）：

```bash
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar xzf /backup/xexam-data.tgz -C /data
docker compose -f docker-compose.legacy.yml up -d
```

> 恢复会用备份覆盖 `/data`——备份之后在新版里做过的所有改动都会丢。只有确认需要时才做。

不用 Docker 的回滚：`git checkout python-v0.2`，运行 `python server.py`（需要 Python 3.8+），数据目录指向同一个 `xexam.db`。

## 6. 从 Python 到 Rust：已经演练过什么

迁移时（Python 代码还在的时候）做过下面这些，**全部只用虚构数据**，没有读取、打印或修改过任何真实数据库。其中需要实际运行旧 Python 代码的脚本，已随旧代码一起移除；结论由已提交的快照测试继续守着。

| 演练 | 怎么做 | 结果 | 现在由什么守着 |
|---|---|---|---|
| 新旧双向往返 | 旧 Python 建库 → 备份 + `integrity_check` → Rust 在**副本**上读（与旧版逐字节相同）→ Rust 导入第二份工作簿、记成绩 → 旧 Python 回读（与 Rust 相同）→ 旧 Python 再写 → Rust 再读 | 19 项通过，包括「原件的 sha256 前后不变」 | `db_golden.rs`：`sample.db`（旧实现写出的库）迁移到 schema v2 时每一行都保留，迁移后的文件之后无论怎样打开、读取、关闭，都一个字节不变；新建库的 `sqlite_master` 是旧实现的原文加上 v2 新增的部分 |
| 黄金响应对照 | 172 份旧版响应与新后端逐项比较 | 0 项未解释的差异，23 项登记在案 | `api_golden.rs` 每次 CI 都重放 |
| 安全探针 | 340 项恶意请求同时打旧版和新版，3 种配置 | 0 项未解释的差异，51 项登记在案（多数是新版更严） | 探针脚本已移除；守卫和上传检查由 `api_*.rs`、`parser_hardening.rs`、`nginx_check.py` 测试 |
| 解析器差分 | 49 个合成工作簿，旧解析器和 Rust 解析器的输出逐项比较；85 条题目 id 向量 | 全部一致 | `parser_golden.rs`、`id_vectors.rs` 每次 CI 都跑 |
| 整栈 | `tests/integration/stack_check.py`：起栈、上传 / 导入 / 交卷、删容器后同一个卷重建数据还在、`docker stop` 用时、非 root | 34 项通过 | 每次 CI 都跑 |

## 7. 从 Python 到 Rust：与旧版的有意差异

完整、带理由的表在 [api.md](api.md) 的第 5 节；每一条都登记在 `tests/baseline/golden/allowed_differences_rust.json` 和 `allowed_probe_differences_rust.json`，没登记的差异会让测试失败。对使用者可见的只有：

| 项 | 变化 |
|---|---|
| 架构 | 一个容器变成两个（nginx + backend）；镜像名 `xexam-frontend` / `xexam-backend`；`XEXAM_BIND` 现在指向 Nginx |
| 更严的上传检查 | 任何编码 / 扩展名里的 DOCTYPE 都拒绝；重复的 ZIP 条目名拒绝；条目数、XML 深度、节点数有上限 |
| 错误文案 | 其余中文逐字保留；两处原来会回显库异常文本的位置改成固定中文 |
| 判分 | 仍在浏览器里，没有搬到服务端（规格建议搬，这里有意不搬，理由见 [migration-plan.md](history/migration-plan.md) 的 Q3） |
| iPhone | 需要 iOS 15.4 以上（构建目标 `safari15`） |
| 不再提供 | 后端不再提供静态文件（由 Nginx 提供）；`Server` 响应头 |

v0.5 的 API 变化列在 [api.md](api.md) 的「v0.5 的变化（相对 v0.4）」一节。

## 8. 从 Python 到 Rust：没有验证的东西

- 真正的 Cloudflare Tunnel / Access、iPhone 真机、在真正的 ARM 硬件上运行（arm64 只在 QEMU 下构建并启动验证过）。
- 用**你自己真实数据库**的副本演练：我没有也不该读取它；请按第 3 节在自己的机器上做一遍。

## 9. 从 v0.1 升级

数据库文件名在 v0.2 起是 `xexam.db`。如果数据目录里有别的 `.db` 文件而没有 `xexam.db`，后端启动日志会提醒你；把那个文件改名成 `xexam.db` 再启动即可（旧文件不会被删，也不会被读取）。旧的 `progress.json` 每次启动都会检查一次，库里还没有学习记录、也没迁移过时才迁移，只迁一次、不覆盖已有记录。
