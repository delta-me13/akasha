# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-12

## 一句话

**阶段 2「端到端最小终端」5/5 完成**；**阶段 3「托盘与应用生命周期」6/6 完成**。
**阶段 4「存储与凭据池」在做（7/9）**：ADR-0002（机密存储与可搬迁）**已进入「实现中」**。
已落地七块：**SQLCipher 加密库能开**（plan 0401）、**口令只从一条路进来，而且能真的验证它**
（0402 —— 拆开了"打开"与"新建"，原来在没有文件的路径上**任何口令都能开**）、
**口令在内存里也受保护**（0406）、**四套池能增删改查**（0403 —— v1 的四张表、不变量写在库自己身上、
私钥读出来进受保护页）、**库里的东西拿得出去也放得回来**（0404 —— 加密导出可在另一目录还原；
明文导出有两道**写进实现**的门槛；dump 结构上不含私钥）、
**解锁与锁定是一条完整的生命周期**（0407 —— 谁持有解好的库、口令从哪来、什么时候锁、锁定时抹掉什么；
真 app 上实测 `VmLck` **0 → 192 → 0 kB**）。

**下一步是 plan 0405（可搬迁性验证）**：`mv` 整个文件夹 → 启动 → 断言"原有主机/密钥/规则都在"
（只验证"能开"不算过），外加 `portable.md` §4 第 3 条（便携目录不可写要**明确报错**）。
它现在有真路径可用了：`vault_unlock` 能解库并读出四套池各有多少行 —— 那正是"搬家后内容还在"
要的那一步。搬完这一项，**ADR-0002 就可以转「已定案」**（条件：plan 0401–0405 全部完成）。

**点叉的语义由配置 × 托盘共同决定**（plan 0302 + 0303）：

| `close_behavior` | 托盘建成了吗 | 点叉之后 |
|---|---|---|
| `tray`（默认） | 是 | **窗口隐藏、进程留着** —— 会话与终端缓冲原样存活 |
| `tray` | **否** | 退出（**降级**：藏起来就再也叫不回来，坑 #60） |
| `exit` | 任意 | 退出（走 0204 的收尾路径，零残留） |

配置文件 = **数据目录**里的 `config.json`（`docs/portable.md` §3.1）：bin 同目录存在
`akasha-data/` 就用它（便携模式），否则退回 OS 数据目录（Linux 上 =
`~/.local/share/fans.cyrene.akasha-terminal/`）。**只读、不自动创建**；读不到 / 值不认识 →
默认值 + 一条日志。⚠️ **只在启动时读** —— 改完要重启 app。

**单实例（plan 0304）**：第二个实例会把已有窗口**叫回来**（还原 → 显示 → 置前）然后自己退出
（实测 150–205 ms、退出码 0）。窗口**藏起来时也一样** —— 只 `set_focus()` 是叫不回一个隐藏
窗口的，这正是它必须接 0302 的地方。

| 平台 | 机制 | 依赖外部服务？ |
|---|---|---|
| Linux | D-Bus 会话总线上的名字 `<identifier>.SingleInstance` | **是** —— 没有会话总线就不注册（降级为"可以多开" + `warn`） |
| Windows | 命名 mutex + 一个 message-only 窗口 | 否 |
| macOS | `/tmp/<identifier>_si.sock` | 否 |

"何时回收"因此有七种触发（粒度从一个会话到整个进程）：

| 触发 | 谁被回收 |
|---|---|
| 关一个终端标签页 / 会话自己结束 | **只有那一个会话** |
| **关窗口（默认 = 收托盘）** | **不回收** —— 进程、会话、终端缓冲全留着（0302） |
| 关窗口（`close_behavior = exit`） | 全部（`RunEvent::Exit` → `Sessions::shutdown_all()`） |
| **从托盘菜单退出** | 全部（`shutdown_all` → `app.exit` → `RunEvent::Exit` 再收一次，幂等） |
| 正常退出 | 全部（同上，`trigger="exit"`） |
| panic | 全部（panic hook：打崩溃现场 → 回收 → `abort()`） |
| `tauri dev` 重载 / `kill -9` / `kill -TERM` | 全部 —— **另一个进程**：看门狗读到管道 EOF（ADR-0005） |

⚠️ **关标签页 ≠ 关窗口 ≠ 退出应用**：关掉**最后一个**标签页只是**空状态**（界面空了、进程留着）；
关窗口（默认）只是**隐藏**。只有**三大终端**（local / ssh / serial）的标签页有关闭按钮；
转发 / 密码库 / 文件传输是**仅渲染**的视图标签页（**无关闭按钮**）—— 见 `docs/scope.md` §5.6。

## ⚠️ UI 现状：**当前界面是功能验证壳层，不是设计稿**

**正式 UI 的布局 / 视觉 / 交互尚未有设计稿。** `src/**` 现有的界面（标签栏、状态栏、配色、
空状态文案）只有一个用途：让后端行为能被看见、能被验证。规则写在 `AGENTS.md` §4.0，
展开在 `docs/scope.md` §1.3。三句话：

- **不要**把当前界面当产品约束或"既有风格"，不要在它上面做视觉打磨；
  前端改动的判据是"**这条后端行为能不能被验证**"，不是"好不好看"。
- 后端**不得**依赖前端的呈现方式：界面整体重做时，命令 / 事件 / `Session` 状态机与收尾路径
  应当**原样可用**。
- 验证用的探针与选择器（`window.__akashaTerminal`、`.tab-pane.is-active …`）是**测试接口**，
  不是 UI 规范。**重做界面属于尚未规划的工作**（没有设计稿就没有验收标准，因此不进 ROADMAP）。

