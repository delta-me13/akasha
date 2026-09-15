# ADR-0007：Bitwarden CLI 的获取、变体与状态归属

- **状态**：**实现中**（Implementing，2026-09-15）
- **日期**：2026-09-15
- **决策者**：cyrene
- **影响范围**：`scope.md` §7 与 §10（"用户自备前置"这条已定案口径、以及"不打包 `bw`"这条非目标的**后半句**被取代）、
  数据目录里多一棵 `bitwarden/` 子树、新的 crate `akasha-bw`、前端多一个应用级面板、配置文件的字段增加
- **关联**：[plan 0902](../plans/0902-bw-acquire-and-preflight.md)（获取与前置检查）、
  [plan 0905](../plans/0905-bw-login-session-ui.md)（登录会话与界面）、
  [`../bitwarden.md`](../bitwarden.md)（上游条款、命令面与实测）

---

## 1. 背景

`scope.md` §7 在 2026-09-11 定案的口径是：**`bw` 作为用户自备的系统前置，不打包、不下载**。
理由是许可证 —— 专有变体禁止分发（2.3(i)），也禁止生产环境使用（2.1）。代价如实写在那里：
未安装 `bw` 的机器上该功能**整体不可用**。

2026-09-15 的当次指令改口径（用户指令优先于本仓库既有约定）：

1. CLI 走**运行时下载**；
2. 前端要能完成**登录，含自托管服务器**；
3. `lock` / `unlock` 状态与登录状态**同步**；
4. session token **只在内存中**。

这条改动够格写 ADR：它决定"上游二进制的哪一份、从哪来、装在哪、CLI 自己的状态放哪"
（用户数据与许可证两条线都不可轻改），并且它是**对一份已定案决定的取代** ——
后人一定会问"为什么当初定的是用户自备，现在改成下载"。

## 2. 事实依据（本机实测 + 官方文档，2026-09-15）

官方文档是**用 `bw` 的主依据**：<https://bitwarden.com/help/cli/>（它同时是 `bw --help`
的镜像，页面上写明"Most information you'll need can be accessed using `--help`"）。

| 项 | 事实 | 来源 |
|---|---|---|
| 命令面 | `login [email] [password] --method <m> --code <c>` / `login --apikey` / `login --sso`；`unlock [password]`（另有 `--passwordenv` / `--passwordfile`）；`lock`；`logout`；`status`；`sync`；`config server [url]` | 官方文档 |
| 全局选项 | `--raw`（只输出裸值）、`--nointeraction`（禁止交互提问）、`--session <key>`、`--quiet` | 官方文档 + `bw --help` 实测 |
| 两个变体 | 每个 bundle 都有 OSS（`bw-oss-*`）与非 OSS（`bw-*`）两版；**非 OSS 是各分发平台的默认包**，多出 device approval 一类非 OSS 许可的功能 | 官方文档原文 |
| 变体的可执行判据 | 两版的 `bw --version` **都**输出 `2026.8.0`；`bw --help` 的命令表里**只有非 OSS** 那一份有 `device-approval` 行 | 实测（两份 `cli-v2026.8.0` 资产各跑一次） |
| 下载落点 | GitHub release `cli-v<版本>` 下的 `bw-oss-<os>[-<arch>]-<版本>.zip`；zip 内是单个可执行文件 `bw` | 实测（release 资产表 + 解包） |
| 校验文件 | `bw-*-sha256-<版本>.txt` 在 `cli-v2024.12.0` / `cli-v2025.1.0` 存在，在 `cli-v2025.6.0` 与 `cli-v2026.8.0` **不存在** | 实测（release 资产表逐个核对） |
| `bw status --raw` | 未登录 = `{"serverUrl":null,"lastSync":null,"status":"unauthenticated"}`；文档给出的完整形状多 `userEmail` / `userId` 两个键；`status` 三取值 `unlocked` / `locked` / `unauthenticated` | 实测 + 官方文档 |
| 错误形态 | 未登录时 `bw list items` 与 `bw unlock` 输出 `You are not logged in.`，**退出码 1**；`bw status` 恒退出码 0 | 实测 |
| 非交互传主密码 | `--passwordenv <VAR>` 从子进程环境取；`--passwordfile <path>` 从文件第一行取；也可以把口令写成位置参数 | 官方文档 + `bw unlock --help` 实测 |
| 明文 HTTP | `bw login` 对 `http://` 服务器报 `InsecureUrlNotAllowedError: Insecure URL not allowed. All URLs must use HTTPS.` | 实测 |
| 自签证书 | 文档：设 `NODE_EXTRA_CA_CERTS` 指向 PEM。实测：指向自签证书后，`bw login` 真的对本地桩发出了 `GET /api/config` 与 `POST /identity/accounts/prelogin/password` | 官方文档 + 实测 |
| `bw config server` | 无参数 = 读回当前服务器（输出**不带换行**）；带值 = 写入；文档注明后续任何一次 `config` 调用会覆盖此前全部取值 | 实测 + 官方文档 |

