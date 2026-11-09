# Xexam Phase 0 审计草稿：前端清单与 TypeScript / Vite 迁移（dimension: frontend）

> 范围：只读审计，无实现。标记约定：**[事实]** = 读代码或实际运行验证过（带 file:line 或命令结果）；**[规格]** = 车主 spec 的要求；**[建议]** = 我的推荐。
> 命名：全文只用 xexam。数据库文件实名为 `xexam.db`；spec 中把它写成 `<old-name>.db`，这是 **spec 与实际文件名不一致**，按 `xexam.db` 处理。
> 车主的两个私人工作簿只以"工作簿#1 / #2"指代，只统计过数量，没有复制任何题目文字。

## 0. 依赖的文件与 md5（读取时）

| 文件 | 行数 | md5 |
|---|---|---|
| web/app.js | 865 | 0dd0e1139626fe020f483ff03a02de07 |
| web/style.css | 344 | 7e5a52336451cdbea29fb0517d540c54 |
| web/index.html | 24 | 738e7167a1c666410cc6830e834917a2 |
| web/icon.svg | 12 | c1b10c05289f760c4692b7a0e9aab6d2 |
| web/apple-touch-icon.png | 180x180 RGB, 1192 B | 9bd5349bf439dca786f30e2674a95cc4 |
| server.py | 477 | 48306e1f716d5dadeeedb63e805cebae |
| app/db.py | 308 | 02846cbc21b21da42bb6f680f8638792 |
| app/xlsx_parser.py | 410 | 239b594ef8cbba5206e9a6f116178afe |
| VERSION | `0.2` | 15085825464d9079e36d615be010435c |
| Dockerfile | | b031d35fad7d2bae6c2777be520e0399 |
| docker-compose.yml | | 160023431bbe53035860a022124df9ae |
| README.md | | **读取过程中被别人改了三次**：567104f5... -> c374112a... -> 最终 b1cc377271ee6ba1c97d36631b3ca14b。我引用的 README 行号以最终版为准：判分在浏览器 README:166、时序图 `B->>B 判分` README:311、`POST /api/attempt（每题只发 {题目 id, 对/错}）` README:312、"提交的只有 题目 id + 对错" README:319。README 还会继续变，定稿前请复核行号 |

git：HEAD = 8cec408（v0.1）；web/app.js 相对 v0.1 只有 3 处小改（localStorage 键名改前缀、上传返回非 JSON 时的提示、"登录已过期"提示文字），见 `git diff web/app.js`。

## 1. 现状：前端是什么

[事实] 单页、零构建、零依赖、一个经典 `<script>`：
- `web/index.html:14-22`：`<link rel=icon href="icon.svg">`、`apple-touch-icon.png`、`style.css`，`<div id="app">`、`<div id="toast" role="status" aria-live="polite">`，末尾 `<script src="app.js"></script>`（无 `type`、无 `defer`；运行时验证：`type:""`, `defer:false`, `module:false`）。无内联脚本、无内联样式、无 manifest、无 service worker、无 favicon.ico。
- 体积（实测）：app.js 44451 B（gzip 13750 B）、style.css 21259 B（gzip 5115 B）。
- 无 CSS 动画/过渡/`prefers-color-scheme`（grep 无命中）；`<meta name=color-scheme content=light>`（index.html:7）。字体只用系统字体栈（style.css:21-23），无 `url()`、无 webfont。
- 所有来自 Excel 的文字用 `createTextNode` / `textContent` 写入（app.js:2 注释；`h()` 在 app.js:38）；grep 全文件无 `innerHTML`/`insertAdjacentHTML`/`eval`/`document.write`。**必须在 TS 版保持**（建议 ESLint 禁用 innerHTML）。
- 不涉及 `?.`、`??`（grep 无命中）；用了 `Array.prototype.flat`（app.js:36）、`replaceChildren`（app.js:415,747,846）、`<dialog>.showModal()`（app.js:81）。

### 1.1 屏幕与导航

[事实] 导航是**纯状态驱动**：`S.view ∈ loading | fatal | home | admin | quiz | result`（app.js:11），`render()`（app.js:835-848）按 `S.view` 选择视图并 `replaceChildren` 整体重绘。
- 没有 `pushState` / `replaceState` / `popstate` / `hashchange` / `location.*`（grep 无命中）。运行时验证：`location.pathname/hash/search = ["/","",""]`，`history.length = 2`。
- 所以：**不需要 SPA fallback**。浏览器"后退"会直接离开应用，而不是回到上一屏。
- 原服务端：`/`、`/index.html` 返回页面；未知路径返回 404 `not found`（server.py:260-267；实测 `/nope`、`/api/bank/` 均 404）；路径穿越被挡（server.py:261-265；实测 `--path-as-is /../server.py` 为 404）。
- 屏幕：
  1. **loading**：文字"正在读取题库…"（app.js:839）
  2. **fatal**：连不上服务（app.js:840-841），含"登录已过期"提示
  3. **home**：开始考试设置（工作表多选、模式、题数、打乱）、"继续作答"提示框、学习记录面板、题库信息条、折叠说明、页脚（版本号 + 关闭程序）
  4. **admin**：题库管理（上传区 → 预览 → 确认；现有题库；导入记录）
  5. **quiz**：答题（题面/选项/热点小项、日/中切换、标记、答题卡、计时、进度条、键盘）
  6. **result**：成绩 + 每题对错条 + 答案与解析（过滤"只看错题 / 全部题目"）
  7. 叠加层：`<dialog>`（`ask()`）与 `#toast`
  8. 已关闭屏（`quitProgram` 之后，app.js:415）

## 2. 全局状态

[事实] 单一对象 `S`（app.js:9-23），外加模块级变量 `toastTimer`（app.js:62）。

| 字段 | 类型/含义 | 说明 |
|---|---|---|
| `lang` | `'ja'|'zh'` | 初值读 localStorage `xexam.lang.v1`，只有 `'zh'` 才是 zh（app.js:10） |
| `view` | 见上 | |
| `info` | `{version, canShutdown}` | 来自 `/api/info`（`arch`、`bankCount` 页面未使用） |
| `admin` | `{preview, busy, imports, done, error}` | 进入/离开 admin 时整体重置（app.js:421, 431） |
| `bank` | `/api/bank` 响应 | |
| `progress` | `/api/progress` 响应；交卷/清空后被响应替换 | |
| `bankSig` | 字符串 | `sigOf()` 计算，**从未被读取**（app.js:16,114,134），死代码 |
| `cfg` | `{mode, selected:Set|null, known:[], count:number|null, shuffle}` | 首页配置，跨视图保留；`syncSelection()` 在题库变化后保留已选并自动勾选新工作表（app.js:116-125） |
| `session` | `{qs, answers, flags:Set, idx, base, t0, mode, sheets}` | 进行中的考试 |
| `result` | `{results:[{q,ans,correct}], seconds, mode, prev}` | 刚交的卷 |
| `rvFilter` | `'wrong'|'all'` | |
| `timer` | `setInterval` 句柄 | `render()` 每次先 `clearInterval`（app.js:836） |
| `refresh` | `(()=>void)[]` | 答题页用于就地刷新 aria-pressed / 答题卡 / 进度的闭包列表（app.js:577,582,601...） |