## 已验证为绿（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + gen-types-check + docs-check） | 退出码 **0**，**6/6 全绿** |
| `just test` | **180 tests run: 180 passed**（`akasha` 52 + `akasha-pty` 37 + `akasha-core` 15 + **`akasha-store` 76**） |
| ↑ 本轮新增 | **8 条**：`akasha-store` 4 条（plan 0407 的生命周期与内存扫描）+ `akasha` 4 条（`vault` 的映射与建/开判据单测） |
| ↑ **锁定之后进程内存里一处机密都不多**（plan 0407 判据） | ✅ 把**整个进程内存**（匿名段，含 `---p`）扫一遍找那两串字节：口令 `1 → 2 → 2 → 1` 处、派生密钥 `3 → 1` 处。**解锁期间多出来的那一处正好是 `---p` 受保护页**；丢掉连接之后派生密钥一处不剩 |
| ↑ ↑ **带正对照**（否则这条是永真式，坑 #87） | ✅ 两条扫描用例都要求"解锁期间必须比基线**多**扫到" —— 少了它，"锁上之后 0 命中"与"扫描器根本没在工作"是同一条绿。⚠️ 扫描器本身栽过两次（坑 #90/#91），都写在用例的文档里 |
| ↑ **`VmLck` 走一个来回** | ✅ 库层：`0 → 4（只有口令页）→ 152（解锁中）→ 4（丢掉连接）→ 0 kB`，`---p` 段数同时回落；重新 `open` 那条路也是 `4 → 172 → 4`。**真 app 上：`0 → 192 → 0 kB`**（测试进程自己读 `/proc/<pid>/status`，不是 app 自报） |
| ↑ **`cipher_memory_security` 的擦零是实测的**（不是"上游这么说"） | ✅ 派生密钥是用 `openssl` CLI 按 SQLCipher 4 的默认参数（PBKDF2-HMAC-SHA512 / 256000 / 16 字节盐）**独立算出来**的：连接活着时扫到 2 处（SQLCipher 的），drop 之后**一处不剩**。机制可逐行核对：`sqlcipher_mem_malloc/free` 每次分配 `mlock`、每次释放先擦零再 `munlock` |
| ↑ **推翻一处想当然：SQLCipher 不留口令本体** | ✅ 解锁期间口令**只**多出一处（受保护页）—— 它手里只有派生密钥。ADR-0002 §7.5 的副本清单因此少了一行 |
| ↑ **真路径：解锁 → 读一次池 → 锁定**（`just test-e2e` 新增 `vault_unlock`） | ✅ `vault_status` 报 `missing`/`unlocked:false` → 造库（每套池一行）→ `vault_unlock` 返回 `{"keys":1,"hosts":1,"serials":1,"forwards":1}` → `VmLck` 涨到 192 → `vault_lock` → **回落到 0** → 错误口令被拒**且仍然锁着** → 锁上之后还能再解开 |
| ↑ 建 / 开的选择 | ✅ `Missing`/`Empty` → **建**，`Present` → **开**（单测钉住三种状态各走哪条路；存储层那两条路仍然是分开的，没有"打不开就建"的兜底） |
| ↑ `mlock` 失败变成**用户能懂的一句话** | ✅ `内存锁不住（mlock 失败），出于安全拒绝解锁：<操作系统的原话>` —— 不把上游那层 `Memory error:` 包装丢给用户（单测用真的 `StoreError::MemoryProtection` 钉住） |
| ↑ 本轮新增 | **8 条**：`akasha-store` 4 条（plan 0407 的生命周期与内存扫描）+ `akasha` 4 条（`vault` 的映射与建/开判据单测） |
| ↑ **锁定之后进程内存里一处机密都不多**（plan 0407 判据） | ✅ 把**整个进程内存**（匿名段，含 `---p`）扫一遍找那两串字节：口令 `1 → 2 → 2 → 1` 处、派生密钥 `3 → 1` 处。**解锁期间多出来的那一处正好是 `---p` 受保护页**；丢掉连接之后派生密钥一处不剩 |
| ↑ ↑ **带正对照**（否则这条是永真式，坑 #87） | ✅ 两条扫描用例都要求"解锁期间必须比基线**多**扫到" —— 少了它，"锁上之后 0 命中"与"扫描器根本没在工作"是同一条绿。⚠️ 扫描器本身栽过两次（坑 #90/#91），都写在用例的文档里 |
| ↑ **`VmLck` 走一个来回** | ✅ 库层：`0 → 4（只有口令页）→ 152（解锁中）→ 4（丢掉连接）→ 0 kB`，`---p` 段数同时回落；重新 `open` 那条路也是 `4 → 172 → 4`。**真 app 上：`0 → 192 → 0 kB`**（测试进程自己读 `/proc/<pid>/status`，不是 app 自报） |
| ↑ **`cipher_memory_security` 的擦零是实测的**（不是"上游这么说"） | ✅ 派生密钥是用 `openssl` CLI 按 SQLCipher 4 的默认参数（PBKDF2-HMAC-SHA512 / 256000 / 16 字节盐）**独立算出来**的：连接活着时扫到 2 处（SQLCipher 的），drop 之后**一处不剩**。机制可逐行核对：`sqlcipher_mem_malloc/free` 每次分配 `mlock`、每次释放先擦零再 `munlock` |
| ↑ **推翻一处想当然：SQLCipher 不留口令本体** | ✅ 解锁期间口令**只**多出一处（受保护页）—— 它手里只有派生密钥。ADR-0002 §7.5 的副本清单因此少了一行 |
| ↑ **真路径：解锁 → 读一次池 → 锁定**（`just test-e2e` 新增 `vault_unlock`） | ✅ `vault_status` 报 `missing`/`unlocked:false` → 造库（每套池一行）→ `vault_unlock` 返回 `{"keys":1,"hosts":1,"serials":1,"forwards":1}` → `VmLck` 涨到 192 → `vault_lock` → **回落到 0** → 错误口令被拒**且仍然锁着** → 锁上之后还能再解开 |
| ↑ 建 / 开的选择 | ✅ `Missing`/`Empty` → **建**，`Present` → **开**（单测钉住三种状态各走哪条路；存储层那两条路仍然是分开的，没有"打不开就建"的兜底） |
| ↑ `mlock` 失败变成**用户能懂的一句话** | ✅ `内存锁不住（mlock 失败），出于安全拒绝解锁：<操作系统的原话>` —— 不把上游那层 `Memory error:` 包装丢给用户（单测用真的 `StoreError::MemoryProtection` 钉住） |
| ↑ 导出与还原（plan 0404，上一轮） | 加密导出可在另一目录还原（逐字段一致）；明文导出两道门槛（逐字短语 7 个近似输入 / 文件名自曝 / 独立口令）；不泄密带对照组（**0 命中 vs 1 命中**）；失败不留半成品（负例验过）；导出件 **600**。细节在 [archive/0404](./plans/archive/0404-dump-export.md) |
| `just test-e2e`（自包含：起 Vite + app → **两段** → 收尾） | 退出码 **0**，**15 个用例通过**（其中 `window_close` 内部**显式跳过**：这台机器 `tray_ready=false` → 关窗语义降级为退出，它只验"隐藏"那条路并打印了判据）：`smoke` 3 / `integration` 2 / `session_channel` 1 / `terminal_render` 2 / `tab_close` 1 / `single_instance` **2** / `vault_status` 1 / **`vault_unlock` 1** / `exit_residue` 1 |
| ↑ **`vault_status` 真路径（plan 0403，未退化）** | ✅ `{"path":"…/src-tauri/target/debug/akasha-data/akasha.db","state":"missing"}`；父目录名是 `akasha-data` 且它是 **bin 同目录**（P2 的落点判据在真路径上） |
| ↑ **§7 的 registry 那一条：当前不可满足**（实测，见坑 #82） | `get_registry` 回 **`[]`** —— 本仓库的命令**都没有 `#[inspectable]`**；`detect_ghost_commands` 的 `reliability` 是 **low**。**替代证据**是真路径上的 `invoke_command` 成功。用 REST 兜底问到的（沙箱里 MCP 连不到另一个 bash 命名空间里的 app，坑 #33） |
| ↑ 单实例（0304，Linux 实测，本轮复跑） | ✅ probe `{"registered":true}`；`window manage hide` → 再起同一个二进制 → **204.8 ms** 后 `exit=0`、`activations=1`、`visible=true` |
| ↑ 关闭行为可配置（0303，Linux 实测） | ✅ 三种情形都验过：文件不存在 → 默认 tray；`{"close_behavior":"exit"}` → 关窗即退出且 `sessions reclaimed`；`"nope"` → `config invalid` + 回默认，app 照常启动 |
| ↑ 关标签页 / 敲 `exit`（0305 / 0306，真 UI 点击） | ✅ 关标签页 → 探针 A 在 **79 ms** 内消失、探针 B 不受影响；在终端里敲 `exit` → 标签页自己关掉（app 仍在） |
| ↑ 退出零残留（0204/0205，未退化） | ✅ `app 已退出（pid 2495）`；`✅ 零残留：忽略 SIGHUP 的 3251 已随会话被收掉` |
| ↑ 终端 / 会话判据（未退化） | `renderer = webgl`；8 MB 灌流后仍可交互；raw 通道 10.73 MB / 164 批；收尾帧 1 个、console 零异常 |
| `pnpm build`（tsc + vite build） | 退出码 0；产物 **843 kB / gzip 231 kB**（本轮未改前端，数字沿用） |
| `just deny-offline` | `bans ok, licenses ok, sources ok`（`--workspace` 已加，补上了"未被人依赖的成员不在图里"那个盲区） |
| `just docs-check` | 全过（ROADMAP 条目在 3 行内且无代码块 / plan ≤200 行且索引一致） |
| `ast-grep scan` | 退出码 **0**；**六条**规则均已用正负例验证（本轮未新增规则） |
| `cargo tree -p akasha-core` \| `grep -c tauri` | **0**（分层成立） |
| `just bench`（criterion） | 52.7 GiB/s / 14.1 ns 每批 / 9.64 GiB/s（**0201 的数字，本轮未复跑**） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。