## 3. 决策

**D1 —— 只获取 OSS 变体（`bw-oss-*`）。**
GPL-3.0-only 对使用者没有用途限制，专有变体的 2.1（仅限内部开发与测试、非生产）与
2.3(i)（禁止分发）两条都不适用。代价是没有 device approval 一类企业管理命令 ——
本项目的范围（登录 / 解锁 / 只读导入）一条都不需要它们。

**D2 —— 版本在运行时解析**：取 GitHub 上最新的 `cli-v*` release（跳过 prerelease 与草稿）。
不钉版本，是为了让上游的安全修复能进来；换来的代价是"行为随上游变"，因此下载之后
**把实际版本读出来并显示**（`bw --version`），而不是假装它是我们选的那一个。

**D3 —— 完整性只到 HTTPS，如实说明它到哪。**
下载走 HTTPS、来源是上游自己的 release 资产（版本号参与 URL）。⚠️ 上游自 `cli-v2025.6.0`
起不再发布 SHA-256 文件（§2 实测），所以**不得**声称"已按上游校验"。做的是：
把下载物的 SHA-256 算出来并记进日志与界面（TOFU，供事后核对），并且**不复用**上一次的哈希
去拦这一次的下载 —— 那会把一次上游更新变成一次"校验失败"。

**D4 —— 两个轴各自独立，默认都取 `host`。**

| 轴 | 取值 | 含义 |
|---|---|---|
| 二进制来源 | `host`（默认） | `PATH` 里找到的 `bw` |
|  | `managed` | 运行时下载到数据目录里的那一份 |
| 状态目录 | `host`（默认） | CLI 自己的默认目录（`BITWARDENCLI_APPDATA_DIR` 不设） |
|  | `managed` | 数据目录下的 `bitwarden/appdata/` |

默认取 `host` 是用户当次指令（CLI 的状态里含 access token，让它在系统默认位置不动，
比随我们的数据目录迁移风险低）。两个轴**不得合并成一个三档开关** —— "用哪份 CLI"与
"状态放哪"回答的是两个问题，合并之后就没有"宿主机器的 CLI 配隔离状态"这一档。

**D5 —— `host` 那一轴上找不到 `bw` 时不静默切换。** 界面显示"这台机器上没有 `bw`"并给出
一个显式的"改用运行时下载"动作；切换只由用户动作触发（`host` 那一轴解析不到，与
`managed` 那一轴还没下载，是两件不同的事，报错要说清是哪一件）。若 `PATH` 里那一份是
**专有变体**（§2 的判据），界面必须带一句许可证提示 —— 这是 `scope.md` §7 "不得让用户
在生产环境中使用专有变体而无任何提示"的落点。

**D6 —— `managed` 的落点。**
二进制：`<数据目录>/bitwarden/bw-<版本>/<可执行文件名>`（版本进路径 = 换版本不动旧目录，
解包与替换因此可以做到"先写新目录、再改指认"）。
状态目录：`<数据目录>/bitwarden/appdata/`（**不带版本** —— 它是用户数据，换 CLI 版本不该丢登录）。
数据目录的口径沿用 [`config.rs`](../../src-tauri/src/config.rs) 的 [`data_dir_of`]：便携目录优先。

**D7 —— session token 只在内存，且住在受保护页里。**
`bw unlock` / `bw login --raw` 交出的 session key 放进 `akasha_store::protected::Protected`
（ADR-0002 D13 的同一个原语，`akasha-ssh` 的内存凭据缓存是同一个先例），**不写盘、不进日志、
不进事件载荷**。`bw lock` / `bw logout` / 进程退出即抹掉。它与库口令是两种机密：
前者要能原样交给 `bw` 子进程，所以按 `Exposed` 提权窗口取用，用完即关。