## 3. 函数清单（逐个，一句话 + 分组）

| 组 | 函数 | 行 | 作用 |
|---|---|---|---|
| 常量 | `LS_KEY` `LS_LANG` `MODE_NAME` `SVGNS` | 4-7 | localStorage 键、模式中文名、SVG 命名空间 |
| 状态 | `S` | 9-23 | 见 §2 |
| 工具/DOM | `h(tag, props, ...kids)` | 27-41 | 建 DOM：`class`→className；`on*` 函数→addEventListener；`checked/disabled/value/open`→属性赋值；其余→setAttribute（`true`→`''`，`null/false`→跳过）；子节点 `flat(Infinity)`，字符串→文本节点 |
| 工具/DOM | `paras(text, extra, lang)` | 43-44 | 按行拆成 `<p lang=...>`，空行丢弃 |
| 工具 | `fmtTime(sec)` | 46-51 | `m:ss` / `h:mm:ss` |
| 工具 | `shuffle(arr)` | 53-60 | Fisher-Yates，`Math.random` |
| 工具/UI | `toast(msg)` | 63-69 | 3400 ms 提示条 |
| 工具/UI | `ask({title,body,ok,cancel,danger})` | 71-83 | `<dialog>.showModal()` 确认框，返回 `Promise<boolean>`；Esc/关闭 → false |
| 网络 | `post(path, body)` | 85-89 | JSON POST；失败抛 `Error('HTTP <status>: <body文本>')` |
| 工具/DOM | `svgMark(ok)` | 91-103 | 对/错 SVG 图标 |
| 业务(客户端) | `qStat(id)` `masterStreak()` | 107-108 | 取某题统计；掌握阈值（`progress.masterStreak`，缺省 2） |
| 业务(客户端) | `inNotebook(id)` | 109-113 | **错题本规则**：`wrong>0 && streak<masterStreak` |
| 数据 | `sigOf(b)` | 114 | 题库签名（死代码） |
| 数据 | `syncSelection()` `setBank(b)` | 116-136 | 题库变化后维护已选工作表 |
| 业务(客户端) | `pool()` | 127-130 | 出题范围 = 已选工作表 ∩（weak 模式时 inNotebook） |
| 业务(客户端) | `stateOf(q,a)` | 140-147 | 答题卡状态 0 未答 / 1 热点部分 / 2 已答 |
| 业务(客户端) | `grade(q,a)` | 149-154 | **判分**：热点 = 长度相等且逐项全等；单/多选 = 排序后集合全等；非数组 = false |
| 展示 | `kindLabel(q)` `elapsed(se)` | 156-157 | 题型中文名；`base + (Date.now()-t0)/1000` |
| 持久化 | `saveSession()` `clearSaved()` `loadSaved()` | 161-202 | 暂存/清除/恢复（§5） |
| 流程 | `startSession(qs,mode)` | 206-215 | 新建 session、存档、渲染 |
| 流程 | `resumeSession()` | 217-225 | 从存档恢复（有被删题目时 toast） |
| 流程 | `leaveQuiz()` | 227-233 | "暂存退出" |
| 流程 | `submit()` | 235-265 | 确认框 → 本地判分 → `clearSaved()` → `POST /api/attempt` → 成绩页 |
| 首页 | `renderHome()` | 269-375 | 设置面板、题库信息条、折叠说明、页脚 |
| 首页 | `brand()` `historyPanel()` `quitProgram()` | 377-416 | 标题 logo；学习记录 + 清空；关闭程序 |
| 管理 | `openAdmin()` `closeAdmin()` | 420-435 | 进入时 `GET /api/imports`；返回时若有预览则 `POST /api/import/cancel` |
| 管理 | `uploadFile(file)` | 437-462 | 校验扩展名 → `POST /api/upload` → 解析响应 |
| 管理 | `confirmImport()` | 464-492 | 下架>0 时二次确认 → `POST /api/import/confirm` → 更新题库 |
| 管理 | `pill()` `previewView()` `renderAdmin()` | 494-571 | 预览数字块、预览面板、管理页 |
| 答题 | `renderQuiz()` | 575-671 | 题面、选项/小项、语言开关、答题卡、计时器、进度条 |
| 答题 | `setLang` `syncQuiz` `go` `pick` `pickItem` `toggleFlag` | 673-717 | 切语言（存 LS，保留滚动）；刷新+存档；翻题；选项逻辑；热点小项；标记 |
| 答题 | `keydown` 监听 | 719-731 | ←/→ 翻题，M 标记，A-H 选项；对话框打开或在 INPUT/TEXTAREA/SELECT 时忽略 |
| 成绩 | `renderResult()` `reviewItem()` | 735-831 | 成绩页、过滤、对错条、逐题解析 |
| 入口 | `render()` | 835-848 | 视图分发、设置 `document.title` |
| 入口 | `init` IIFE | 850-865 | 并行 GET 三个接口；任一失败 → `fatal` |

### 3.1 h() 辅助函数的精确语义（TS 化时必须保持）
[事实] app.js:27-41。`props` 中值为 `null/undefined/false` 的键跳过；`class`→`className`；`on…`+函数→事件；`['checked','disabled','value','open']` 走属性赋值；**其他一律 `setAttribute(k, v===true ? '' : v)`**，包括 `style`（7 处：app.js:363, 396, 401, 505, 514, 527, 655）、`lang`、`aria-*`、`title`。

## 4. 全部网络调用

[事实] 全部是相对 URL（`/api/...`），同源；没有设置 `credentials`（fetch 默认 same-origin，会带同源 cookie，Cloudflare Access 的 cookie 靠这个）；没有 AbortController / 超时；服务端无 CORS 头、无 `do_OPTIONS`（server.py 中 grep 无命中），所以前端只能同源调用。