## 待验证（本地跑不了 / 沙箱跑不了）

- **解锁 / 锁定还没有界面**：命令、状态、生命周期都在了（plan 0407），但界面上**没有能输口令的地方**
  （设计稿未定，`AGENTS.md` §4.0）。今天的真路径证据来自 E2E 的 `invoke_command`；
  "用户点得到的那条路"要等 UI。
- **口令经 IPC 的两份副本够不着**：tauri 的请求体缓冲与 `serde_json::Value` 在这一次调用之后被
  free 而**不擦**（ADR-0002 §7.5 的边界）。我们这一份（`PassphraseInput` → `Passphrase::new`）
  被擦零。更紧的做法是 raw body，代价是前端手写裸 `invoke` —— 记为后续，不在 0407 里做。
- **`mlock` 失败那条路只有单测**：本机 `RLIMIT_MEMLOCK` 是 8 MB，造不出真的失败；
  用户文案是用一个**真的** `StoreError::MemoryProtection` 钉住的，但没有"把 `mlock` 弄失败"的端到端证据。
- **内存扫描只在 Linux、只扫匿名段**：`/proc/self/maps|mem` 的读走 `FOLL_FORCE`（Linux 专有）；
  文件支撑的段不扫（机密不可能落在只读的库文件映射里）。派生密钥那条用例在 `openssl` CLI
  缺席时**显式跳过并打印原因**，不静默通过。
- **导出与还原还没有 IPC 命令**：库层的函数与契约测试都在（plan 0404），
  但"导出成哪个文件、确认短语怎么问"要等文件选择器与确认界面（设计稿未定）。
- **明文导出的确认界面不存在**：门槛在库层（逐字短语 + 文件名自曝），没有 UI 能点。
  接 UI 时要证明的是"**界面上没有一条路能在没有短语的情况下导出明文**"。
- **权限位（0600）只在 unix 上有判据**：Windows 没有等价物（D12 如实记为不做）。
- **导出件在跨机器 / 跨文件系统上的表现未验**：`rename` 的原子性只在同一文件系统内成立
  （本步已把 `…partial` 放在**目标同目录**），而"从 Linux 导、到 Windows 还原"这条路没跑过。
- **单实例在 Windows / macOS 上未验**：本机只有 Linux。机制完全不同（命名 mutex / `/tmp` 下的
  unix socket），CI 的**类型检查挡不住运行期差异**。
- **CI 的 Linux E2E 上几条必然跳过**：`single_instance` 与 `window_close` 都要会话总线 + 可写
  `$XDG_RUNTIME_DIR`，xvfb 两样都没有（用例**显式跳过并打印判据**）。
- **托盘图标在面板里"看得见"**：机器只能证明"注册进了 watcher"（`scope.md` §5.5）。
- **CI 三个 job 是否真能变绿** —— 仓库还没有 remote，从没跑过。
- **`tauri dev` 重载那条路径没有门禁**：只能手动实测（plan 0205 的实施记录里有脚本与输出）。
- **前端类型检查不在任何门禁里**：`just ready` 只覆盖 Rust + 文档，`pnpm build`（tsc）要手动跑。
- **在"有后台作业握着 PTY"的标签页里敲 `exit`**：不会有 EOF、不会关标签页（刻意，坑 #55），
  但没有用例守着它。
- **宿主 MCP 连不到沙箱内运行的 app**（私有 PID / 临时目录；坑 #33）。
- **`just dev-web` 的模拟后端没在真浏览器里点过**（本环境没有浏览器）。
- **大流量下的 JS heap 数字没取**。