**D8 —— 主密码经 `--passwordenv` 传给子进程。**
不用位置参数（argv 对同机进程可见，`ps` 就能读到），不用 `--passwordfile`（会把主密码落盘），
也不走交互提问（inquirer 的逐字符回显会把口令写进管道，且 2FA 的往返不可控）。
⚠️ 残余暴露面如实记下：`bw` 存活期间，同用户进程可读 `/proc/<pid>/environ`；
这与 `russh` 需要一份普通 `String` 口令是同一类无法消除的副本（ADR-0003 D8 已如实记录一次）。

**D9 —— 自托管先设服务器再登录。**
`bw config server <url>` 成功后才是 `login`。**只接受 https**：`bw` 自己会拒绝明文 HTTP
（§2 实测），但那次拒绝发生在 `login` 时、且错误里的 URL 是它拼出来的 `/api` 地址 ——
所以在设置服务器这一步就先给出可读报错。默认服务器（官方云）**不写** `config`：
`bw` 自己的默认值就是它，写一次反而多一份可能与上游默认漂移的状态。

**D10 —— 登录 / 锁定状态的唯一真相是 `bw status --raw`。**
界面上的 `unauthenticated` / `locked` / `unlocked` 三个取值来自它，不由我们的内存状态推断。
反向的同步只有一条：`status` 说不是 `unlocked` 时，**我们持有的 token 立即丢弃**
（外部用 `bw lock` 锁过之后，我们手里的旧 key 已经失效，留着它只会让下一条命令以
`You are not logged in.` 失败）。每次命令前后各读一次 `status`，界面据此刷新。

## 4. 被否掉的替代方案

| 方案 | 为什么不选 |
|---|---|
| 打包 `bw-oss`（GPL-3.0-only） | 合法，但把 GPL 义务带进本项目分发（许可证文本 + 源码获取途径），还要自己承担下载、校验与更新；而运行时下载把这三件事都留在上游 |
| 打包或下载**专有**变体 | 2.3(i) 禁分发、2.1 把用途限制在内部开发与内部测试且非生产环境 —— 上一版 `scope.md` §7 的结论原样成立 |
| 钉死版本 + 源码里写期望哈希 | §2 实测：上游自 `cli-v2025.6.0` 起不发布校验文件，钉住的哈希只能来自我们自己的首次下载（TOFU），钉版本换不来"官方校验"；而它会让上游的安全修复进不来 |
| 用 `bw serve` 的 REST 接口代替逐次调用 CLI | 多一个常驻进程、多一个本地 HTTP 端口与一套鉴权（文档写明默认阻止带 `Origin` 的请求），而本阶段只需要几条一次性命令 |
| 主密码走 argv 或 `--passwordfile` | 前者对同机进程可见，后者把主密码落盘 —— 两条都与 ADR-0002 D13 的口径相反 |
| 自己实现 vault 密码学 | 非目标（`scope.md` §10）：错了即灾难，由 Bitwarden 负责 |
| 换 `rbw` 或官方 Rust SDK | 已核实并否决过，见 [`../bitwarden.md`](../bitwarden.md) §3 |
| 状态目录只有"隔离"一档 | 用户当次指令要求两轴可切、默认 host（CLI 状态随我们的数据目录迁移的风险高于收益） |

## 5. 风险与边界

- **许可证**：本项目**不分发**二进制 —— 下载发生在用户机器上、来源是上游自己的 release。
  这是"不打包"结论能继续成立的原因；若将来改成随产物分发，D1 与 `scope.md` §10 必须重开。
- **`host` 轴上可能是专有变体**：提示由界面承担（D5），不给"我确定"的旁路。
- **状态目录取 `host` 时，状态不在便携目录里**：整体迁移之后，登录态**不会**跟着走
  （需要重新登录）。这一条与 `docs/portable.md` 的"搬走文件夹数据仍在"不冲突 ——
  它说的是本项目自己的数据四类池与库；CLI 的 access token 是上游的状态。
- **主密码的内存副本**：本进程内它在受保护页里；交出去的那一份在子进程环境里（D8）。
- **`bw status --raw` 的完整形状只在文档里见过**：`userEmail` / `userId` 两个键在
  **未登录**时不存在（实测），登录之后的形状**本机没有实测过**（需要一个真实 vault）。
  解析因此必须把这两个键当可选。
- **变体判据是"读 `--help` 的命令表"**：上游改命令表就会失效。判据写成"有没有
  `device-approval` 这一行"，并在它读不出来时**报"判不出变体"**而不是默认成 OSS。

## 6. 修订记录

- 2026-09-15 初稿，状态「实现中」。取代 `scope.md` §7 原"用户自备系统前置"的口径与
  §10 里"运行时下载官方二进制"那条非目标的前半句（"不打包"仍然成立）。