| # | 调用 | 位置 | 请求 | 成功解析 | 失败处理 |
|---|---|---|---|---|---|
| 1 | `GET /api/bank` | app.js:854 | 无 | `r.json()`，**不检查 `r.ok`** | 任一失败（含非 JSON 的 403/登录页）→ `S.view='fatal'`（app.js:861-863） |
| 2 | `GET /api/progress` | 855 | 无 | 同上 | 同上（三个用 `Promise.all`） |
| 3 | `GET /api/info` | 856 | 无 | 同上 | 同上 |
| 4 | `GET /api/imports` | 425, 483 | 无 | `.imports \|\| []` | 全部吞掉，"没有也行" |
| 5 | `POST /api/upload` | 448 | body = `File` 原始字节；头 `Content-Type: application/octet-stream`、`X-Filename: encodeURIComponent(name)` | 以 `Content-Type` 含 json 判断；`d=json`；`!ok` → `d.error \|\| '上传失败（HTTP n）。'`；`ok` 但非 JSON 或缺 `token` → "服务返回的不是预期的内容…登录已过期…"；成功 → `A.preview=d` | 网络异常 → "上传失败：连不上服务…" |
| 6 | `POST /api/import/confirm` | 479 | `{token}` | `d.bank`→`setBank`；`d.result` + `filename` + `total` → `A.done` | 抛出的 `Error.message` 形如 `HTTP 409: {"error": "…"}`；UI 去掉 `^HTTP \d+:\s*` 再 `JSON.parse`，取 `.error`，取不到用"导入失败，题库没有任何改动。"（app.js:485-487）。实测消息格式：`HTTP 400: {"error": "没有成绩可保存"}` |
| 7 | `POST /api/import/cancel` | 430, 529 | `{}` | 忽略 | 吞掉 |
| 8 | `POST /api/attempt` | 252 | `{results:[{id,correct}], sheets, mode, seconds}`（真实 UI 抓到的 body，见 §7） | 整个 progress 替换 `S.progress`；`hist[1]` 若同模式则作"上一次" | 仅 toast"成绩没能保存到学习记录…这一场的结果仍然可以看"（app.js:258），**没有重试** |
| 9 | `POST /api/reset` | 406 | `{}` | 替换 `S.progress` + toast | 通用 toast，不显示服务端原因 |
| 10 | `POST /api/shutdown` | 414 | `{}` | | 吞掉（服务端关得快会断开）；仅 `info.canShutdown` 时显示按钮 |

[事实] 服务端 JSON 用 `ensure_ascii=False`，默认分隔符（`": "`、`", "`），`Content-Type: application/json; charset=utf-8`（server.py:211）。前端只检查 `json` 子串；但 `scripts/smoke-test.sh` 用 `grep -q '"bankCount": 0'` 和 `"version": "$want"`，**依赖带空格的 JSON 格式**——Rust 的 serde_json 紧凑输出会让这个 grep 失败（兼容性提示，见 §14）。

[事实] 所有响应都有 `Cache-Control: no-store` 和 `X-Content-Type-Options: nosniff`（server.py:204-205），静态文件也一样；没有 ETag。

### 4.1 JSON 形状（实测自临时库，端口 8720）
- `/api/info`：`{"version":"0.2","arch":"AMD64","canShutdown":true,"bankCount":5}`
- `/api/bank`：`{file, mtime, error:"", questions:[Q], sheets:[{name, usable, inactive, total, skipped:[]}]}`（db.py:151-155；`error` 恒为 `''`，`sheets[].skipped` 恒为 `[]`）
- `Q`：`{id, sheet, row, kind:'single'|'multi'|'hotspot', tag:''|'ホットスポット', stem, options:[{key,text}], items:[{text,choices:[string]}], answer, translation, zh, explanation, mnemonic, note}`
  - single/multi：`options` 非空、`items:[]`、`answer: string[]`（如 `["A","C"]`）、`zh = {stem, options:string[]} | null`
  - hotspot：`options:[]`、`items` 非空、`answer: number[]`（选项下标，如 `[0,1,0]`）、`zh = {stem, items:string[]} | null`
  - `/api/bank` **把 answer / explanation / mnemonic / translation 在考试前就全部发给浏览器**（实测）
- `/api/progress`：`{version:1, questions:{[id]:{attempts,correct,wrong,streak,last?:boolean,at?:string}}, history:[{at,sheets,mode,total,correct,percent,seconds}]（最多 60 条，新→旧）, masterStreak:2}`
- 预览（`/api/upload` 成功）：`{added,updated,unchanged,restored,removed,skipped,usable,removed_samples:[string],removed_stats,filename,token,sheets:[{name,total,usable,skipped:[{row,reason}],added,updated,removed,restored,skippedMore,skippedCount}]}`；`token` 22 字符
- 确认（`/api/import/confirm`）：`{result:{…同预览但无 filename/token，sheets 无 skippedMore/skippedCount}, bank:{…}}`
- 错误体：JSON `{"error":"…"}`；GET 的 Host 拒绝是 **text/plain** 403（server.py:247），POST 的拒绝是 JSON 403（server.py:311）；未知 GET 路径是 text/plain 404，未知 POST 路径是 JSON 404。

## 5. localStorage（精确载荷）

[事实] 只有两个键，全部在 try/catch 里（隐私模式/存不了时静默）。

| 键 | 值 | 写入时机 |
|---|---|---|
| `xexam.lang.v1` | 字符串 `'ja'` 或 `'zh'`（读取时：只有 `'zh'` 算 zh，其余一律 ja，app.js:10） | `setLang`（app.js:676） |
| `xexam.session.v1` | JSON（实测抓取）：`{"v":1,"ids":["4ae830b3bd98",…],"answers":{"4ae830b3bd98":["B"],"822349f3b921":[0]},"flags":["94e4bfe00e8d"],"idx":2,"elapsed":0,"mode":"all","sheets":["demo"]}` | `saveSession`（app.js:161-170），在 `startSession`、`go`、`syncQuiz`（选/小项/标记）、`leaveQuiz` 时 |

- `answers[id]`：单/多选 = 选项键数组（多选已排序）；热点 = 与 `items` 等长的数组，未答项为 `null`。实测（probe5）：3 个小项的热点题只点第 2 项的第 2 个选项 -> 存成 `[null,1,null]`；再点一次同一选项 = 取消，全部为 `null` 时该题的键从 `answers` 里整个删除（`afterSecondClickHasKey:false`）。
- `v:1` 被写入但**从不读取校验**（`loadSaved` 只检查 `Array.isArray(d.ids)`，app.js:176-177）。
- `elapsed` 是整数秒（`Math.floor(elapsed(se))`），只在上述操作时更新：直接关标签页会丢失"最后一次操作之后"的用时；暂存退出后到继续之间的时间不计入（`t0` 重置，`base`=存档 elapsed，app.js:220）。
- `loadSaved()`（app.js:174-202）：丢掉题库里已不存在的 id（`dropped` 数，toast 提示）；热点答案按 `choices.length` 规范化；选项键必须属于该题；`idx` 钳制；`mode` 白名单；`d.idx|0`、`d.elapsed|0` 强制取整。`renderHome` **每次渲染都调用一次**（app.js:271）。
- `submit()` 在 POST **之前**就 `clearSaved()`（app.js:248）：POST 失败时存档已丢，只能看本场结果；"放弃这场"也清。
- [事实] v0.1（已推送）用旧名称前缀的键；工作树 v0.2 改成了 `xexam.*`（`git diff web/app.js` 第 4-5 行）。结果：从 v0.1 升级时，旧键下暂存的考试和语言偏好不会被读取（这是 v0.2 已有的行为，不是迁移引入的）。
- [建议/标准行为，未实测] localStorage 按"源（协议+主机+端口）"隔离：新的 Nginx 若换了端口或域名，暂存会话和语言偏好在新源下是空的，且无法跨源迁移。部署文档要提醒。