## 当前基线（2026-09-12 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` / **`akasha-store`** |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，坑 #29） |
| 后端模块 | `bindings` / `session` / `tray` / `config`（载体 + **数据目录**）/ `lifecycle` / `single_instance` / **`vault`（库的落点、状态与解锁生命周期）** / `watchdog` |
| **关窗语义** | 判据 = `akasha-core::CloseAction::decide(close_behavior, tray_ready)`；app 侧 `CloseRequested` → **先 `hide()`、成功才 `prevent_close()`**。**不挂 `RunEvent::ExitRequested`**（理由见 `lib.rs` 注释与坑 #65） |
| **单实例** | 插件注册在**第一个插件位**；唤起 = `unminimize()` → `show()` → `set_focus()` **三步无条件都做**；`available()` 在 Linux 上 = 会话总线连得上 |
| **配置** | `<数据目录>/config.json`，`{"close_behavior":"tray"\|"exit"}`；`serde_json` + `deny_unknown_fields`；**只读不写**；在 `.setup()` 里读一次 |
| **数据目录** | bin 同目录存在 `akasha-data/` → 用它（便携）；否则 `app_data_dir()`。**不自动创建**。开发构建里便携目录 = `src-tauri/target/debug/akasha-data` |
| **系统托盘** | 图标 = `bundle.icon` 那张；Linux 上落盘到 `$XDG_RUNTIME_DIR/tray-icon/…`；菜单 id 是稳定字面量（`window.toggle` / `tunnels` / `tunnels.empty` / `app.quit`）；**会话表一变整份重建** |
| **probe** | `lifecycle` → `{"close_behavior":…,"tray_ready":…,"close_action":…}`；`single_instance` → `{"registered":bool,"activations":n}`。**库没有 probe**：状态本身就是命令（`vault_status`），再加一个只会多一条观察路径、不多一点信息。⚠️ 也**不靠探针自报**来验"锁定时内存还回去了"：那条判据由测试进程自己去读 `/proc/<pid>/status` 的 `VmLck` |
| 出字节路径 | PTY read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write` |
| 前端结构与布局 | `src/tabs/TabStrip.tsx` + `src/App.tsx`（多面**同时挂载**、非活动的 `visibility: hidden` 叠放）+ `src/terminal/` + `src/ipc/` |
| **关闭一个标签页** | 移除 → React 卸载该面 → `attachTerminal` 清理（**先** `close_session`，**后** `surface.dispose()`）→ `Sessions::close` → `Transport::shutdown()` → 撤销看门狗登记。**实测 79–93 ms** |
| **会话自己结束** | 合批读循环结束（EOF / EIO）→ 收工 → 收尾线程 `Sessions::retire`（收尸 + 摘牌 + `registry.close` + `forget`）→ `app.emit("session_ended", …)` → 前端关掉那个标签页 |
| 回收路径（进程内 / 进程外） | `RunEvent::Exit` / panic hook / `close_session` / `retire` / 托盘退出；另有一个**进程外**看门狗读管道（`+pid` / `-pid`，EOF = app 死了） |
| 前端渲染器 | **WebGL**（WebKitGTK + MESA 软件栈下仍拿到 WebGL2）；`canvas` 元素 2 块 |
| 大输出实测 | 10.36–11.28 MB / 159–170 批（0202–0305 各轮） |
| 前端产物 | 843 kB（gzip 231 kB） |
| **存储层** | `akasha-store`：**全仓库唯一允许出现 `unsafe` 的 crate**（送口令进 `sqlite3_key()`，ADR-0002 D4），由 workspace 的 `unsafe_code = "deny"` + `.ast-grep/rules/no-unsafe-outside-store.yml` 两层守。**两条路**：`create(path, &Passphrase)` = 有内容就 `VaultExists`（永不覆盖）→ 送密钥 → **一次事务里建 v1 的四张表 + 写 `user_version = 1`**；`open(path, &Passphrase)` = 不存在的/0 字节就 `NoVault` → 送密钥 → 开外键 → 读一次 `sqlite_master` 逼口令错暴露 → **校验版本号 + 四张表都在**（缺表 → `MissingTable`）。两条路都收 0600。**app 现在依赖它**（也补上了 cargo-deny 的图根盲区） |
| **四套池** | `keys` / `hosts` / `serials` / `forwards`，各 5 个函数 + 反查（`hosts_using_key` / `hosts_jumping_to` / `forwards_of_host`）。`New*`（没有 id）与 `*`（有 id）**是两种类型**；不变量写在库上（`STRICT` + `CHECK` + 外键 `RESTRICT`，ADR-0002 D14），应用层只把 sqlite 的失败翻成 `Conflict`。⚠️ 跳板链的成环**库表达不了**：`update` 时逐跳走链挡住（`MAX_JUMP_DEPTH` = 32 是防死循环的兜底） |
| **私钥** | 池里存 BLOB，**出库直接进受保护页**（`PrivateKey` = `memsafe::Secret<[u8; 16384]>`，与口令共用 `protected.rs`）；`expose()` 是**公开**的（SSH 层要读它），返回 `impl Deref<Target = [u8]>` 的**提权窗口**。空私钥与**超过一页（16384 B）**在**构造层**就被拒 —— 库里因此不可能有一条"读不出来"的行。⚠️ 读出时经过两块普通内存，**擦不掉的那一块照实说**：SQLCipher 的行缓冲走它自己的安全分配器（每个连接先开 `cipher_memory_security`），rusqlite 拷出来那个 `Vec` 被 `memsafe` 擦零 |
| **口令** | `Passphrase` = 口令在进程里的唯一形态：空值**造不出来**、**没有 `Debug`**（`{:?}` 是编译错误）、无 `Display`/`Serialize`、`expose()` 只对本 crate 可见、**不实现 `Clone`**；本体住在 `memsafe::Secret` 的一整页**受保护内存**里（`mlock` + 静止态 `PROT_NONE` + `dd` + `wf`，读它要 `&mut` = 一次提权动作） |
| **导出与还原（新）** | `export::to_encrypted(source, 源口令, dest, 导出口令)`：**两把口令相同 → `SharedPassphrase`**（D6 的"独立口令"）；`export::to_plaintext(source, dest, PlaintextAck)`：只收**那个凭据**，且文件名必须含 `plain`（大小写不敏感），否则**什么都不写**；`export::restore` / `restore_plaintext`：还原 = **替换**，只写进空的库槽（有内容 → `VaultExists`）。三段式：预建 **0600** 的 `…partial`（同目录）→ `ATTACH DATABASE ?1 AS export KEY ?2`（**口令走绑定参数**，不进 SQL 文本）→ `sqlcipher_export` → **显式写 `user_version = 1`**（不传递，D7）→ `DETACH` → **原子改名**；写完再用真读者 `open` 自检一次 |
| **dump（新）** | `dump::dump(&conn)` → `Dump { format_version, keys, hosts, serials, forwards }` + `row_counts()` + `to_text()`。**结构上不含私钥**（`Key` 里没有那个字段 —— 0403 定的，到这一层成了免费的性质），因为诊断的输出会进日志与 issue |
| **库文件的磁盘事实** | `akasha.db`（ADR-0002 D1，与 `config.json` 同目录）；建库后 **36864 字节 = 9 页**；SQLCipher 4.5.7 + vendored OpenSSL 3.6.3 + 内嵌 SQLite **3.46**；`user_version = 1` 是格式权威（`!= 1` 一律拒绝，**含 0**），**且要四张表都在**；盐 16 字节随机、就在文件头前 16 字节；SQLite 自己建出来是 **644**，我们显式收紧到 **600**；不带 `-wal` / `-shm`（D8）；解锁代价 **~105 ms**（KDF），所以解锁命令是 `async` + `spawn_blocking`。`cipher_memory_security` 是**进程级、只能开不能关**；它让 SQLCipher **每次分配 `mlock`、每次释放先擦零再 `munlock`** —— 解锁期间 `VmLck` 的大头是它（152 kB 里 148 kB），连接一 drop 就还回去 |
| **解锁与锁定（新）** | app 侧 `Vault { unlocked: Mutex<Option<Unlocked>> }`，`Unlocked { conn, passphrase }` —— **同生共死**（"锁定"只有一种写法）。`vault_unlock`：`Missing`/`Empty` → 建、`Present` → 开，返回四套池行数（`u32`，checked 转换）；已经解开 → `AlreadyUnlocked`（**不替换**）。`vault_lock` → 返回"刚才真锁上了一个吗"（幂等）。**只有显式锁**：关窗/退出不锁、无空闲超时。锁定时把两半显式 drop（连接的 SQLCipher 分配被擦零 + 口令页 `munmap`） |
| **口令经 IPC 进来的形态（新）** | `PassphraseInput`（newtype，`Deserialize` + `specta(transparent)` → TS `string`）：**没有 `Debug`/`Clone`**，唯一出路是 `into_bytes()`，缓冲由 `memsafe` 擦零。⚠️ tauri 自己那两份（请求体 + `serde_json::Value`）够不着（ADR-0002 §7.5） |
| **导出件的磁盘事实（新）** | 加密件与明文件都是 **36864 字节 / 600**；加密件头部是随机字节，**明文件头部是 `SQLite format 3\0`**（`文件里 grep 得到私钥` 是它"明文"的定义）；两者都带 `user_version = 1` |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| CSP | `default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:`；`devCsp` 多一个 `ws://localhost:1420 http://localhost:1420` |
| capabilities | 仍只有 `core:default` + `opener:default`（+测试用的 `victauri`）。**托盘、配置、单实例都没有加任何 permission**（全在 Rust 侧，前端碰不到，最小权限 §4.3）；库的三条命令（`vault_status` / `vault_unlock` / `vault_lock`）是**我们自己的 command**，不需要 ACL permission —— 前端能调的只有这三条，而它们不接受任何能指定路径 / 文件的参数 |
| 前提条件 | **需要能写 `$HOME`**；托盘另需能写 `$XDG_RUNTIME_DIR`、单实例另需会话总线（否则各自只降级、不影响启动）。导出另需目标目录可写（那正是它报错的地方） |

