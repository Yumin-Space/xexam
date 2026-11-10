# 迁移与回滚

> 从 Python 版（v0.2.x）切换到 Rust + TypeScript + Nginx 版（v0.3）。**数据库文件和 schema 一个字都没变**，所以切换是「换程序」，不是「搬数据」。
> 决策依据：[migration-plan.md](migration-plan.md)。风险：[risk-assessment.md](risk-assessment.md)。

## 1. 原则

1. **不碰原件。** 先备份，先在副本上试，确认没问题再切。
2. **随时能回去。** 两个实现读写同一个文件、同一份 schema（迁移时用合成数据演练过双向往返，见 §5）。
3. **旧实现没有丢。** 旧代码在 Git 标签 `python-v0.2`；旧镜像 `ghcr.io/yumin-space/xexam:v0.2.2`（以及 `:v0.2`、`:v0.2.1`）仍在 GHCR，不会被新流程覆盖。仓库的主干上已经没有 Python 代码。

## 2. 升级前清单

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

  浏览器打开 <http://localhost:8765>，确认题数、错题本、成绩历史和旧版一致。**试完删掉这个临时目录。**
- [ ] **记下旧版的数字**：`/api/info` 的 `bankCount`、首页上的总题数、错题数——升级后逐个对。
- [ ] **确认 `.env`**：`XEXAM_ALLOWED_HOSTS` / `XEXAM_BIND` 等沿用旧值即可，变量名没变。

## 3. 升级

```bash
# 1. 停旧版（不要 down -v）
docker compose stop

# 2. 删掉旧的容器（只删容器，不删卷）。旧版的服务和容器都叫 xexam，新版的是 frontend / backend，
#    如果不删，旧容器会留在项目里当"孤儿"，而且它的端口映射还占着 8765，新的 xexam-frontend 会因端口冲突起不来。
docker rm xexam

# 3. 换上新的 docker-compose.yml（现在有两个服务），拉取并启动（同一个卷 xexam_data、同一个端口）
docker compose pull
docker compose up -d

# 4. 看两个容器是否 healthy
docker compose ps
```

`docker rm xexam` 只删除**容器**，不会删除数据卷 `xexam_data`（那需要 `docker volume rm` 或 `down -v`，不要做）。

升级后核对：

- [ ] `/api/info` 的 `version` 是 `0.3`，`bankCount` 和升级前一样。
- [ ] 首页的题数、错题本里的题数、成绩历史和升级前一样。
- [ ] 随便答几道题、交卷，成绩出现在历史里。
- [ ] （可选）再上传一次同一份 Excel：预览应该显示「新增 0、更新 0、下架 0」——这证明新旧解析器给同一批题算出了**相同的题目 id**。如果这里出现大量「下架 + 新增」，**立刻回滚**（§4）。

> ⚠️ 新旧版的项目名（`xexam`）、卷（`xexam_data`）和端口都相同，所以**不能同时运行**。旧版的容器名是 `xexam`，新版是 `xexam-frontend` 和 `xexam-backend`，所以升级时要像上面第 2 步那样把旧容器删掉。

## 4. 回滚

任何时候都可以回到 Python 版，**数据不丢**。旧版的 compose 文件已不在主干上，从标签里取：

```bash
# 1. 停新版（不要 down -v）
docker compose stop

# 2. 取出旧版的 compose 文件，起旧版（它拉取 ghcr.io/yumin-space/xexam:latest，如需固定版本请把 :latest 改成 :v0.2.2）
git show python-v0.2:docker-compose.yml > docker-compose.legacy.yml
docker compose -f docker-compose.legacy.yml up -d
```

注意旧 compose 里的镜像标签是 `:latest`，**而 `ghcr.io/yumin-space/xexam:latest` 在本次发布后不再更新**（新镜像用的是 `xexam-frontend` / `xexam-backend` 两个新名字），所以它仍指向最后一个 Python 版本；保险起见请显式写成 `:v0.2.2`。

旧版继续使用同一个 `xexam_data` 卷。新版写进去的数据（新导入的题、新成绩）旧版都读得懂：新后端不改 schema、不改 `journal_mode`、不加表和列、不设 `user_version`，也不留 `-wal` / `-shm` 文件。

如果连数据也要回到升级之前（比如怀疑新版写坏了）：