## 6. 浏览器里算的 vs 服务端算的

| 项目 | 位置 | 说明 |
|---|---|---|
| 判分 `grade()` | **浏览器**（app.js:149-154，submit 内 243-246） | 服务端只收 `{id, correct}` |
| 作答完成度 `stateOf()` / 答题卡 | 浏览器 | |
| 出题范围 `pool()`、错题本 `inNotebook()` | 浏览器（app.js:109-130）；阈值 `masterStreak` 来自服务端 `progress.masterStreak`（db.py:23,253） | 首页的错题数、`weak` 模式可用性（app.js:272-273）、每工作表"错题 N"（app.js:305）都在浏览器算 |
| 打乱 `shuffle()`、题数截取 | 浏览器，`Math.random` | "再考一次"总是重新打乱，**忽略**"打乱题目顺序"复选框（app.js:764）；"只重做错题"同理（765） |
| 计时 | 浏览器 `Date.now()`（app.js:157,660） | `seconds` 由客户端上报 |
| 成绩百分比（成绩页） | 浏览器 `Math.round(100*good/n)`（app.js:738） | |
| 成绩百分比（学习记录） | **服务端** Python `round(100*good/total)`（db.py:269） | 见下，两者可不一致 |
| streak / attempts / wrong 累计、历史截断 60 条 | 服务端（db.py:255-272） | 错题本规则的"原料"在服务端，"规则"在浏览器 |
| 版本号、canShutdown | 服务端 `/api/info` | |

### 6.1 已验证的现存缺陷：百分比舍入不一致
[事实] 同一场 1/8 答对：成绩页显示 `Math.round(12.5)=13`，服务端存 `round(12.5)=12`（Python 银行家舍入）。实测 `POST /api/attempt` 8 题 1 对 → 历史 `percent:12`，浏览器 `Math.round(100*1/8)=13`；另验证 5/8（62.5→62 vs 63）、1/40（2.5→2 vs 3）也不同。结果：成绩页显示 13%，回到首页的历史列表显示 12%。
- [建议] Rust 后端复刻时**不能**用 `f64::round()`（半数远离零舍入，得到 13），需用 ties-to-even（`f64::round_ties_even`，Rust 官方文档核验：自 1.77.0 稳定；`f64::round` 文档原文是"half-way ... round away from 0.0"），否则历史数据与旧版逐项对比会出现"无法解释的差异"。是否顺手修掉这个不一致（让成绩页读服务端 percent）是车主决定，见 openQuestions。

### 6.2 服务端对 `/api/attempt` 的信任（实测）
[事实] `clean_attempt`（server.py:132-145）：最多 2000 条 `results`，`id` 是 ≤64 字符的字符串，`correct` 必须是 bool；`sheets` ≤50 个、各 ≤80 字符；`mode` 不在白名单则变 `all`（实测 `"bogus"`→`all`）；`seconds` 钳到 0..7 天（实测 99999999→604800）。
- **不验证 id 是否存在于题库**：实测提交 `zzz-not-a-question` 会产生统计记录 `{attempts:1, wrong:1, …}`。
- **不去重**：同一 id 提交两次 `correct:true` → `attempts:2, streak:2`。
- **不验证正确与否**：服务端完全信任 `correct`；`total` = 提交条数，不是当次实际题数。
- 因此服务端目前只"记录客户端所说"。这是现状的信任模型：单用户、无登录、前面可有 Cloudflare Access。

## 7. 真实 UI 流程抓包（headless Chrome，端口 8720 / DevTools 9640-9643，临时库）
- 开始考试→交卷（含 1 题未答）：对话框文字"还有 1 道题没答完 / 没答完的题按答错计分。现在交卷吗？"；实际 body：`{"results":[{"id":"94e4bfe00e8d","correct":false},…],"sheets":["demo"],"mode":"all","seconds":0}`，头只有 `Content-Type: application/json`；结果页 `得分 67%`，`.rv` 2 项；交卷后 localStorage 为空；首页出现历史行和"错题 2"、"2 题待巩固，连续答对 2 次后移出"。
- 21 MB 上传（Chrome fetch）→ 413 JSON `{"error":"文件太大了（最大 20 MB）…"}`，UI 能正常显示（原服务端会把多余数据读掉再回应，server.py:272-290）。
- `.xls` 文件名：客户端直接拦截"「a.xls」不是 .xlsx 文件…"；乱码 `.xlsx`：服务端 400 的中文消息原样显示。
- 同一浏览器 `ask()` 关闭（Esc/`close()`）→ resolve(false)。

## 8. 与服务端 pending 机制的前端相关事实（影响 UI 回归测试）
[事实] 服务端 pending 是**全局单槽**（server.py:150-175）：
- 再上传一次会让上一个 token 失效（实测：先传 A 后传 B，确认 A → 409 `这次上传的预览已经过期或已被处理，请重新上传。`）。
- `POST /api/import/cancel` 不看 token，直接丢弃当前槽（实测）。
- token 一次性，确认后同 token 再确认 → 409（实测）；错 token → 409。
- 因此"两个标签页同时上传"会互相踢掉；前端对此没有特殊处理，只是显示服务端的 `error`。
- `closeAdmin()` 在有预览时会静默取消（app.js:430）。

## 9. 规格冲突分析：判分与错题本在浏览器里

### 9.1 冲突
- **[规格]** "frontend 不做业务逻辑、不做权威校验、不生成可信 token"。
- **[事实]** 现状判分（`grade`）、错题本规则（`inNotebook`）、出题范围（`pool`）、百分比、计时、打乱都在浏览器；服务端只存 `{id, correct}`（README:166、312、319 也明确写了这一点，行号为最终 md5 b1cc3772... 版）。另外 `/api/bank` 在考试前就把答案和解析发给浏览器。

### 9.2 若要"字面满足"spec，必须移走什么

| 要移走的逻辑 | 需要的新/改 API | 兼容代价 |
|---|---|---|
| 判分 | `POST /api/attempt` 改为接收原始作答 `results:[{id, answer}]`（`answer: string[] \| (number\|null)[]`），服务端用库里的 `answer` 判分，返回每题对错 | **破坏**"保持与现有前端契约兼容"：旧 body `{id, correct}` 要继续接受才能做新旧对比；需 DTO 版本化（可同时接受 `correct` 或 `answer`） |
| 答案不提前泄露 | `/api/bank` 去掉 `answer/explanation/mnemonic/translation`；交卷响应或新接口 `POST /api/review`（或在 attempt 响应里）返回答案与解析 | `/api/bank` 形状变化是**破坏性**的；旧 Python 前端不能再用；"POST 失败也能看结果"（app.js:258）的现有体验消失；暂存会话恢复后断网不能交卷 |
| 出题范围 / 错题本 | 新增 `GET /api/notebook`（或 `progress.notebook: string[]`），或 `POST /api/session {sheets, mode, count, shuffle}` 由服务端抽题并返回 | 后者需要服务端会话状态；暂存/继续如果也要上服务端，就要新表 = **schema 变更**（spec 要求论证+迁移+可回滚）；前者只是新增字段，旧前端忽略，低成本 |
| 计时/秒数 | 服务端记录 session 开始时间 | 同样需要服务端会话状态 |
| 百分比 | 前端改读服务端 `percent`（已存在） | 低成本，同时修掉 §6.1 的不一致，但会改变成绩页显示的半数舍入结果 |