## 进行中 / 下一步

- [ ] **下一步 = plan 0405**（[可搬迁性验证](./plans/0405-portability-verify.md)，骨架）：`mv` 整个文件夹 →
  启动 → 断言"原有主机/密钥/规则都在"（**只验证"能开"不算过**），外加 `portable.md` §4 第 3 条
  "便携目录不可写就明确报错"（D12 的落地项，尚未实现）。
  **为什么现在做得了**：`vault_unlock` 会解库并返回四套池各有多少行 —— 那就是"搬家后内容还在"要的那一步；
  之前它没有真路径（写池的命令还没有，所以 E2E 造数据仍然直接调库函数）。
- [ ] **阶段 3 收口后的两条复核**（托盘时代带来的前提变化，都还没做）：
  - 0205 的看门狗生命周期仍然 = 一个 app 实例（ADR-0005 §6 的复审条件之一）；
  - 0305/0306 的前提 ② "关最后一个标签页 = 空状态"在"窗口隐藏"成为常态之后是否仍然合适。
- [ ] **ADR-0002 转「已定案」**（阶段 4 的 plan 0401–0405 全部完成时 —— 现在只剩 0405）。**加它的理由**：
  不定个时间点，它会永远停在"实现中"，而"不可修改"这份约束也就永远不会生效。
- [~] **plan 0102（CI 平台矩阵）**：本地部分完成，最终判据 = **推上去三个 job 全绿**，卡在没有 remote
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：本地已实测，剩 CI 三平台格子
- [ ] **正式 UI**：等设计稿（见上面「UI 现状」）—— 没有验收标准，故**不进 ROADMAP**

### 本轮完成（plan 0407：解锁与锁定的生命周期）

**判据是 ROADMAP 那一句**：解锁 → 读一次池 → 锁定之后**进程里不留机密**（`VmLck` 回落到解锁前）。
做成了**会红的测试**（库层 4 条 + app 单测 4 条 + 真路径 1 条），而且**先量再写**。

- [x] **先量再写**（探针写完即删）：把**整个进程内存**扫一遍找口令与派生密钥 —— 为此现学了两条测量纪律
  （坑 #90：扫描器自己的读缓冲就住在它要扫的地址空间里；坑 #91：给"大段"设上限会让该看见的副本落在窗口外）
- [x] **判据有正对照**：解锁期间必须比基线**多**扫到 —— 否则"锁上之后 0 命中"与"扫描器没在工作"是同一条绿
- [x] **量出三件原先是"上游这么说"的事**：口令在锁定时一处不剩；**SQLCipher 不留口令本体**（只有派生密钥）；
  `cipher_memory_security` 的"释放时擦零"真的发生（`sqlcipher_mem_malloc/free` 每次 `mlock` / 擦零 + `munlock`）
- [x] **四个问题各有结论**：谁持有（一个 `Mutex<Option<Unlocked>>`，连接与口令同生共死）、
  口令从哪来（`PassphraseInput` → `into_bytes()` → `Passphrase::new`，我们那一份被擦零；够不着的那两份照实记）、
  什么时候锁（**只有显式锁**：关窗不锁、退出不锁、不做空闲超时）、并发（第二个拿到 `AlreadyUnlocked`）
- [x] **`mlock` 失败变成一句人话**：`内存锁不住（mlock 失败），出于安全拒绝解锁：<OS 的原话>`，
  并且**不把上游那层包装**（`Memory error:`）丢给用户
- [x] **真路径**：新增 E2E `vault_unlock`（`just test-e2e` 的清单里）—— 测试进程**自己**读 app 的
  `/proc/<pid>/status`，所以那个 `0 → 192 → 0 kB` 不是 app 自报的数字
- [x] 门禁：`just ready` **6/6**；`just test` **180 passed**（`akasha-store` 72 → **76**）；
  `just test-e2e` **退出码 0**（15 个通过）

### 更早（各 plan 的细节在 `docs/plans/archive/` 里，这里只留结果）