```bash
docker compose stop
docker run --rm -v xexam_data:/data -v "$PWD:/backup" alpine tar xzf /backup/xexam-data.tgz -C /data
docker compose -f docker-compose.legacy.yml up -d
```

> 恢复会用备份覆盖 `/data`——这之后在新版里做过的所有改动都会丢。只有确认需要时才做。

不用 Docker 的回滚：`git checkout python-v0.2`，运行 `python server.py`（需要 Python 3.8+），数据目录指向同一个 `xexam.db`。

## 5. 已经演练过什么

迁移时（Python 代码还在的时候）做过下面这些，**全部只用虚构数据**，没有读取、打印或修改过任何真实数据库。其中依赖旧 Python 现场运行的脚本，随旧代码一起移除了；结论由已提交的快照测试继续守着。

| 演练 | 怎么做 | 结果 | 现在由什么守着 |
|---|---|---|---|
| 新旧双向往返 | 旧 Python 建库 → 备份 + `integrity_check` → Rust 在**副本**上读（与旧版逐字节相同）→ Rust 导入第二份工作簿、记成绩 → 旧 Python 回读（与 Rust 相同）→ 旧 Python 再写 → Rust 再读 | 19 项通过，包括「原件的 sha256 前后不变」 | `db_golden.rs`：`sample.db`（旧实现写出的库）打开、读取、关闭后一个字节不变；新建库的 `sqlite_master` 与旧实现逐字相同 |
| 黄金响应对照 | 172 份旧版响应与新后端逐项比较 | 0 项未解释的差异，23 项登记在案 | `api_golden.rs` 每次 CI 都重放 |
| 安全探针 | 340 项恶意请求同时打旧版和新版，3 种配置 | 0 项未解释的差异，51 项登记在案（多数是新版更严） | 探针脚本已移除；守卫和上传检查由 `api_*.rs`、`parser_hardening.rs`、`nginx_check.py` 测试 |
| 解析器差分 | 49 个合成工作簿，旧解析器和 Rust 解析器的输出逐项比较；85 条题目 id 向量 | 全部一致 | `parser_golden.rs`、`id_vectors.rs` 每次 CI 都跑 |
| 整栈 | `tests/integration/stack_check.py`：起栈、上传 / 导入 / 交卷、删容器后同一个卷重建数据还在、`docker stop` 用时、非 root | 34 项通过 | 每次 CI 都跑 |

## 6. 与旧版的有意差异

完整、带理由的表在 [api.md](api.md) 的第 5 节；每一条都登记在 `tests/baseline/golden/allowed_differences_rust.json` 和 `allowed_probe_differences_rust.json`，没登记的差异会让测试失败。对使用者可见的只有：

| 项 | 变化 |
|---|---|
| 架构 | 一个容器变成两个（nginx + backend）；镜像名 `xexam-frontend` / `xexam-backend`；`XEXAM_BIND` 现在指向 Nginx |
| 更严的上传检查 | 任何编码 / 扩展名里的 DOCTYPE 都拒绝；重复的 ZIP 条目名拒绝；条目数、XML 深度、节点数有上限 |
| 错误文案 | 其余中文逐字保留；两处原来会回显库异常文本的位置改成固定中文 |
| 判分 | 仍在浏览器里，没有搬到服务端（规格建议搬，这里有意不搬，理由见 [migration-plan.md](migration-plan.md) 的 Q3） |
| iPhone | 需要 iOS 15.4 以上（构建目标 `safari15`） |
| 不再提供 | 后端不再提供静态文件（由 Nginx 提供）；`Server` 响应头 |

## 7. 从 v0.1 升级的提醒

数据库文件名在 v0.2 起是 `xexam.db`。如果数据目录里有别的 `.db` 文件而没有 `xexam.db`，后端启动日志会提醒你；把那个文件改名成 `xexam.db` 再启动即可（旧文件不会被删，也不会被读取）。旧的 `progress.json` 每次启动都会检查一次，库里还没有学习记录、也没迁移过时才迁移，只迁一次、不覆盖已有记录。

## 8. 没有验证的东西

- 真正的 Cloudflare Tunnel / Access、iPhone 真机、在真正的 ARM 硬件上运行（arm64 只在 QEMU 下构建并启动验证过）。
- 用**你自己真实数据库**的副本演练：我没有也不该读取它；请按 §2 在自己的机器上做一遍。