### 9.3 建议（针对这个单用户训练应用）
1. **Phase 3/4 保持判分、错题本、出题范围在浏览器**，作为有意识的、文档化的偏离（写进 docs/security.md："成绩由客户端报告，服务端记录；信任模型 = 单用户 + 外层 Cloudflare Access"）。理由：
   - 没有第二个对手：唯一能"作弊"的人是车主本人，伤害的只是自己的统计；
   - 答案本来就随 `/api/bank` 到了浏览器，服务端判分并不带来保密性；
   - 判分和错题本是最关键的业务规则，一次性迁移语言+架构+契约三件事叠加风险过大，违反"保持行为/API 兼容、无法解释的差异=缺陷"的验收口径；
   - 迁移后的 Phase 1 基线对比需要旧 Python 当 oracle，契约一变就失去 oracle。
2. 把这些逻辑隔离成纯函数模块 `src/domain/`（`grade`、`stateOf`、`inNotebook`、`pool`），配 Vitest 单测 + 与 Rust 共享的"黄金用例"（同一批输入的预期输出），保证以后搬到服务端时有现成测试。
3. **低成本、向后兼容的"向 spec 靠拢"选项**（可选，不属于迁移范围，建议作为迁移完成后的独立小版本）：
   - 服务端在 `/api/progress` 增加 `notebook: string[]`（规则由 Rust 单点实现，旧前端忽略）；
   - `/api/attempt` 增加可选 `answers`，服务端重算并只记录"客户端 correct 与服务端重算不一致"的计数（不记录内容、不 token）；
   - 服务端对 id 不存在于题库/重复 id 的处理：现状是照收（§6.2），Rust 版**先逐项复刻**，再作为缺陷项交车主决定。
4. 车主若坚持字面满足，我建议分两步：先做"新增字段、旧字段保留"的加法契约（9.3 第 3 条），最后才删 `/api/bank` 里的答案；每一步都保证旧前端仍可用。

## 10. 现存缺陷与技术债（前端侧，均已验证）
1. **百分比舍入不一致**（§6.1）。
2. **交卷请求在途时的交互空指针**：`submit()` 先 `S.session=null`（app.js:249）、`S.view` 仍是 `'quiz'` 直到 await 结束；此时答题 DOM 仍在，实测在 POST 延迟 700 ms 的期间点击选项/标记/下一题/交卷，抛出 3 个未处理错误（`Cannot read properties of null (reading 'answers'/'flags'/'qs')`）；最终状态正常进入成绩页。TS 的 `strictNullChecks` 会把这类访问全部标出来，建议加 `submitting` 守卫并禁用按钮。
3. `sigOf`/`bankSig` 死代码；`info.arch`、`info.bankCount` 页面未用；`.rvlist` 在 JS 中使用但 CSS 无规则（无害）。
4. `post()` 的错误文本只有 `confirmImport` 解析；`/api/attempt`、`/api/reset` 失败只给通用 toast，Host/Origin 拒绝的可操作原因（server.py:296-306）用户看不到。
5. `session.v` 不校验；暂存无周期性保存（§5）。
6. `zh` 与 `options`/`items` 长度一致性靠服务端解析器保证（xlsx_parser.py:265,280）；前端直接用 `q.zh.options[i]`、`q.zh.items[i]`（app.js:597,607）不校验。[建议] TS 版加廉价防御：长度不等就当作没有中文（保持有效数据下行为不变，满足"翻译解析失败不暴露错位内容"）。
7. `<dialog>` / `showModal()` 要求 iOS Safari ≥ 15.4（caniuse：iOS 14.5-15.3 为 n，15.4 起 y）。在 15.0-15.3 上 `ask()` 内抛 TypeError，"交卷/清空/放弃"按钮无反应。即**现有前端实际的 iOS 下限是 15.4，而不是 15.0**。实测（probe5，Chrome 里把 `HTMLDialogElement.prototype.showModal` 置 `undefined` 来模拟，**不是真 iOS**）：点"交卷"-> 一个未处理的 Promise 拒绝 `d.showModal is not a function`，视图仍是 quiz、会话还在，`<dialog>` 元素被插入 body 但未打开且不会被移除（`strayDialogsInBody:1`）；用户看到的就是"按钮没反应"。
8. **上传进行中点"返回首页"会留下无人取消的服务端 pending**（实测，probe5）：`closeAdmin()` 只在 `S.admin.preview` 已存在时才发 `POST /api/import/cancel`（app.js:430）；上传请求在途时 `preview` 还是 `null`，"返回首页"按钮此时**没有被禁用**（`backDisabledDuringUpload:false`，因为 `busy` 只禁用了"选择 Excel 文件"按钮，app.js:538；返回按钮 app.js:546 不受 busy 影响）。之后上传响应到达，写进的是旧的 `A` 对象（已被 `closeAdmin()` 换掉），UI 回到首页，但服务端已有一个 pending 项：用旧 `A.preview.token` 去确认仍返回 **200**（`confirmStatusWithOrphanToken:200`，临时库）。后果有限（要拿到 token 才能确认，单槽、30 分钟过期），但属于"预览后未显式取消"的状态泄漏；TS 版建议在 busy 时禁用返回或在响应到达时检查视图后补发 cancel。

## 11. 浏览器支持下限与 Vite / TypeScript / CSS 处理