- [x] **plan 0404**：dump 与导出 / 还原（加密导出可在另一目录还原；明文导出两道**写进实现**的门槛；
  不泄密带对照组的 grep）；细节在 [archive/0404](./plans/archive/0404-dump-export.md)
- [x] **plan 0403**：v1 的四张表 + 四套池 CRUD；不变量写进库（`STRICT` / `CHECK` / 外键 `RESTRICT`，D14）；
  P2 从散文变成两条判据（列名不许像路径 + 任何值不许提到数据目录，且是**现场枚举**表与列）；
  私钥按 D13 判据表重验（16384 字节那一页 / `VmLck` +16 kB / `drop` 后 `munmap`）；`vault_status` 上真路径
- [x] **plan 0406**：口令进 `memsafe` 的受保护页 —— **把上游四条承诺变成断言**，并**同时钉住边界**
  （`/proc/self/mem` 的读走 `FOLL_FORCE`，绕过页保护；有一条测试**故意断言"读得出来"**）。
  用户裁定为**通则**（`AGENTS.md` §3.4 + ADR-0002 D13）
- [x] **plan 0402**：口令 → KDF → 库密钥；`Passphrase` 类型（空值造不出来）；**拆开 `open` 与 `create`**
  （实测：文件不存在或 0 字节时**任何口令都能"打开"**）；`deny.toml` 常驻禁令把"无 `keyring` 依赖"变成门禁
- [x] **plan 0401**：SQLCipher 打开路径 + ADR-0002 §7 的实测清单；`unsafe` 收敛到本 crate 的**唯一单点**
- [x] **plan 0400**：ADR-0002 写完并进入「实现中」；ADR 三态（提议中 → 实现中 → 已定案）
- [x] **文档审计**：删掉指向从未存在的 `.taurignore` 的规则；正文不再写"行数 / 配方数"这类会漂的数字；
  端口敲门降级为 `later`
- [x] **plan 0301–0306 / 0201–0205 / 0107 / CI 去 Gitea 化 + 布局收口**（见 git 历史与 archive）

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。**仓库根没有 `Cargo.toml`** ——
  在根目录直接跑 `cargo …` 会失败（坑 #8），一律用 `just` 转发。⚠️ **临时脚本里也一样**。
- **四个 crate 的分工**：`akasha-core`（Session 模型 + 配置模型与判据，**零 Tauri 依赖**）、
  `akasha-pty`（`Transport` + portable-pty + 合批 + `teardown` + **`watchdog`**）、
  **`akasha-store`**（库的打开 / 创建 / 四套池 / dump / 导出与还原 —— 唯一允许 `unsafe` 的地方；
  **app 依赖它**，但只用"落点、状态与解锁"三样 —— 注意它把 `Connection` 再导出了一遍，
  那是为了让 app 不必依赖某个 `rusqlite` 版本，不是给它开一条绕开四套池直接写 SQL 的路）、
  `akasha`（app 包 = IPC 薄壳 + 托盘 + 配置 + 关窗语义 + 单实例 + 退出钩子 + 看门狗接线 + 事件 +
  **库的解锁状态**（`vault::Vault`）+ 代码生成 bin）。
- **前端四层**：`src/ipc/`（唯一允许碰后端）、`src/tabs/`、`src/terminal/`、`src/App.tsx`。
- **调试白屏**：Victauri 的 `logs {action:"console"}`；读不到"模块执行期就抛错"的失败 ——
  那时临时往 `index.html` 塞 `window.onerror` 钩子（坑 #35）。⚠️ **React 的 effect 清理函数里抛错
  会卸载整棵树**（坑 #50）。
- **调试 E2E / 真 app**：`just test-e2e` 的 app 日志落在 `$tmp/akasha-e2e-app1.log` / `-app2.log`；
  在**同一个 bash 调用**里才能同时读到 app 与测试（坑 #33）。
- **调试托盘**：走会话总线查 `org.kde.StatusNotifierWatcher` 的注册表与 `com.canonical.dbusmenu`
  的布局（坑 #62 / #63）。
- **调试"点叉之后怎么了"**：`app_state { probe: "lifecycle" }`；日志里还有 `config loaded` /
  `config not found` / `config invalid` / `close behavior degraded`。
- **调试"库里有什么"**：`akasha_store::dump::dump(&conn)`（**不含私钥**）；导出 / 还原的落点与权限
  看 `target/store-pools/` 下留下的真文件（契约测试**故意不删**它们）。
- **调试"锁上了没有 / 锁的时候还回了什么"**：`vault_status` 的 `unlocked`；日志里 `vault created` /
  `vault unlocked` / `vault locked` / `vault unlock failed`；内存那一半看
  `akasha-store/tests/unlock_lifecycle.rs` 的扫描（它会打印每一处命中的地址与段权限）。
- **文档三级粒度**：`ROADMAP.md`（判据）→ `docs/plans/TTxx-*`（手段）→ 本文件的坑（痕迹）。
- 命令入口分两处：项目级在根 `justfile`，crate 级在 `src-tauri/justfile`；
  **权威清单在 `docs/just.md` §2**，由 `just docs-check` 强制同步。

## 踩过的坑（避免重复踩）

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency**。
2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**。
3. **just 的变量写 `$p`，不是 Make 的 `$$p`**。
4. **just 用 justfile 所在目录作为配方工作目录**。
5. **系统库缺失只在 cargo 构建脚本阶段暴露**；本机是 CachyOS（Arch 系），不是 apt。
6. **Tauri 没有 Rust 热重载，Victauri 也不提供**。
7. **just 的 shebang 配方需要可写的 runtime dir**，受限环境会失败。
8. **仓库根没有 `Cargo.toml`** → 根目录下一切 cargo 命令失败（**临时脚本也算**）。
9. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** —— 跑一次 `just fmt`。
10. **CI 里 `libappindicator3-dev` 已不存在**，要用 `libayatana-appindicator3-dev`。
11. **受限环境下"写工作区之外被拒"看起来像工具/代码故障** —— 识别 → **直接提权重试**。
12. **`git checkout <file>` 会静默丢弃未提交的改动** —— 负例自检用 `cp` 备份/还原
    （⚠️ 备份别放 `/tmp`：沙箱每次调用一个私有 `/tmp`）。