- **[规格/用户]** iOS Safari 15。
- **[事实] 前端用到的较新特性**：`<dialog>.showModal`（15.4，app.js:81）；CSS `:has()`（15.4，style.css:88）；`:focus-visible`（15.4；15.0-15.3 默认关，style.css:39）；`accent-color`（style.css:90,98；MDN：Safari 15.4 起部分支持，随后全面支持）；`aspect-ratio`（Safari 15，style.css:187）；`clamp()`、`env(safe-area-inset-*)`、`padding-inline`、`scroll-margin-top`、`replaceChildren`（来自我的记忆，iOS 14 级别，**未逐项核验**）；`Array.flat`（iOS 12+，caniuse 核验）。
- **[事实] Vite 8.3.7 默认 `build.target = 'baseline-widely-available'` = chrome111 / edge111 / firefox114 / safari16.4 / ios16.4**（vite.dev 文档，该主版本固定日期 2026-01-01）；`build.cssTarget` 默认同 target；`build.cssMinify` 默认 `'lightningcss'`；转换器是 Oxc；Vite 不做 polyfill；Vite 自身的原生 ESM 下限是 Safari ≥ 11.1（文档）。
- **[建议]**
  - `vite.config.ts` 显式设 `build.target: ['es2020', 'safari15']`（或 `'ios15'`；写法依文档"浏览器+版本"模式，**未实际构建验证**），否则默认 ios16.4 会把 iOS 15 排除在转译目标之外。`cssTarget` 跟随 target。
  - CSS 保持原样搬运，**不要改版**（避免视觉回归）；当前 CSS 不含嵌套/新语法，Lightning CSS 主要是压缩。
  - 不引入 `@vitejs/plugin-legacy`（iOS 15 支持原生 ESM）。
  - `tsconfig`：`target: ES2020`、`lib: [ES2020, DOM, DOM.Iterable]`、`module: ESNext`、`moduleResolution: bundler`、`strict: true`、`noUncheckedIndexedAccess: true`、`noImplicitOverride`；`useUnknownInCatchVariables`（strict 已含）。
  - 下限要不要改成 15.4（与现状一致）由车主确认（openQuestions）。
- **工具版本（本次只读 `npm view`，未安装任何东西）**：vite 8.3.7（engines node `^20.19.0 || >=22.12.0`）；typescript `latest` = 7.0.2，但 typescript-eslint 8.72.2 的 peer 是 `typescript >=4.8.4 <6.1.0`，openapi-typescript 7.13.0 的 peer 是 `typescript ^5.x` → [建议] 先用 TS 5.9.x（或 6.0.x）并在 Phase 4 试装后锁定，不要盲目上 7.x；vitest 5.1.0；@playwright/test 1.65.0（node ≥20）；eslint 10.14.0；jsdom 31（node `^22.22.2 || ^24.15.0 || >=26`）；happy-dom 21.0.4。本机 node 为 v26.13.0。

## 12. Vite 开发服务器代理 /api 与 Host/Origin 控制

[事实] 用 curl 模拟代理行为，对原服务器验证（Host/Origin 是原服务器的现有安全控制，必须保留）：

| 场景 | Host | Origin | 结果 |
|---|---|---|---|
| Vite 默认代理（`changeOrigin:false`，http-proxy-3 默认 false，文档核验） | `localhost:5173` | `http://localhost:5173` | **200**（host_name 去端口，localhost 在默认白名单，server.py:439；Origin 与 Host 同主机） |
| `changeOrigin:true` | `127.0.0.1:8720` | `http://localhost:5173` | **403**（Origin 主机 ≠ Host 主机） |
| 局域网 IP 开发 | `192.168.1.5:5173` | 同 | **403** Host 不在白名单（除非 `XEXAM_ALLOWED_HOSTS`） |

[建议] `vite.config.ts`：

```ts
server: { port: 5173, proxy: { '/api': { target: 'http://127.0.0.1:8765', changeOrigin: false } } }
```
不要 `rewrite`（URL 契约不变）；不要开 `changeOrigin`；开发时只用 `localhost` 或 `127.0.0.1` 打开页面。Vite 自己的 `server.allowedHosts` 默认放行 localhost 和所有 IP（文档核验）。`vite preview` 的代理文档本次未读（notVerified）。不要让前端直连另一端口（无 CORS、无 OPTIONS，必然被浏览器拒绝，[推断]）。

[建议/Nginx，未实测] Nginx 默认转发给上游的 `Host` 是上游名（`$proxy_host`），后端会看到 `backend:xxxx` 而 403 全部请求；必须显式 `proxy_set_header Host $http_host`（或 `$host`）并保留 `Origin`，后端白名单继续用 `XEXAM_ALLOWED_HOSTS/ORIGINS`。Cloudflare Tunnel 场景下浏览器 Origin 是公网域名，Host 可能被 cloudflared 改写（server.py:223-227 注释），Nginx 在两者之间不能让情况变差。

## 13. 静态资源、缓存、SPA fallback

- `frontend/public/` 应放：`icon.svg`（508 B）、`apple-touch-icon.png`（180x180，1192 B），**保持在站点根、不加哈希**（`scripts/smoke-test.sh` 通过 `/icon.svg`、`/apple-touch-icon.png` 取；iOS "添加到主屏幕"依赖前者）。`index.html` 中改成绝对路径 `/icon.svg` 等（Vite 对 public 资源要求绝对路径）。
- 缓存：现状全部 `no-store`，无需 cache-busting。迁移后 [建议] `index.html` 用 `no-cache`；Vite 产物 `assets/*-[hash].js|css` 用 `immutable` 长缓存；`/api/*` 保持 `no-store`；public 资源短缓存。
- SPA fallback：**不需要**（无 URL 路由）。[建议] Nginx 用 `try_files $uri =404`（保持现在"未知路径 404"的行为），`/` 返回 index.html。若将来加深链接才需要 fallback，那是功能变更。
- 无 favicon.ico、无 manifest、无 SW（现状如此）。
- CSP：[事实] 页面无内联脚本。若 Nginx 设 `style-src 'self'`：`h()` 用 `setAttribute('style', …)` 的 7 处会被 `style-src-attr` 拦截（实测 `setAttribute` 被拦，`el.style.height=` 与 `style.cssText=` 不被拦）。[建议] 要么把这 7 处变成 CSS 类，要么 `h()` 的 `style` 键改走 `el.style.cssText`；动态高度（学习记录柱状条 app.js:396）用 `el.style.height`。`script-src 'self'` 可行。Vite 的 modulepreload polyfill 与 CSP 的关系本次未验证。
- 上传大小：UI 文案写"最大 20 MB"（app.js:537），前端没有客户端大小检查。[风险，未实测] Nginx `client_max_body_size 20m` 超限会返回 **HTML** 的 413，前端会走 `isJson=false` 分支，显示通用的"上传失败（HTTP 413）。"（友好文案丢失）；且 Nginx 提前拒绝可能让浏览器看到网络错误而显示"连不上服务"。[建议] `error_page 413` 返回与现状相同形状的 JSON `{"error": …}`，并在集成测试里用真实 Chrome 验证；Rust 端仍要独立限制真实 body 大小。

## 14. 兼容性问题清单（前端相关）
1. **全局名**：经典脚本时，函数声明成为 `window` 属性（实测 `openAdmin`、`uploadFile`、`confirmImport`、`render`、`post`、`grade`、`pool`… 共 40 个自有属性）；`const` 绑定（`S`、`LS_KEY`、`LS_LANG`、`MODE_NAME`、`SVGNS`、`qStat`、`masterStreak`、`sigOf`、`kindLabel`、`elapsed`、`clearSaved`、`brand`…）不是 window 属性，但在全局词法作用域内可被 `Runtime.evaluate` 读到（实测 `window.S === undefined`，`typeof S === 'object'`）。
   车主的脚本依赖：`tunnel_sim.py` 调 `openAdmin()`、`uploadFile(new File(...))`、`confirmImport()`，读 `S.admin.error/preview/done`、`S.bank.questions`；`scenes.py` 还用 `S.view/session/result/progress/cfg/lang`、`render()`、`clearSaved()`、`LS_LANG`、`post()`。**ES 模块后这些全都不存在**。
   [建议] 生产构建不暴露全局；测试改用 Playwright 通过真实 UI 操作（`#xlsx-input` 的 `setInputFiles`、点击按钮）与 `localStorage`；如确需状态探针，仅在 `import.meta.env.MODE === 'test'` 构建里挂 `window.__xexam = { state }`。`host_test.py` 只走 HTTP，不受影响。