13. **正向校验若不限定到目标段落就形同虚设** —— `docs-check` 用 `awk` 取 §2。
14. **`docs-check` 的反向检查**已扩到 `AGENTS.md` / `ROADMAP.md` / `docs/**/*.md`。
15. **后台遗留的 `just dev` 会让 Vite 继续监听 1420 而 app 早已不在**。
16. **含反引号的 grep 模式在 justfile 配方里必须整体放进单引号**。
17. **多文件行数检查要逐文件取**（awk 的 `NR` 会跨文件累加）。
18. **`[profile.*]` 写在 workspace 成员里会被静默忽略**。
19. **整体 `mv` 构建缓存会留下写死的绝对路径**。
20. **`cargo` 在成员目录里只选当前包** —— crate 级配方不写 `--workspace` 会**静默漏掉**成员。
21. **tauri CLI 默认只监听 `src-tauri`** —— 成员放外面 = 开发循环静默失效。
22. **cargo 的空 glob 是硬错误**（`members = ["crates/*"]`）。
23. **justfile 里不能出现完整的 `{{ … }}`**（要写字面量用 `{{{{`）。
24. **"一份工作流喂两个 forge"是一笔持续交的税**。
25. **兼容层的遗产会以"看起来更稳"的样子留下来**。
26. **阻塞的 `Read` 与"按时间交付"天生冲突** —— 正解是读线程 + `recv_timeout(期限)`。
27. **零匹配的测试过滤器在 nextest 里是"报错"**。
28. **`cargo bench` 会顺带用 bench 模式跑一遍单测目标**（正常）。
29. **仓库里出现第二个 bin 会让 `tauri dev` 起不来** —— 修法是 `default-run`。
30. **只写 `path` 的依赖等于版本号写 `*`** —— path 依赖要**同时写 `version`**。
31. **`Channel<Vec<u8>>` 不是二进制通道** —— 真正走 raw 的只有
    `Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`。**别只看字节数验收**：
    小消息（<1 KiB）的 raw 帧曾变成 `number[]`，用例里要数"JSON 帧 == 0"。
32. **`u64` 不能直接过 IPC**：改用壳层 `u32` 句柄 + **checked** 转换。⚠️ 被生成器拒绝的是
    **一整类**（`usize` / `isize` / `i64` / `u64` / `i128` / `u128`）—— plan 0407 又撞了一次
    （四套池的行数、`PassphraseTooLong.max`），两次都写成 checked 转换而不是 `as`。
33. **沙箱里 E2E 必须与 app 在**同一次** bash 调用内**（每次调用都是独立的 bwrap）。
34. **`pkill -f <模式>` 会匹配到自己** —— 用 `pkill -f '[v]ite'` 或按 PID/进程组杀。
35. **`@xterm/addon-unicode11` 需要 `allowProposedApi: true`** —— 不开的话 `loadAddon` 抛在
    React **effect** 里 → React 卸载整棵树 → 症状是**整屏白屏**且看不到报错。
36. **不在门禁里的测试等于没测**：`just test-e2e` 既不在 `ready` 里、又要真 app。
37. **`git mv` 之后 `docs-check` 会同时验两件事**（文件在不在、索引指得对不对）。
38. **每加一个依赖就多一份要维护的放行**：CSP 的 `style-src 'unsafe-inline'` 就是 xterm 逼出来的。
39. **"测试自己抛的异常"会污染同一 app 上后跑的用例**（raw 频道的收尾帧曾被当成数据帧）。
40. **vite 默认只监听 `[::1]:1420`**；`ls /tmp/victauri/<pid>/` 里的 `pid` 目录名**就是 app 的 pid**。
41. **`(cmd) &` 在 fish 里是命令替换，不是子 shell** —— E2E 敲进**用户登录 shell** 的命令必须
    在 fish / bash / sh 下语义相同（用 `sh -c '…' &`）。
42. **`cargo test` 一次收多个 `--test` 时按目标名字母序跑**，不按参数顺序。
43. **`tauri-plugin-log` 默认的 `TargetKind::LogDir` 会让"日志目录不可写"变成 app 打不开**。
44. **`tracing/log-always` 会把依赖树的 TRACE 一起转成 `log` 记录**（含 `tracing::span::active`）。
45. **SIGKILL 的投递是异步的**：`kill()` 返回后立刻读 `/proc/<pid>/stat` 会读到 `R`。
46. **portable-pty(unix) 的 `Child::kill()` 不是纯 SIGKILL**：先 SIGHUP、等 5×50 ms 宽限，再 SIGKILL；
    而且它只管那个 shell（plan 0204）。
47. **早于日志插件注册的 `tracing` 事件会静默消失**（看门狗的"已启动"因此推迟到 `.setup()`）。
48. **`/proc/<pid>` 存在 ≠ 进程还活着**：僵尸（`Z`）也有目录项。判活要读状态位。
49. **按"命令行里含某段文本"找进程会误伤**：bwrap 自己的 cmdline 里带着整段脚本文本。
50. **`term.dispose()`（xterm）会抛，而它跑在 React 的 effect 清理函数里** —— 双重后果：
    ① 抛在清理里 = React **卸载整棵树**；② 它若排在别的工作前面，后面的工作**永远执行不到**。
51. **多标签之后 DOM 选择器不再唯一** —— 输入与探针都跟着**活动面**走。
52. **断言超时不一定是"慢"**：本轮 E2E 报的"标签页只剩一个 超时"，真实原因是**界面已经被卸载**。
53. **`tauri-specta` 的事件必须 `mount_events`**：漏了不在启动时报错，而是在**发**的时候 panic。
54. **官方 `Channel` 不会告诉你"流结束了"**：收尾帧只把回调注销掉，**不通知** `onmessage`。
55. **会话里还有别的进程握着 PTY 时，主端读不到 EOF**：这时会话**不算**结束、标签页**不该**关。
56. **接线一次的回调必须走 `ref`**：否则走到过期闭包，表现是"晚发生的会话事件处理错了"。
57. **日志消息里的"括号解释"会自己长大**。正解是**消息只放事件名、变量进字段**。
58. **`tauri-plugin-log` 默认 formatter 的时间戳只到秒** —— 多进程日志顺序靠事件本身判断。
59. **`signal` 字段的值是本地化的**（`libc::strsignal`，zh_CN 下 `SIGKILL` 写成 `已杀死`），
    且信号编号在 portable-pty 内部就被丢掉。已知修法要动依赖，尚未做。
60. **托盘在 Linux 上要写盘**（`$XDG_RUNTIME_DIR/tray-icon/`）—— 只读 runtime dir 里建不起来，
    所以它只能是**可选能力**，且降级之后**不能**把"关窗口"做成隐藏（那样窗口再也叫不回来）。
61. **`libayatana-appindicator3` 与老的 `libappindicator3` 都是运行时 dlopen**，顺序前者优先。
62. **dbusmenu 的 item id 会随菜单重建而改变** —— 手写的测试客户端必须每次点击前重读布局。
63. **SNI 注册用的是唯一名**（`:1.x`），不是 `org.kde.StatusNotifierItem-<pid>-1`。
64. **Victauri 的 `window` 工具能机器验证窗口状态**（`manage_action` + `get_state.visible`）。
65. **`AppHandle::exit()` 也会触发 `RunEvent::ExitRequested`** —— 托盘模式下不能用它拦关窗。
66. **便携数据目录就在 bin 同目录**，而 `just dev` 与 `just test-e2e` **共用同一个 bin**：
    E2E 写进去的 `close_behavior=exit` 会影响下一次 `just dev`（配方先备份、跑完还原）。
67. **Victauri 的 REST 兜底接口返回的是 `{"result": …}` 包了一层**。
68. **`/proc/<pid>/exe` 可能带 ` (deleted)` 后缀**：cargo 重建会换掉那个 inode。
69. **文档里的"事实"没人核对就会自己长大**；共同病根是**门禁查不了"这句话还成不成立"**。
70. **cargo 的 workspace lint 继承是"全有或全无"**，而 `forbid` 不能被 `allow` 覆盖 ——
    正解：workspace 用 `deny` + 一条 ast-grep 规则把"唯一单点"圈回来。
71. **带 `links = "..."` 的原生库在依赖树里只能有一个版本**。
72. **SQLCipher 的空 key 不是"静默关掉加密"，而是"返回错误且不挂 codec"**。
73. **SQLite 自己建出来的库文件是 644**（umask 022），不是 0600。
74. **`PRAGMA cipher_settings` 的输出是一列 `pragma` 行**，不是"参数名 / 值"两列。
75. **cargo-deny 的图根是"manifest 指向的那个包"，不是整个 workspace**（加 `--workspace`）。
76. **"0 字节的库"不是"空库"，是"还没有密钥"** —— 所以"打开"与"新建"必须是两条路。
77. **`cipher_memory_security` 是进程级、单向的**（设 `OFF` 既不报错也不生效）。
78. **模块内的 `#[cfg(test)] mod tests` 也要自己 `allow(clippy::unwrap_used)`**。
79. **`/proc/<pid>/mem` 的读用 `FOLL_FORCE`，绕过页保护** —— `memsafe` 挡的是"意外"，不是"对手"。
80. **`/proc/self/smaps` 的字段不是处处都有，而且属性行带缩进**（`trim_start()` 之前 `strip_prefix` 永远
    返回 `None`）。
81. **子串匹配会把规则变成笑话**：`no_absolute_paths` 的第一版把 `forwards.direction` 判成了路径 ——
    正解是**按 `_` 分词、整词比较**，并配一对命中 / 诱饵负例。
82. **Victauri 的 `get_registry` 在本仓库是空的**（命令都没标 `#[inspectable]`）—— `AGENTS.md` §7
    那条当前只能靠"真路径上 `invoke_command` 成功"替代。⚠️ 沙箱里问 app 只能走 **REST 兜底**，
    且"起 app + 问 app + 收 app"必须在**同一次** bash 调用里。
83. **clippy 会把"两个常量比较"的断言判红**（`assertions_on_constants`）—— 搬进 `const { … }` 反而更好
    （"取值依据"从"某天有人跑测试才发现"升级成**编译不过**）。
84. **E2E 配方两段可能跑在**不同**的数据目录里**（便携目录不存在时 app 退回 OS 目录）——
    正解：两段都先 `mkdir -p` 便携目录。
85. **集成测试的共用脚手架放 `tests/common/mod.rs`，但必须自己 `#![allow(dead_code)]`**。
86. **`pkill -f <pattern>` 会匹配到你自己那条命令行** —— 要么用记录的 pid，要么 `pkill -x`。
87. **"什么都没发生"这类判据最容易写成永真式**：本轮"失败之后不留 `…partial`"看起来天经地义，
    可如果失败发生在**更早一步**（连半成品都没建出来），它照样绿。正解是**先用负例确认**：
    把 `discard()` 临时改成空操作 → 用例必须**变红**。同类还有"文件不存在""进程没起来"——
    断言之前先问一句"这条路径上它本来会不会存在"。**因此也别把失败点选在第一步**：
    目标指向一个**已存在的目录**，才能让失败发生在改名那一步（写已经成功、半成品已经躺在盘上了）。
88. **`sqlcipher_export` 写出来的文件默认是版本 0**（它不传递 `user_version`），而版本 0 的库我们
    自己会拒（D7）—— 症状是"**导出成功、还原打不开**"，而且报的是"不支持的版本 0"，
    完全不提"你少写了一句 pragma"。附挂库那句是 `pragma_update(Some(DatabaseName::Attached("export")), …)`
    （`Some` 里要的是 `DatabaseName`，不是 `&str`）。
89. **"导出另开一条实现路径"是最贵的那种省事**：明文导出看起来"无非是不加密"，实际上会多出
    一份会漂移的代码（漂移方向总是"明文那条少一个校验"）。先量一下 `ATTACH … KEY ?` 吃不吃绑定参数、
    空 key 参数算不算 `KEY ''` —— 两条实测把这件事变成了"同一段代码，只是 key 给空"。
    同理，口令要经**绑定参数**进 `ATTACH`：文本形式与 D4 拒绝 `PRAGMA key = '…'` 是同一个理由。
90. **扫描器自己就住在它要扫的地址空间里**：第一版"整进程内存找口令"每次分配一块 64 MiB 的读缓冲，
    而那块缓冲**也在被扫的段里** —— 数字里混进了它自己的拷贝，而且越扫越多。正解：**缓冲复用 + 读完即擦**，
    并且**针要真随机**（运行期算出来的固定序列会跟内存里别的东西撞上，基线里就冒出十几处命中，
    于是分不出"我们的缓冲区"与"别处"）。
91. **给"要扫的段"设上限 = 让该看见的副本落在窗口外**：按 8 MiB 截断时，SQLCipher 的 codec 副本
    正好在一个更大的段里 —— 扫描器"什么都没扫到"，而用例照样绿（坑 #87 的同一类：**漏扫与没泄是同一条绿**）。
    正解是**按"是不是匿名段"过滤**（文件支撑的只读映射里不可能有我们的机密）而不是按大小，
    并且用**正对照**（解锁期间必须多扫到）兜住"扫少了"。
92. **解锁期间 `VmLck` 涨的**大头不是我们那一页**：`cipher_memory_security = ON` 会把 SQLite 的分配器
    换成 `sqlcipher_mem_malloc/free` —— 每次分配 `mlock`、每次释放先擦零再 `munlock`。
    实测口令页只占 4 kB，而解锁中有 152 kB。看到数字涨到几十上百 kB 别去翻 `memsafe`，
    那是 SQLCipher 自己锁的（目录见 ADR-0002 §7.5）。