2. **script 形式**：`<script src="app.js">` → `<script type="module" src="/src/main.ts">`（模块天然 defer，顶层绑定不再全局）。
3. **版本页脚 / `/api/info`**：版本只来自后端 `/api/info.version`（server.py:37-47 读 VERSION），页脚 `Xexam v{version}`（app.js:372）；`canShutdown` 控制"关闭程序"按钮。后端契约保留，不要把前端构建版本当作页脚版本。
4. **JSON 格式**：smoke-test 的 grep 依赖 `": "`（§4）。
5. **localStorage 源变化**（§5）。
6. **相对 URL**：全部相对根路径，Nginx 同源提供即可；不要给 Vite 设 `base` 子路径。
7. **413 / 网络错误文案**（§13）。
8. **Host/Origin**（§12）。
9. **百分比舍入**（§6.1）。
10. 响应的 `Content-Type` 必须仍含 `json`（上传分支靠它判断，app.js:452）。

## 15. 严格 TypeScript 迁移要点
- **`h()`**：用泛型 `h<K extends keyof HTMLElementTagNameMap>(tag: K, props?: Props | null, ...kids: Child[]): HTMLElementTagNameMap[K]`；`Child = Node | string | number | false | null | undefined | Child[]`；`Props` 中 `on*` 值是事件处理函数；property 赋值与 attribute 的分流保持一致（§3.1）。
- **可空 DOM**：`getElementById('app'|'toast')`、`querySelector('dialog[open]')`、`e.target.files[0]`（`FileList|null`、元素可能为 `undefined`）、`e.dataTransfer?.files[0]`、`e.target.tagName`（需 `instanceof Element`）。建议一个 `must(id)`，启动时断言一次。
- **catch 变量**：`unknown`；`post()` 抛的 `Error('HTTP …')` 改为 `ApiError {status, bodyText, serverMessage?}`，但**界面上展示的文字必须保持不变**（`confirmImport` 取 `.error` 的逻辑，app.js:485-487）。
- **判别联合 `Question`**：`kind:'single'|'multi'` 与 `kind:'hotspot'` 区分 `options/items/answer/zh`；会话答案 `Record<string, string[] | (number|null)[]>`，在 `pick`/`pickItem`/`grade`/`stateOf`/`reviewItem` 里按 `kind` 收窄。
- **持久化数据是不可信输入**：`loadSaved` 的 `JSON.parse` 结果是 `unknown`，需运行时类型守卫（保持 `|0` 取整、题库缺失过滤等语义）。
- **后端 JSON**：理想情况下由 OpenAPI 生成（`openapi-typescript`）；注意非显然形状：`answer` 联合、`zh` 可空且按题型两种形状、`last?`/`at?` 可选、预览 `sheets[].skipped` 是对象数组而 `/api/bank` 的恒为 `[]`、确认响应的 `result.sheets` 缺 `skippedMore/skippedCount`。
- `S.timer` 用 `ReturnType<typeof setInterval> | null`；`S.refresh: Array<() => void>`；`Math.max(...hist.map())`、`kids.flat(Infinity)` 需要 `lib` 含 ES2019。
- `noUncheckedIndexedAccess` 会让 `q.zh.options[i]`、`hist[1]` 变 `| undefined`，正好暴露 §10-6 的隐式假设。

## 16. 建议的目录与行数估计（`frontend/`，不重命名 spec 的树）

```
frontend/
  index.html                (~25)   绝对路径 /icon.svg 等；<script type=module src=/src/main.ts>
  package.json, package-lock.json, vite.config.ts (~25), tsconfig.json (~25)
  public/  icon.svg, apple-touch-icon.png
  src/
    main.ts                 (~60)   init、render 分发、keydown 注册
    types/api.ts            (~140)  Question/Bank/Progress/Preview/ConfirmResult/Info（优先由 OpenAPI 生成）
    types/state.ts          (~60)   AppState/Session/ResultState/Cfg
    api/client.ts           (~90)   request<T>、ApiError、JSON/错误体解析、相对 URL
    api/bank.ts             (~25)   getBank、getInfo
    api/progress.ts         (~45)   getProgress、postAttempt、postReset
    api/import.ts           (~80)   uploadXlsx、confirmImport、cancelImport、listImports
    domain/grading.ts       (~70)   grade、stateOf、inNotebook、pool、percent（纯函数，可单测）
    utils/dom.ts            (~80)   h、paras、svgMark、must
    utils/format.ts         (~15)   fmtTime
    utils/random.ts         (~12)   shuffle
    utils/storage.ts        (~70)   安全的 localStorage 读写 + 暂存会话类型守卫（save/load/clear）
    state/store.ts          (~60)   S 的创建与重置
    components/toast.ts (~15), dialog.ts (~25), brand.ts (~12), historyPanel.ts (~55),
               previewView.ts (~65), answerCard.ts (~50), questionView.ts (~100), reviewItem.ts (~75)
    pages/home.ts           (~150)  renderHome + 设置区
    pages/admin.ts          (~120)  renderAdmin + upload/confirm 流程
    pages/quiz.ts           (~130)  renderQuiz + 计时 + pick/pickItem/go/flag/lang
    pages/result.ts         (~85)   renderResult
    pages/status.ts         (~25)   loading / fatal / 已关闭
    styles/main.css         (344)   先原样搬运；之后可按原有 7 个注释分段拆成 base/home/quiz/result/admin/overlay
```
合计 TS 约 1.5-1.7 千行（现 865 行 JS，增量来自类型、守卫与拆分），CSS 不变 344 行。

**移植顺序**（每步都可运行并对旧版对照）：
1. 脚手架：Vite + tsconfig + dev 代理；原样搬 `index.html`、CSS、public 资产；截图对比应与旧版像素一致。
2. `types/` + `api/`（先手写，OpenAPI 就绪后换成生成类型）。
3. `utils/`（`h`、`fmtTime`、`shuffle`、`storage`）。
4. `domain/grading.ts` + Vitest 黄金用例（对旧 JS 同输入同输出，包含 §6.1 的舍入）。
5. `state/`。
6. `components/`（toast、dialog、brand、historyPanel、previewView、answerCard、questionView、reviewItem）。
7. `pages/`：home（只读）→ admin → quiz → result。
8. `main.ts` 接线、删除死代码（`sigOf`）、加 `submitting` 守卫。
9. Playwright 端到端：同一套用例分别对旧 Python 应用和新栈运行。

## 17. UI 回归测试必须覆盖的用户可见流程
W1 空题库首页（"题库还是空的" + "去上传题库"）；W2 加载失败（服务不可达、403 文本、非 JSON 登录页）→ fatal 文案；W3 上传 .xlsx → 预览（6 个数字块、各工作表行、"读不出来"折叠及 ≤30 条 + "还有 N 条类似的"）→ 确认 → 成功框 → "去考试"；W4 预览后"取消，不导入"与"返回首页"（后者静默 cancel）；W5 预览 `removed>0` 的二次确认框（含/不含 `removed_stats` 两种文案、"回去检查"）；W6 预览"没有可导入的题 / 无需导入"按钮禁用态；W7 上传错误：客户端扩展名、旧 .xls/损坏/XML 实体（服务端中文消息）、413、非 JSON 响应、网络错误；W8 拖放上传与 busy 状态；W9 过期/重复确认 409 文案；W10 首页配置：工作表勾选、模式（错题本无错题时禁用并回退 all）、题数钳制、打乱、池大小文案、"开始考试"；W11 答题：单选/多选/热点、再点取消、答题卡四种状态、标记、上下题、键盘（←/→/M/A-H）、日/中切换（无 zh 时禁用且 title）、`lang` 属性与字体、计时、进度条、`document.title`；W12 暂存退出→首页"继续作答/放弃这场"（含确认框）、刷新后恢复、被删题目的 toast；W13 交卷：未答完与全答完两种对话框、取消回到答题、成绩页（百分比、对错条、过滤 wrong/all、三种题型的解析、解析/口诀/备注/中文翻译折叠/累计统计）、"只重做错题/再考一次/回首页"、POST 失败 toast（并且仍能看结果）；W14 学习记录：最近一场、最高、柱状条、最近 6 条、清空（确认框 + toast + 失败 toast）；W15 错题本闭环：答错→计入"错题 N"→weak 模式→连续答对 2 次移出；W16 关闭程序（仅 `canShutdown`）→"程序已关闭"；W17 页脚版本号来自 `/api/info`；W18 响应式：≤880、≤480、`pointer:coarse`、`hover:none`（隐藏键盘提示）、iPhone 安全区与底部固定翻页栏；W19 localStorage 不可用（隐私模式）不崩溃；W20 经 HTTPS 域名/反向代理访问时全流程（Host/Origin 行为，沿用 `host_test.py` 55 项 + `tunnel_sim.py` 的场景）；W21 交卷在途中的重复点击不抛错（现状缺陷，修复后加回归）；W22 上传进行中点"返回首页"后服务端不残留可确认的 pending（现状缺陷 §10-8，修复后加回归）；W23 `<dialog>` 不可用时的降级（仅在把 iOS 下限定为 15.0 时需要）。

## 18. 车主两个私人工作簿的统计（只读，内存解析，仅计数）
- 工作簿#1：47266 B；2 个工作表；可用 25 题、读不出来 25 条；题型 single 12 / multi 5 / hotspot 8；25 题都有可切换的中文（`zh` 非空）；questions JSON 约 79 KB。
- 工作簿#2：54850 B；2 个工作表；可用 50 题、读不出来 0；single 20 / multi 22 / hotspot 8；50 题都有 `zh`；约 159 KB。
- 最大题干 293 字符；选项最多 5；热点小项最多 6。→ UI 与答题卡网格（5/8 列）按此尺寸即可；`zh=null` 路径和"读不出来"路径（真实数据里 #1 就有）要用**合成夹具**覆盖，不能用私人文件。

## 19. 回滚（前端侧）
保持 Python + `web/` 作为可恢复的 legacy（分支或 `legacy/` 目录）；前端新旧共用同一 `/api` 契约，只要后端兼容，可以只回滚后端或只回滚前端；localStorage 键名保持 `xexam.session.v1` / `xexam.lang.v1`，源不变则暂存不受影响。

## 20. 自我披露（过程中发生的、需要车主知道的事）
- **读取了 progress.json**：我第一次用仓库里的 `server.py --data <临时目录> --import <demo.xlsx>` 导入演示库时，`open_store()`（server.py:381-387）会**自动**去读 `APP_DIR/progress.json` 并迁移进临时库，输出一行"迁移了 3 场成绩、25 道题的答题记录"。我没有打印、保存或使用其内容；随即删除了那个临时库，改用仓库文件的**副本**目录（`.../frontend/appcopy`，不含 progress.json）重新建库和起服务。仓库没有被修改（设置了 `PYTHONDONTWRITEBYTECODE=1`）。**这是对 Phase 1 的重要提示**：从仓库目录运行原版，即使指定了临时 `--data`，也会把车主的 progress.json 迁移进测试库，测试必须在不含该文件的代码副本里跑。
- 车主两个私人工作簿只用副本解析器在内存里统计数量，没有输出任何题目文字。
- 启动/停止：我用到的端口与进程：8720（`--allow-shutdown`，已通过 `/api/shutdown` 停止）、8721（`capture.py`，脚本结束时终止）、8722（忘了带 `--allow-shutdown`，`/api/shutdown` 返回 403，随后**只**用 PID 精确结束了这一个 python 进程）、8723（`probe5.py`，脚本结束时终止）；Chrome 用过 DevTools 9640-9645，均已终止；末尾复核时 8723 无响应、没有属于我的 python/chrome 进程；未安装任何东西；写入仅限 `xexam_phase0/frontend/`。
- **一次操作失误**：清理 8722 时，我先用了一个过宽的进程过滤器（命令行包含我的目录），它匹配到并结束了 3 个 `bash.exe` 包装进程（我自己各条 Bash 命令的外壳，不是别的审计维度的进程；python 服务不在其中）。随后改用端口+PID 精确定位，只结束了 PID 33040。其他维度的审计进程（`xexam_audit/...` 等）未被触碰。
- 仓库状态：全程 `git status` 前后一致（均为 v0.2 未提交改动：.dockerignore、docker.yml、.gitignore、Dockerfile、README、VERSION、两个 compose、server.py、web/app.js、使用说明.txt 已修改，`scripts/` 未跟踪）；没有执行任何写 git 的命令；用 `PYTHONDONTWRITEBYTECODE=1` 防止在仓库里生成 `__pycache__`（仓库里原有的 `__pycache__` 是 05:22 的，不是我生成的）。
- 证据文件：`evidence/api-samples.json`（真实接口返回样例 + 12 种错误响应，token 已打码）、`evidence/golden-legacy-functions.json`（旧前端纯函数在真实页面里的黄金输出：`grade` 17 例、`stateOf` 9、`fmtTime` 15、`inNotebook` 9、`pool` 5、`syncSelection` 4、`loadSaved` 27、百分比 9）；生成脚本 `capture.py` + `golden_probe.js`。黄金输出只用合成数据和虚构演示题库。
