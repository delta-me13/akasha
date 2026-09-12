# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-12

## 一句话

**阶段 2「端到端最小终端」5/5 完成**；**阶段 3「托盘与应用生命周期」6/6 完成**。
**阶段 4「存储与凭据池」在做（5/9）**：ADR-0002（机密存储与可搬迁）**已进入「实现中」**。
已落地四块：**SQLCipher 加密库能开**（plan 0401）、**口令只从一条路进来并能真的验证它**
（plan 0402 —— 拆开了"打开"与"新建"，原来在没有文件的路径上**任何口令都能开**，
并把"无 `keyring` 类依赖"变成**常驻门禁**）、**口令在内存里也受保护**（plan 0406）、
**四套池能增删改查**（plan 0403 —— v1 的四张表、不变量写在库自己身上、
私钥读出来进受保护页；**库第一次被 app 依赖**，`vault_status` 走通了真路径）。
下一步是 plan 0404（dump 与导出）与 0405（可搬迁性验证）；本步顺带把**解锁生命周期**
立成 plan 0407（谁持有解好的连接、口令从哪来、锁定时抹什么）—— 它是 0404/0405 之后的事。

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
| `just test` | **160 tests run: 160 passed**（`akasha` 47 + `akasha-pty` 37 + `akasha-core` 15 + **`akasha-store` 61**） |
| ↑ 本轮新增 | **38 条**（plan 0403）：四套池的 round-trip（8）、v1 的形状与外键/`CHECK`（10）、**库里没有绝对路径**的两条判据 + 规则自己的诱饵负例（4）、**私钥那一页**按 D13 判据表重验（2）+ 各枚举 `parse`/`as_str` 往返等模块内单测 |
| ↑ **四套池（plan 0403）** | ✅ 建库后文件 **36864 字节 = 9 页**（0402 时是一页 4096）、内嵌 SQLite **3.46**（`STRICT` 表要 ≥3.37，有断言）、`PRAGMA foreign_keys` 读回 **1**（sqlite 默认是**关**的 —— 声明了不生效是静默失效）、删还被引用的密钥/主机 → `Conflict` 且行还在。**私钥那一页**：smaps 多出一页 `---p`、正好 **16384 字节**，`VmLck` **68 → 84 kB**，`VmFlags = mr mw me lo ac wf dd sd`，`drop` 之后那一页**消失**（`munmap` 还回去了） |
| ↑ **内存防护（plan 0406）** | ✅ 进程级 `VmLck` **0 → 4 kB**；那一页在 smaps 里是 `---p`、正好 4096 字节；`VmFlags = mr mw me lo ac wf dd sd`（**`dd` 不进 core dump、`wf` fork 后清零**）。⚠️ **同一批测试也钉住了边界**：`/proc/self/mem` 读那一页**成功**（`FOLL_FORCE` 绕过页保护）—— 这一层挡的是"意外"，不是"能在你进程里跑代码的人" |
| ↑ **两条判据（plan 0402）** | ✅ **无 `keyring` 类依赖**：`deny.toml` 的 `[bans] deny` 常驻禁令 + `just deny-offline` 一直守着（负例验过：临时加 `keyring` → `bans FAILED` 并逐个报出）；✅ **口令不以任何形式落盘**：标记口令跑完 `create` + 解锁 + 解锁失败后，递归扫数据目录 **0 命中**，而对照目录的诱饵文件 **1 命中**（证明扫描器不是坏在原地） |
| ↑ **一处推翻隐含假设的实测（plan 0402）** | ⚠️ **文件不存在或 0 字节时，任何口令都能"打开"** —— 库里没有东西可解、KDF 根本没跑（**~0.19 ms**，真实解锁 **~105 ms**）。于是 0401 的 `open()` 在"新建"这条路上**没有验证过口令**。已拆成 `open`（没有就 `NoVault`）/ `create`（**永不覆盖**，写 `user_version = 1` 把口令钉进文件） |
| `just test-e2e`（自包含：起 Vite + app → **两段** → 收尾） | 退出码 **0**，**14 个用例通过**（其中 `window_close` **显式跳过**：这台机器上 `tray_ready=false` → 关窗语义降级为退出，它只验"隐藏"那条路并打印了判据）：`smoke` 3 / `integration` 2 / `session_channel` 1 / `terminal_render` 2 / `tab_close` 1 / `single_instance` 2 / **`vault_status` 1** / `exit_residue` 1 |
| ↑ **`vault_status` 真路径（plan 0403）** | ✅ 真 app 上 `invoke_command("vault_status")` → `{"path":"…/src-tauri/target/debug/akasha-data/akasha.db","state":"missing"}`，与测试进程自己 `stat` 同一个文件得到的结论一致；父目录名是 `akasha-data` 且它是 **bin 同目录**（P2 的落点判据第一次走通真路径） |
| ↑ **§7 的 registry 那一条：当前不可满足**（实测，见坑 #82） | `get_registry` 回 **`[]`** —— 本仓库的命令**都没有 `#[inspectable]`**，注册表不镜像命令集；`detect_ghost_commands` 的 `confirmed_ghosts` 也是空的，但它自己的 `reliability` 是 **low**（前端一次命令都没调过）。**替代证据**：真路径上的 `invoke_command` 成功（上面那一行）。用 REST 兜底问到的（沙箱里 MCP 连不到另一个 bash 命名空间里的 app，坑 #33） |
| ↑ **单实例（0304，Linux 实测）** | ✅ probe `{"activations":0,"registered":true}`、日志 `single instance registered`；`window manage hide` → `visible=false`；再起同一个二进制 → **150–205 ms** 后 `exit=0`、`activations=1`、`visible=true`；进程表只剩 app + 它的看门狗；**藏起来之前的屏幕内容仍在**（是原来那个窗口） |
| ↑ **单实例降级（0304）** | ✅ `unset DBUS_SESSION_BUS_ADDRESS` + runtime dir 里没有 `bus` → 日志 `single instance unavailable`、probe `{"registered":false}`，**窗口照常起来**（降级不挡启动） |
| ↑ **dev 重启不被挡（0304）** | ✅ `kill -9` 主实例后立刻重启：新实例照常注册（D-Bus 名字挂在连接上，进程一死就释放，没有陈旧的锁） |
| ↑ **关窗 = 隐藏（0302，Linux 实测）** | ✅ 第一段（没有配置文件 = 默认收托盘）：`app_state{probe:"lifecycle"}` → `{"close_action":"hide","close_behavior":"tray","tray_ready":true}`；`window manage close` 之后**进程仍在**、`visible=false`；隐藏期间屏幕内容仍读得到、忽略 SIGHUP 的探针仍活着（**预期**）；`show` 之后还能继续敲命令 |
| ↑ **关闭行为可配置（0303，Linux 实测）** | ✅ 三种情形：**文件不存在** → 默认 tray，日志 `config not found … path=~/.local/share/fans.cyrene.akasha-terminal/config.json`；**`{"close_behavior":"exit"}`**（放在 `<target>/debug/akasha-data/`，即便携分支）→ probe `close_action=exit`、关窗后 app 退出且 `sessions reclaimed reclaimed=1 trigger="exit"`；**`"nope"`** → `config invalid err=unknown close_behavior value "nope" (expected "tray" or "exit")` + 回默认，app 照常启动 |
| ↑ 关标签页 = 立刻丢弃会话（0305，真 UI 点击） | ✅ 两个忽略 SIGHUP 的探针 → 点 `+` / 切换 / 点 `×`：探针 A 在 **79 ms** 内消失、**不需要第二次点击**；探针 B 仍在且屏幕内容还在；关掉最后一个 → 空状态 + 探针 B 也被丢 |
| ↑ 敲 `exit` → 标签页跟着关（0306，反方向） | ✅ `在终端里敲 exit：标签页自己关掉（app 仍在）`；随后再开一个标签页可交互 |
| ↑ 退出零残留（0204/0205，未退化） | ✅ `app 已退出（pid 3008）`；`✅ 零残留：忽略 SIGHUP 的 3758 已随会话被收掉`。这条现在同时是**"配置真的被读到"的证据**：配置没生效的话关窗只会隐藏 |
| ↑ 终端判据（未退化） | `renderer = webgl`、8 MB 灌流后仍可交互；`WEBGL_lose_context` 后退到 canvas **且屏幕内容保留** |
| ↑ 会话判据（未退化） | raw 通道 10.73 MB / 164 批；收尾帧 1 个、console 零异常 |
| `pnpm build`（tsc + vite build） | 退出码 0；产物 **843 kB / gzip 231 kB**；生产包里 `akashaTerminal` / `activateProbe` / `mockIPC` 命中数 **0**（本轮未改前端，数字沿用） |
| `just check` / `just clippy`（`--workspace --all-targets`） | 退出码 **0** |
| `just deny-offline` | `bans ok, licenses ok, sources ok`。⚠️ 本轮**发现并修好了一个盲区**：cargo-deny 默认只把 manifest 指向的包当图根（本仓库 workspace root 同时是真实包 `akasha`），于是 `crates/*` 里尚无人依赖的成员**连同它独有的整棵子树都不在图里** —— `[bans] deny` 写 `keyring` 也静默不生效。加 `--workspace` 后图 **580 → 583**，负例立刻从 `bans ok` 变成 `bans FAILED`。**这同时补上一个先于本轮的洞**：`akasha-store` 的 vendored OpenSSL 此前从未被许可证门禁看过 |
| `just docs-check` | 全过（ROADMAP 条目在 3 行内且无代码块 / plan ≤200 行且索引一致） |
| `ast-grep scan` | 退出码 **0**；**六条**规则均已用正负例验证（本轮新增 `no-unsafe-outside-store`，用"真 unsafe 命中 / akasha-store 里的同类不命中 / 注释与字符串里的 unsafe 不命中"三例验过才删探针） |
| `cargo tree -p akasha-core` \| `grep -c tauri` | **0**（分层成立；单实例与配置载体都只在 app 包里） |
| `just bench`（criterion） | 52.7 GiB/s / 14.1 ns 每批 / 9.64 GiB/s（**0201 的数字，本轮未复跑**） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。

## 待验证（本地跑不了 / 沙箱跑不了）

- **单实例在 Windows / macOS 上未验**：本机只有 Linux。机制完全不同（命名 mutex / `/tmp` 下的
  unix socket），而 CI 的**类型检查挡不住运行期差异** —— 这三个平台各点一次是唯一的办法。
- **CI 的 Linux E2E 上 `single_instance` 必然跳过**：xvfb 没有会话总线，app 降级为"可以多开"，
  用例**显式跳过**并打印 probe。也就是说这条判据只在**有会话总线的开发机**上被执行
  —— 与托盘、`window_close` 是同一个缺口。
- **托盘图标在面板里"看得见"** —— 机器只能证明"注册进了 watcher"，**不能**证明宿主面板
  有托盘模块（`scope.md` §5.5）。
- **`window_close`（关窗 = 隐藏）在 CI 上必然跳过**：它需要托盘宿主（会话总线 + 可写
  `$XDG_RUNTIME_DIR`），xvfb 两样都没有。
- **托盘没有自动化门禁**：验它要 D-Bus 会话总线 + 宿主 watcher，CI（xvfb）两样都没有。
  本轮的证据是**手工实机**（命令与输出在 `docs/plans/archive/0301` 的实施记录里）。
- **CI 三个 job 是否真能变绿** —— 仓库还没有 remote，从没跑过。三条只在真 runner 上见分晓的
  风险记在 [`docs/plans/0102`](./plans/0102-ci-platform-matrix.md) 的实施记录里。
- **E2E 矩阵的三个格子**（Linux/xvfb + macOS + Windows，三格跑同一条 `just test-e2e`）——
  同上：没有 remote 就没跑过。本地只覆盖 **Linux/Wayland** 这一格。
- **`tauri dev` 重载那条路径没有门禁**：只能手动实测（plan 0205 的实施记录里有脚本与输出）。
- **前端类型检查不在任何门禁里**：`just ready` 只覆盖 Rust + 文档，`pnpm build`（tsc）要手动跑。
- **在"有后台作业握着 PTY"的标签页里敲 `exit`**：不会有 EOF、不会关标签页（刻意，见坑 #55），
  但没有用例守着它。
- **宿主 MCP 连不到沙箱内运行的 app**（私有 PID / 临时目录）。沙箱内可用，
  但**必须让 app 与测试在同一次 bash 调用里**（坑 #33）。
- **`just dev-web` 的模拟后端没在真浏览器里点过**（本环境没有浏览器）。
- **大流量下的 JS heap 数字没取**。

## 当前基线（2026-09-12 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` / **`akasha-store`** |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，坑 #29） |
| 后端模块 | `bindings`（命令 + 事件 + 代码生成）/ `session`（会话表 + 回收）/ `tray`（托盘）/ `config`（配置文件载体 + **数据目录**）/ `lifecycle`（关窗语义 + probe）/ `single_instance`（单实例 + probe）/ **`vault`（库的落点与状态）** / `watchdog`（进程外兜底） |
| **关窗语义** | 判据 = `akasha-core::CloseAction::decide(close_behavior, tray_ready)`；app 侧 `CloseRequested` → **先 `hide()`、成功才 `prevent_close()`**。**不挂 `RunEvent::ExitRequested`**（理由见 `lib.rs` 注释与坑 #65） |
| **单实例** | 插件注册在**第一个插件位**（= 第二个实例在别的插件的 setup 之前就退掉；"不会先闪窗口"由 tauri 的时序保证，与顺序无关）；唤起 = `unminimize()` → `show()` → `set_focus()` **三步无条件都做**；`available()` 在 Linux 上 = 会话总线连得上 |
| **配置** | `<数据目录>/config.json`，`{"close_behavior":"tray"\|"exit"}`；`serde_json` + `deny_unknown_fields`；**只读不写**；在 `.setup()` 里读一次 |
| **数据目录** | bin 同目录存在 `akasha-data/` → 用它（便携）；否则 `app_data_dir()`。**不自动创建**。开发构建里便携目录 = `src-tauri/target/debug/akasha-data` |
| **系统托盘** | 图标 = `bundle.icon` 那张（构建脚本已解码进二进制）；Linux 上落盘到 `$XDG_RUNTIME_DIR/tray-icon/tray-icon-akasha-0.png`；菜单 id 是稳定字面量（`window.toggle` / `tunnels` / `tunnels.empty` / `app.quit`）；**会话表一变整份重建**（dbusmenu revision +1、item id 全换） |
| **`lifecycle` probe** | `app_state { probe: "lifecycle" }` → `{"close_behavior":…,"tray_ready":…,"close_action":…}`；启动期还没登记时回 `{"initialized": false}`（E2E 靠它决定"该验隐藏还是该跳过"） |
| **`single_instance` probe** | `app_state { probe: "single_instance" }` → `{"registered":bool,"activations":n}`；`registered` 决定 E2E 真跑还是跳过，`activations` 是"第二个实例的话真的带到了这个进程"的证据 |
| 出字节路径 | PTY read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write` |
| 前端结构与布局 | `src/tabs/TabStrip.tsx`（标签栏）+ `src/App.tsx`（标签模型，多面**同时挂载**、非活动的 `visibility: hidden` 叠放）+ `src/terminal/`（xterm 面与会话接线）+ `src/ipc/`（唯一的后端入口） |
| **关闭一个标签页** | 移除 → React 卸载该面 → `attachTerminal` 清理（**先** `close_session`，**后** `surface.dispose()`）→ `Sessions::close` → `Transport::shutdown()` → 撤销看门狗登记。**实测 79–93 ms** |
| **会话自己结束** | 合批读循环结束（EOF / EIO）→ `forward` 收工 → 收尾线程 `Sessions::retire`（收尸 + 摘牌 + `registry.close` + `forget`）→ `app.emit("session_ended", SessionEnded { handle, status })` → 前端关掉那个标签页 |
| 回收路径（进程内） | `RunEvent::Exit` / panic hook / `close_session` / `retire` / **托盘退出** → `Transport::shutdown()` |
| 回收路径（进程外） | 看门狗（每个 app 实例一个）读管道：`register` 写 `+<会话首进程 pid>`，收干净后写 `-<pid>`；**EOF = app 死了** → 逐个 `kill_session` |
| 事件通道 | `tauri-specta` 生成 `events.sessionEnded`（`src/ipc/bindings.ts`）；`Builder::mount_events` 在 `.setup()` 里必须调用（坑 #53） |
| 前端渲染器 | **WebGL**（WebKitGTK + MESA 软件栈下仍拿到 WebGL2）；`canvas` 元素 2 块；DOM 渲染器未启用 |
| 大输出实测 | 11.18 MB / 170 批（0202）；11.28 MB / 168 批（0204）；10.80 MB / 162 批（0205）；10.41 MB / 159 批（0305）；10.36 MB / 159 批（0302/0303）；10.73 MB / 164 批（0304） |
| 前端产物 | 843 kB（gzip 231 kB） |
| **存储层** | `akasha-store`：**全仓库唯一允许出现 `unsafe` 的 crate**（送口令进 `sqlite3_key()`，ADR-0002 D4），由 workspace 的 `unsafe_code = "deny"` + `.ast-grep/rules/no-unsafe-outside-store.yml` 两层守。**两条路**：`create(path, &Passphrase)` = 有内容就 `VaultExists`（永不覆盖）→ 送密钥 → **一次事务里建 v1 的四张表 + 写 `user_version = 1`**；`open(path, &Passphrase)` = 不存在的/0 字节就 `NoVault` → 送密钥 → 开外键 → 读一次 `sqlite_master` 逼口令错暴露 → **校验版本号 + 四张表都在**（缺表 → `MissingTable`）。两条路都收 0600。**app 现在依赖它**（`akasha-store` 进依赖图也补上了 cargo-deny 的图根盲区） |
| **四套池（新）** | `keys` / `hosts` / `serials` / `forwards` 四个模块，各 5 个函数（insert / get / list / update / delete）+ 反查（`hosts_using_key`、`hosts_jumping_to`、`forwards_of_host`）。`New*`（没有 id）与 `*`（有 id）**是两种类型**；不变量写在库上（`STRICT` + `CHECK` + 外键 `RESTRICT`，ADR-0002 D14），应用层只把 sqlite 的失败翻成 `Conflict`。⚠️ 跳板链的成环**库表达不了**：由 `update` 时逐跳走链挡住（`MAX_JUMP_DEPTH` = 32 是防死循环的兜底） |
| **私钥（新）** | 池里存 BLOB，**出库直接进受保护页**（`PrivateKey` = `memsafe::Secret<[u8; 16384]>`，与口令共用 `protected.rs`）；`expose()` 是**公开**的（SSH 层要读它），返回 `impl Deref<Target = [u8]>` 的**提权窗口**而不是守卫类型。空私钥与**超过一页（16384 B）**在**构造层**就被拒 —— 库里因此不可能有一条"读不出来"的行。⚠️ 读出时经过两块普通内存，**擦不掉的那一块照实说**：SQLCipher 的行缓冲走它自己的安全分配器（每个连接先开 `cipher_memory_security`），rusqlite 拷出来那个 `Vec` 被 `memsafe` 擦零 |
| **口令** | `Passphrase` 类型 = 口令在进程里的唯一形态：空值**造不出来**、**没有 `Debug`**（`{:?}` 是编译错误）、无 `Display`/`Serialize`、`expose()` 只对本 crate 可见、**不实现 `Clone`**；本体住在 `memsafe::Secret` 的一整页**受保护内存**里（`mlock` + 静止态 `PROT_NONE` + `dd` + `wf`，读它要 `&mut` = 一次提权动作）。本 crate 没有任何日志设施，也没有 argv / 环境变量 / 配置读取口 —— 口令只能作为 `&mut Passphrase` 参数进来 |
| **库文件的磁盘事实** | `akasha.db`（ADR-0002 D1，与 `config.json` 同目录）；建库后 **9 页 / 36864 字节**（v1 = 版本号 + 四张表）；SQLCipher 4.5.7 + vendored OpenSSL 3.6.3 + 内嵌 SQLite **3.46**；`user_version = 1` 是格式权威（`!= 1` 一律拒绝，**含 0**），**且要四张表都在**；盐 16 字节随机、就在文件头前 16 字节（`PRAGMA cipher_salt` 逐字节相同）；SQLite **自己建出来是 644**，我们显式收紧到 **600**；不带 `-wal` / `-shm`（D8，`journal_mode` 保持 `delete`）；解锁代价 **~105 ms**（256,000 次 PBKDF2-HMAC-SHA512）。`cipher_memory_security = ON` 是**进程级、只能开不能关** |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| CSP | `csp`：`default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:`；`devCsp` 多一个 `ws://localhost:1420 http://localhost:1420` |
| capabilities | 仍只有 `core:default` + `opener:default`（+测试用的 `victauri`）。**托盘、配置与单实例都没有加任何 permission** —— 它们全在 Rust 侧，前端碰不到（最小权限，§4.3） |
| 前提条件 | **需要能写 `$HOME`**；沙箱内会刷 `dconf-CRITICAL` 与 WebKit 缓存 hard-link 告警，但 app 仍正常起窗口。**托盘另需能写 `$XDG_RUNTIME_DIR`**、**单实例另需会话总线**（否则各自只降级、不影响启动） |

## 进行中 / 下一步

- [ ] **阶段 3 收口后的两条复核**（托盘时代带来的前提变化，都还没做）：
  - 0205 的看门狗生命周期仍然 = 一个 app 实例（ADR-0005 §6 的复审条件之一）；
  - 0305/0306 的前提 ② "关最后一个标签页 = 空状态"在"窗口隐藏"成为常态之后是否仍然合适
    （前提 ① 已由 `window_close` 守住，③ 已有实测支撑）。
- [ ] **下一步 = plan 0404**（[dump 与导出](./plans/0404-dump-export.md)）→ 0405（可搬迁性验证）。
  两者都要库能开、四套池能读写，现在都具备了。⚠️ 0404 的"明文导出必须二次确认"是一条**门槛**
  （不是 UI 细节）：判据要写成"没有确认就走不到那条路"，而不是"界面上有个勾"。
- [ ] **解锁生命周期已立 plan 0407**（骨架，[0407](./plans/0407-unlock-lifecycle.md)）：谁持有解好的
  `Connection`、口令从哪来、锁定时抹什么。**加它的理由**：plan 0403 只接了"库在哪、建过没有"
  （纯函数 + 一次 `stat`，不需要先定生命周期），而下一步真正要读写池就必须回答这四个问题。
  ⚠️ 展开时要处理 `mlock` 失败（`Passphrase::new` 会直接报错 —— 要变成用户能懂的一句话）。
- [x] **机密的内存防护已定为通则**（`AGENTS.md` §3.4 + ADR-0002 **D13**）：口令、私钥、会话令牌、
  Bitwarden 主密码与 `BW_SESSION` 一律走 `memsafe`，**不自己写** `mlock` / `mprotect` /
  `VirtualLock` 封装。**D13 那条"每新增一个用途重验一遍"已经用掉一次**：私钥（`N` 从 256 变成
  16384）按同一张判据表重验，实测记进 ADR §7.3 —— 下次是"会话令牌"或 `BW_SESSION`。
- [x] **ADR-0002 §10 已有 12 条修订**（本轮 +5：D7 补"v1 = 版本号 + 四张表"、D9 的私钥列改 BLOB、
  新增 **D14**「不变量写进库」、新增 **§7.3** 的实测、§7.1 那条 4096 字节的旧数字标注取代）——
  三态里「实现中可改」的用法已经成型：**先改 ADR、记一行，再往下写代码**。
- [ ] **ADR-0002 转「已定案」**（阶段 4 的 plan 0401–0405 全部完成时）——
  ROADMAP 阶段 4 末尾新增的条目。**加它的理由**：不定个时间点，它会永远停在"实现中"，
  而"不可修改"这份约束也就永远不会生效。
- [~] **plan 0102（CI 平台矩阵）**：本地部分完成，最终判据 = **推上去三个 job 全绿**，卡在没有 remote
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：本地已实测 —— `just test-e2e` 自包含、
  **13 个用例全绿**（含两段配置）；剩 CI 三平台格子（同上）
- [ ] **正式 UI**：等设计稿（见上面「UI 现状」）—— 没有验收标准，故**不进 ROADMAP**

### 本轮完成（plan 0403：四套池的 CRUD）

**判据是两条**：各自的 round-trip 单测通过、**库里不存绝对路径**（P2）。

- [x] **四套池 + v1 的形状**：`create` 在**一次事务**里建四张表并写 `user_version = 1`
  （中途断电不会留下"有表没版本号"的半成品），`open` 除版本号外**还查四张表在不在**
  —— `user_version = 1` 的含义从此是"**这四张表**"，不是"一个空库"（D7）。列的**形状**
  有一条快照测试钉着：它红了就是"格式变了"，不是"测试过时了"
- [x] **不变量写进库**（新增 **ADR-0002 D14**）：`STRICT` 表 + `CHECK` + 外键 `ON DELETE RESTRICT`，
  `foreign_keys` **每个连接**开一次（sqlite 默认**关**，声明了不生效）。含一条独立的读回断言
  —— "我们以为开了"不是证据。**代价也记了**：错误来自 sqlite，翻译成 `Conflict { pool, detail }`
  会丢掉"到底是哪条约束"，但用户要做的事反正是同一件
- [x] **P2 从散文变成两条判据**：① 没有**列名**像路径（规则按 `_` 分词、整词比较 ——
  诱饵 `direction` 不许被误判，见坑 #81）；② **任何一个值**都不许提到我们的数据目录。
  第二条是**现场枚举**表与列（`sqlite_master` + `PRAGMA table_info`），所以**将来加的列自动进检查**
- [x] **私钥也进受保护页**（D13 判据表重验）：`N` 从 256 变 16384 → 那一页从 1 页变 4 页、
  `VmLck` +16 kB，四条判据 + 第五条"读完还回去"（`munmap`）。`Protected<N>` 从口令那一份提出来
  两处共用 —— 抄第二份等于让两份各自漂移
- [x] **接进 app 只接"落点与状态"**：`vault_status` 命令 + `config::data_dir_of`，
  **不接解锁生命周期**（它是 plan 0407：谁持有连接、口令从哪来、锁定时抹什么 —— 那是生命周期决定，
  不该顺手定下来）。没有注册 probe：状态本身就是一条命令，再加一个只会**多一条观察路径、不多一点信息**
- [x] **顺带修好 E2E 配方的一个隐患**：便携数据目录不存在时 app 会退回 OS 数据目录，
  于是**两段跑在不同的数据目录里**（第二段写的 `close_behavior` 读不到，第一段还可能读到
  OS 目录里上一次留下的配置）。现在两段都先 `mkdir -p` 便携目录（坑 #84）
- [x] 门禁：`just ready` **6/6**；`just test` **160 passed**（`akasha-store` 25 → **61**）；
  `just test-e2e` **退出码 0**（14 个通过 / 1 个显式跳过）

### 上一轮完成（plan 0406：口令的内存防护）

**判据**：口令在内存里**不被换出、不被 dump、静止时读不到**，而且这四条**各有一条会红的测试**。

- [x] `Passphrase` 从"普通堆上的一块 `Vec<u8>`"换成 `memsafe::Secret<[u8; 256]>` 的
      **一整页受保护内存** —— 选 `Secret<[u8; N]>` 而不是 `MemSafe<Vec<u8>>`（后者只保护
      `Vec` 的 24 字节头，字节还在普通堆上，上游文档管这叫 "the `MemSafe<Vec<u8>>` pitfall"）
- [x] **把上游的四条承诺变成断言**（plan 0406 的判据表）：`mlock` 看进程级 `VmLck`、
      `PROT_NONE` 看 smaps 权限位、`dd` / `wf` 看 `VmFlags`。
      "那一页是我们的"不靠猜 —— **建口令前后各取一次快照，多出来的一页就是它**
- [x] **同时钉住边界**（比"证明有防护"更重要）：`/proc/self/mem` 的读走 `FOLL_FORCE`，
      **绕过页保护**，那一页照样读得出来。这条测试**故意断言"读得出来"** ——
      哪天它开始失败，说明保护变强了，该回来改 ADR 的措辞
- [x] 对外语义没变：空值仍造不出来、仍进不了日志、仍是任意字节；`expose()` 现在返回
      一个**提权窗口**（守卫 drop 就降回 `PROT_NONE`），`open` / `create` 因此收 `&mut`
- [x] **ADR-0002 D5 的内存立场改写**并记 §10：0402 当时写"不做内存擦除"，
      本步推翻的是**结论而不是那句事实** —— 口令是唯一能解开整库的东西、生命期跨多次解锁，
      而 core dump / swap / fork 是真实发生过的泄露路径
- [x] **定为通则**（本步之后，用户裁定）：`AGENTS.md` §3.4 新增一条"内存里长住的机密统一经
      `memsafe`"，ADR-0002 新增 **D13** —— 判据表（四条会红的测试）、六条已知不足、否决的路
      （自研封装 / 只靠 `zeroize` / 靠 `cipher_memory_security` 一把梭）都在那里。
      **跨平台这条不是推测**：实测 `memsafe` 为 `x86_64-pc-windows-msvc` 与
      `aarch64-apple-darwin` 都编得过（CI 从未跑过，所以这是目前唯一的跨平台证据）
- [x] 门禁：`just ready` **6/6**；`just test` **122 passed**（`akasha-store` 22 → **25**）

### 更早（plan 0402：口令 → KDF → 库密钥）

**判据是两条**：无任何 `keyring` 类依赖、口令不以任何形式落盘。

- [x] **`Passphrase` 类型**：空口令从"打开函数里的一个 if"升级成**类型不变量** ——
      空值造不出来、`Debug` 只打 `<redacted>`、没有 `Display`/`Serialize`、
      `expose()` 只对本 crate 可见。于是 D5 那句"不进日志"不是纪律而是**写不出来**
- [x] **`open` / `create` 拆开**（本步最要紧的一条）：实测发现**文件不存在或 0 字节时任何口令
      都能"打开"** —— 空的库里没有东西可解、KDF 根本没跑，所以 0401 的 `open()` 在"新建"这条路上
      **没有验证过口令**，还把这把错口令当成了创建口令。现在 `create` 用 `user_version = 1`
      把口令钉进文件（文件随即从 0 变成 4096 字节），`open` 只开已有的库、并校验版本
- [x] **`cipher_memory_security = ON`**（ADR-0002 §6 的"建议开"）：让 SQLCipher 的密钥材料
      在释放时被擦除。实测它是**进程级、只能开不能关**，且排在 `sqlite3_key` **之前**更优 ——
      §6 与 D4 的措辞据此改精确（各记一行 §10）
- [x] **判据 ① 落成常驻门禁**：`deny.toml` 的 `[bans] deny`（`keyring` + 各平台后端）——
      不再是一次性的 `cargo tree` 检查
- [x] **顺带修好门禁的盲区**（不在原计划里）：cargo-deny 默认只把 manifest 指向的包当图根，
      `crates/*` 里尚无人依赖的成员**连同它独有的整棵子树都不在图里**。加 `--workspace`
      之后负例才真的红，同时补上一个先于本轮的洞（vendored OpenSSL 从未被许可证门禁看过）
- [x] **ADR-0002 新增 4 条 §10 修订**（D5 类型化 / D4 顺序措辞 / D7 的 `<1` 改拒绝 / §6 内存安全）
- [x] 门禁：`just ready` **6/6**；`just test` **119 passed**；`just deny-offline`
      `bans ok, licenses ok, sources ok`（这次真的覆盖 `crates/*` 独有的子树）

### 更早（plan 0401：SQLCipher 打开加密库）

**判据是两条**：用错误口令打不开库、`.db` 文件里搜不到明文密钥 —— 两条都做成了**具名测试**，
fixture 故意落在 `target/store-contract/`（不是 tempdir），因为"库里没有明文"是**安全声明**，
必须能拿一个真实文件手工 `grep` 复核。

- [x] 新建 `src-tauri/crates/akasha-store`：`open(path, passphrase)` = **空口令先拒**
      → `Connection::open` → `sqlite3_key()`（**打开之后的第一件事**，D4）→ 读一次
      `sqlite_master` 把"口令不对"逼到眼前 → 收紧到 0600（D12）
- [x] **实测把 ADR-0002 §7 的 7 项从"预期"变成"事实"**（结果表见上；全在
      `tests/sqlcipher_contract.rs` 里，上游换版本时它们该红）
- [x] **一处与 ADR 不符，按新流程回改了 ADR**：空 key 的机制不是"静默关掉加密"，而是
      `sqlite3_key` **返回 `SQLITE_ERROR` 且不挂 codec**，而连接**照常可用** ——
      D5 的危险点因此更尖锐（不看返回值就会写出明文库，且之后每步都"成功"）。
      已改 D5 并在 §10 记第一行
- [x] **`unsafe` 这一刀比预想的难切**：`unsafe_code = "forbid"` 下 `AGENTS.md` §3.4 承诺的
      "单点 allow + SAFETY 注释"**根本写不出来**（rustc 规定 forbid 不可被 allow 覆盖），
      而"只给一个 crate 放宽"也走不通（cargo 不允许部分覆盖继承来的 lint）。
      落地方案 = workspace 改 `deny` + 新增规则 `no-unsafe-outside-store` 把 `unsafe`
      圈回 `akasha-store`（`AGENTS.md` 单独提交，规则用一对探针验过）
- [x] **顺带发现 rusqlite 的版本不是我们能选的**：`victauri-plugin` 已依赖 `rusqlite ^0.32`，
      而 `libsqlite3-sys` 带 `links = "sqlite3"` —— 同一原生库只许一个版本，
      0.40.2 直接被拒。好处是 features 并集：全 app 只有一个 sqlite，且是 SQLCipher 那一支
- [x] 门禁：`just ready` **6/6**；`just test` **106 passed**；`cargo tree` 里是
      `openssl-src`（vendored）而不是系统 OpenSSL

### 更早（plan 0400：ADR-0002 进入实现中）

阶段 4 的第一项是"动存储代码之前先把数据文件格式定案"。
[`docs/adr/0002-secret-storage.md`](./adr/0002-secret-storage.md) 已写完并**进入「实现中」**；
plan 0400 归档。

- [x] ADR-0002 写完并**进入「实现中」**：12 条决定各带「决定 / 理由 / 否决的替代路」，
      总览表逐条标**出处**（`scope.md` §6 已定案的照抄、没定值的标"本 ADR 新增"）；
      `scope.md` 里"还没有值"的几项**都给了值**（KDF 参数、盐放哪、**一个库文件**承载四套池、
      **不用 WAL**、导出容器、版本字段、BW 缓存"同级"的含义）—— 这才是这份 ADR 存在的理由。
      上游事实**逐条核过**，不是凭印象
- [x] **顺手否掉一条要做的债**：plan 0403 前置检查里"把 `config.json` 并进 DB"——
      并进去之后关窗行为就要等解锁才知道（D11）
- [x] **改了一条流程**（用户裁定）：ADR 原先只有"接受 / 不接受"两态，而"接受"同时意味着
      "可以开工"与"不可再修改" —— 实现期一发现架构问题就无路可走。现为三态：
      **提议中 → 实现中（可改，每次记一行修订）→ 已定案（不可变，只能被新 ADR 取代）**。
      规则在 [`docs/adr/README.md`](./adr/README.md)，规范面在 `AGENTS.md` §8（单独提交）；
      `0001` / `0004` / `0005` 的状态词由「已接受」改为「已定案」（**只换词，内容未动**）
- [x] **修掉 5 处坏链**（`ROADMAP.md` 里三个 `./scope.md` 式的链接少写了 `docs/`；
      两份 `archive/` 文件里的 `../adr/` 应为 `../../adr/`）—— 用一次性脚本扫全部 markdown
      相对链接发现的。⚠️ **`just docs-check` 不查链接**（它只查配方与索引），
      所以这类坏链能一直全绿 —— 要不要把它做成门禁待定（先要想清楚"代码块里的链接示例
      算不算误报"）

### 更早（文档审计 · plan 0304 单实例）

- [x] **文档审计**（进阶段 4 之前逐条对照代码 / justfile / 目录核过一遍，不看措辞看事实）：
      删掉指向**并不存在**的 `.taurignore` 的规则；正文不再写"行数 / 配方数"这类一定会漂的数字；
      `logging.md` 的字段词汇表补上漏掉的一半（`label` 与 `window` 长期并存是同一件事两个名字）；
      `adr/README.md` 的队列标题与一条**不存在的路径**修掉；`scope.md` 里已落地的约束不再写作
      "计划中"。共同病根是**门禁查不了"这句话还成不成立"**（坑 #69）
- [x] **端口敲门降级为 `later`**：`scope.md` §2 / §3 曾把它列进 v1，而 `ROADMAP.md` 与
      `docs/plans/` 里**没有任何条目** —— 两处收回，`scope.md` §10 记一行（理由 + 日期）

### 更早（plan 0304：单实例）

- [x] 接入 `tauri-plugin-single-instance 2.4.4`，注册在**第一个插件位**（插件的 setup 在 `build()`
  里按注册顺序跑 —— 排在前面 = 第二个实例在别的插件的 setup 之前就退掉）
- [x] 唤起 = `unminimize()` → `show()` → `set_focus()`，**三步无条件都做**：最小化的窗口
  `is_visible()` 仍为真，按可见性分支会漏掉"还原"；反之都是空操作。于是"隐藏的窗口必须被显示"
  是**结构上**成立的（plan 0304 步骤 3）
- [x] 降级可见：Linux 注册前先问会话总线（`zbus`，**不是新增 crate** —— 同版本早在
  `tauri-plugin-opener` 的树里），连不上就不注册 + `warn` + probe `registered=false`；
  `warn` 留到 `.setup()` 里打（日志插件之前 `tracing` 没有 `log` 出口，坑 #47）
- [x] E2E `single_instance`：五层判据（自己退出码 0 / `activations` 加一 / **藏着的**窗口重新可见 /
  那之前的屏幕内容仍在 / 只有一个 app 进程）；`just test-e2e` 现在 **13 个用例**
- [x] 开发循环（plan 0304 步骤 4）：**没有出现互相顶掉**，因此不加 dev 专属 instance key
  （`dbus_id` 只影响 Linux，加了反而变成"只有 Linux 上能多开"）
- [x] 顺带修掉一处**规范与实现相反**：`scope.md` §5.4 曾要求 `prevent_exit()` 兜底，
  而那会把托盘菜单的"退出"一起拦掉（坑 #65）
- [ ] **未覆盖**：Windows / macOS 上的单实例未验；CI 的 Linux E2E 上这条用例跳过（xvfb 没有会话总线）

#### 再更早

- [x] **plan 0301 / 0302 / 0303 / 0305 / 0306 / 0201–0205 / 0107 / CI 去 Gitea 化 + 布局收口**
  （见 git 历史与各自的 `docs/plans/archive/`）

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。**仓库根没有 `Cargo.toml`** ——
  在根目录直接跑 `cargo …` 会失败（坑 #8），一律用 `just` 转发。⚠️ **临时脚本里也一样**：
  `cargo run` 的 cwd 必须是 `src-tauri/`。
- **四个 crate 的分工**：`akasha-core`（Session 模型 + 配置模型与判据，**零 Tauri 依赖**）、
  `akasha-pty`（`Transport` + portable-pty + 合批 + `teardown`（会话级回收）+ **`watchdog`**（进程外兜底））、
  **`akasha-store`**（SQLCipher 库的打开路径 + v1 的四张表 + 四套池的 CRUD —— 唯一允许 `unsafe`
  的地方；**app 依赖它**，但只用"落点与状态"两样）、
  `akasha`（app 包 = IPC 薄壳 + 托盘 + 配置载体 + 关窗语义 + 单实例 + 退出钩子 + 看门狗接线 + 事件 + 代码生成 bin）。
- **前端四层**：`src/ipc/`（唯一允许碰后端，含会话事件订阅）、`src/tabs/`（标签栏）、
  `src/terminal/`（xterm 面与会话接线）、`src/App.tsx`（标签模型 = 谁在、谁是活动的）。
- **调试白屏**：Victauri 的 `logs {action:"console"}`；读不到"模块执行期就抛错"的失败 ——
  那时临时往 `index.html` 塞 `window.onerror` 钩子（坑 #35）。⚠️ **React 的 effect 清理函数里抛错
  会卸载整棵树**（坑 #50）—— "界面突然全空"要先怀疑它，而不是先怀疑样式。
- **调试 E2E / 真 app**：`just test-e2e` 的 app 日志落在 `$tmp/akasha-e2e-app1.log` /
  `-app2.log`（**两段各一份**）；在**同一个 bash 调用**里才能同时读到 app 与测试（坑 #33）。
- **调试托盘**：它是原生的，webview 工具看不见 —— 走会话总线查
  `org.kde.StatusNotifierWatcher` 的注册表与 `com.canonical.dbusmenu` 的布局（坑 #62 / #63）。
- **调试"点叉之后怎么了"**：`app_state { probe: "lifecycle" }` 一次读出配置、托盘可用性与实际动作；
  日志里还有 `config loaded` / `config not found` / `config invalid`，以及降级时的
  `close behavior degraded`。
- **调试"第二个实例把话带到了吗"**：`app_state { probe: "single_instance" }` 的 `activations`
  （每被叫一次 +1）；日志里是 `single instance registered` / `single instance unavailable` /
  `second instance activated`。
- **文档三级粒度**：`ROADMAP.md`（判据）→ `docs/plans/TTxx-*`（手段）→ 本文件的坑（痕迹）。
  完成的 plan **整份移入 `docs/plans/archive/`**（不拼接、不追加）。
  规范之外的两份"展开"：`docs/logging.md`（日志形态）、`docs/just.md`（命令清单）。
- 命令入口分两处：项目级在根 `justfile`，crate 级在 `src-tauri/justfile`。
  **权威清单在 `docs/just.md` §2**，由 `just docs-check` 强制同步（正文不写配方数量 ——
  它是那种一定会漂的数字）。

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
    （⚠️ 备份别放 `/tmp`：沙箱每次调用一个私有 `/tmp`，下一次就找不到了）。
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
32. **`u64` 不能直接过 IPC**：改用壳层 `u32` 句柄 + **checked** 转换。
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
42. **`cargo test` 一次收多个 `--test` 时按目标名字母序跑**，不按参数顺序 ——
    要按顺序就得**逐个目标各跑一条**。
43. **`tauri-plugin-log` 默认的 `TargetKind::LogDir` 会让"日志目录不可写"变成 app 打不开**。
44. **`tracing/log-always` 会把依赖树的 TRACE 一起转成 `log` 记录**（含 `tracing::span::active`）。
    默认级别下 app 日志被刷到几十万行并明显拖慢 app —— 级别要显式定。
45. **SIGKILL 的投递是异步的**：`kill()` 返回后立刻读 `/proc/<pid>/stat` 会读到 `R`，
    那不是"没杀掉"。判据必须等"消失"（有截止时间的轮询）。
46. **portable-pty(unix) 的 `Child::kill()` 不是纯 SIGKILL**：它先发 **SIGHUP**、等 5×50 ms
    宽限，再退到 SIGKILL；而且它只管那个 shell —— 会话里其他进程得自己收（plan 0204）。
47. **早于日志插件注册的 `tracing` 事件会静默消失**：看门狗的"已启动"记录因此推迟到 `.setup()`。
    单实例的"注册不上"那条 `warn` 同理。
48. **`/proc/<pid>` 存在 ≠ 进程还活着**：僵尸（`Z`）也有目录项。判活要读
    `/proc/<pid>/stat` 的状态位；判自己的子进程结束要走 `wait`/`try_wait`。
49. **按"命令行里含某段文本"找进程会误伤**：沙箱包装进程（bwrap）自己的 cmdline 里带着
    整段脚本文本。要找探针就比对 **argv 恰好等于**那两个词。
50. **`term.dispose()`（xterm）会抛，而它跑在 React 的 effect 清理函数里** —— 触发状态是
    "WebGL 上下文丢过、已退到 canvas"的终端。后果是**双重的**：① 抛在 effect 清理里 = React
    **卸载整棵树** → 关一个标签页整个界面变空白；② 它若排在别的工作前面，后面的工作**永远执行不到**
    （实测：会话回收那一句被跳过）。两条正解：**清理函数里的"必做项"排在第三方拆解之前**，
    拆第三方时**兜住异常**。
51. **多标签之后 DOM 选择器不再唯一**：`document.querySelector('.xterm-helper-textarea')` 是"第一个
    标签页里的那个"。正解：输入与探针都跟着**活动面**走（`.tab-pane.is-active …`）。
52. **断言超时不一定是"慢"**：本轮 E2E 报的"标签页只剩一个 超时"，真实原因是**界面已经被卸载**
    （读到的是 0）。把"0 意味着界面没了"写进失败消息里，能省掉一整轮误判。
53. **`tauri-specta` 的事件必须 `mount_events`**：命令靠 `invoke_handler` 就够了，事件漏了这一步
    **不在启动时报错**，而是在**发**的时候 panic（`EventRegistry not found`）。
54. **官方 `Channel` 不会告诉你"流结束了"**：收到收尾帧 `{index, end:true}` 时它只把回调注销掉
    （`cleanupCallback`），**不通知** `onmessage` —— 想看"会话结束"必须另发一个事件。
55. **会话里还有别的进程握着 PTY 时，主端读不到 EOF**：这时会话**不算**结束、标签页**不该**关
    （真实终端同样如此）。E2E 里"敲 exit"必须在**干净**的标签页里做。
56. **接线一次的回调必须走 `ref`**：`attachTerminal` 只在挂载时接到回调（`useEffect(…, [])`），
    而调用方每次渲染都给一个新箭头函数（它闭包着**当时**的列表）—— 直接接会走进过期闭包，
    表现是"晚发生的会话事件处理错了"（本轮是"标签页关不掉"，且只在多标签时出现）。
57. **日志消息里的"括号解释"会自己长大**：`A：B（因为 C，见 plan D）` 这种写法一旦开了头，
    下一轮就往里加一句。同一个病根有三个样子：**括号里解释**、**在日志里引用文档**、
    **`?opt` / `?vec` 把 `Debug` 倒进字段**。正解是**消息只放事件名、变量进字段**
    （[`logging.md`](./logging.md)）—— 注意机器只拦得住"消息里有非 ASCII"这一半。
58. **`tauri-plugin-log` 默认 formatter 的时间戳只到秒**（`[日期][时间][target][级别]`），
    且级别在 target 之后。排查"谁先谁后"时不够用 —— 多进程（app + 看门狗）的日志顺序
    靠事件本身判断，别靠时间戳。
59. **`signal` 字段的值是本地化的**：portable-pty 0.9 的信号名取自 `libc::strsignal`
    （其 `src/lib.rs:215`，按 `LC_MESSAGES` 本地化 —— zh_CN 下 `SIGKILL` 写成 `已杀死`），
    而且**信号编号在它内部就被丢掉**，公开 API 拿不回 `SIGKILL`。已知修法：把 `nix`
    （**已经在依赖树里**）升为 `akasha-pty` 的直接依赖，用 `Signal::iterator()` + `as_str()` 反查。
    属于**要动依赖**的决定，尚未做。
60. **托盘在 Linux 上要写盘**：`tray-icon` 把图标 PNG 写到 `$XDG_RUNTIME_DIR/tray-icon/`
    （取不到 runtime dir 才退到 `$TEMP`）。只读 runtime dir / 容器里**建不起来** ——
    所以它只能是**可选能力**（降级 + 记日志），并且**降级之后不能把"关窗口"做成隐藏**：
    那样窗口再也叫不回来（这条直接决定了 0302 的 `tray_ready` 判据）。
    附带一条：`$XDG_RUNTIME_DIR` 里还放着 **wayland socket** —— 把它整个改到别处会让
    GTK 连不上合成器，想让它可写就得把 socket 一起链过去。
61. **`libayatana-appindicator3` 与老的 `libappindicator3` 都是运行时 dlopen**，顺序前者优先：
    只装老库的机器照样能起（**本机就是**：只有 2012 年的 `libappindicator 12.10.1`）。
    要验"支持的那条路"，把 libayatana 的包解到临时目录用 `LD_LIBRARY_PATH` 跑即可 ——
    **不改系统、不需要 root**。
62. **dbusmenu 的 item id 会随菜单重建而改变**：本仓库的托盘在会话表一变就重推菜单，实测
    id 从 12–15 变成 17–20（revision 4→5）。真实面板靠 `LayoutUpdated` 重读布局；
    **手写的测试客户端必须每次点击前重读布局**。拿旧 id 点会得到
    `The ID supplied N does not refer to a menu item we have` —— 这条报错**极易被误判成
    "库坏了"**（本轮就误判过一次，还为此白查了一遍 libdbusmenu 源码）。
63. **SNI 注册用的是唯一名**（`:1.x`），不是 `org.kde.StatusNotifierItem-<pid>-1`：
    "托盘到底注册上没有"要看宿主 watcher 的 `RegisteredStatusNotifierItems`
    （本机由 waybar 提供），**别 grep 总线名** —— grep 只会得到"没注册"的假结论。
64. **Victauri 的 `window` 工具能机器验证窗口状态**（`manage_action: close|show|hide`、
    `get_state` 里的 `visible`）—— "窗口藏起来了没有"不必靠人看，也不必截图。
65. **`AppHandle::exit()` 也会触发 `RunEvent::ExitRequested`**（tauri 的 `exit()` 文档与
    `tauri-runtime-wry` 的 `Message::RequestExit` 都写着）。所以在托盘模式下**不能**用
    `ExitRequested` + `prevent_exit` 来"拦住关窗导致的退出"—— 那会把托盘菜单的"退出"
    一起拦掉，app 变成关不掉。要拦就拦在**更早**的 `CloseRequested`（`prevent_close`，
    窗口根本不会被销毁）。剩下唯一会触发 `ExitRequested` 的窗口路径是 `destroy()` 强拆，
    而那时窗口已经没了、从托盘也叫不回来 —— 放行才是对的。
66. **便携数据目录就在 bin 同目录**（开发构建里 = `target/debug/akasha-data`），而 `just dev`
    与 `just test-e2e` **共用同一个 bin**：E2E 写进去的 `close_behavior=exit` 会**影响下一次
    `just dev`**（症状：点叉直接退出、数据落进 `target/`）。`just test-e2e` 因此**先备份、
    跑完还原**；排查时看启动那条 `config loaded` / `config not found` 的 `path=` —— 它写着
    这一次到底用了哪个文件。
67. **Victauri 的 REST 兜底接口返回的是 `{"result": …}` 包了一层**（MCP 的 content 包装），
    而 `victauri-test` 的 `call_tool` 会把它拆好交给你 —— 手写 `curl` 时别少剥一层，
    否则 `jq '.[0].visible'` 会得到 `Cannot index object with number`（本轮踩过）。
68. **`/proc/<pid>/exe` 可能带 ` (deleted)` 后缀**：cargo 重建时会拿一个**新的 hardlink**
    换掉 `target/debug/akasha`，于是**正在跑的那个进程**的 exe 指向一个已经不存在的路径 ——
    `canonicalize` 直接 `NotFound`。拿它做相等比较的结果是"一个 app 实例都找不到"
    （本轮 E2E 就这么红的，而且报错完全不提这件事）。正解：**比"父目录 + 文件名"，文件名先
    去掉 ` (deleted)`**；要判断"哪个进程是它"就别比完整路径。
69. **文档里的"事实"没人核对就会自己长大**：这一轮专门核了一遍，查出三处**都不是写代码时
    撞得到**的失真 —— `AGENTS.md` 要求用 `.taurignore`（**这个文件从未存在过**）；
    同一件事的数字在三处各不相同（配方数 `README.md` 20 / `AGENTS.md` 21 / `just --list` 22）；
    `logging.md` 的字段词汇表漏掉了一半在用的字段（于是 `label` 与 `window` 长期并存）。
    共同点：**门禁只查"索引对不对、命令在不在"，查不了"这句话还成不成立"** ——
    所以进新阶段之前要专门去核一遍，而**正文里不写数字**能消掉最常见的那一类。
70. **cargo 的 workspace lint 继承是"全有或全无"**：成员不能"继承 + 覆盖其中一条"
    （`cannot override workspace.lints`），而 rustc 又规定 **`forbid` 不能被 `#[allow]` 覆盖**
    —— 于是 `unsafe_code = "forbid"` 下 `AGENTS.md` 那句"确有必要时单点 allow"**永远写不出来**，
    "只给一个 crate 放宽"也走不通。正解：workspace 用 `deny`，再用一条 ast-grep 规则
    （`no-unsafe-outside-store`）把"唯一单点"圈回来 —— 两层加起来等价于 `forbid`，
    而且多一条"能改的口子"，比原来更诚实。
71. **带 `links = "..."` 的原生库在依赖树里只能有一个版本**：`libsqlite3-sys` 带
    `links = "sqlite3"`，而 `victauri-plugin` 已经钉了 `rusqlite ^0.32` —— 我们
    `cargo add rusqlite@0.40` 直接 `failed to select a version`，**报错只说"与另一个包冲突"，
    不说"是谁先占的"**。被别人的依赖定死版本时，先找占位者（`cargo tree -i <crate>` 对
    非本包的 crate 会说不匹配，要顺着报错里点名的那个包看）。
72. **SQLCipher 的空 key 不是"静默关掉加密"，而是"返回错误且不挂 codec"**：
    `sqlite3_key_v2` 在 `nKey == 0` 时直接 `return SQLITE_ERROR`（源码
    `libsqlite3-sys-0.30.1/sqlcipher/sqlite3.c:107794`），**连接随后照常可用** ——
    于是"只看返回值而不中断"的后果是写出一个**明文库**，而之后每一步都"成功"。
    ⚠️ `ATTACH … KEY ''` 是**另一条路径**，那里"不加密"正是本意（明文导出）。
73. **SQLite 自己建出来的库文件是 644**（umask 022），不是 0600：库文件与加密是两件事，
    权限要显式收紧；反过来也别写测试假设"默认就是 0600"。
74. **`PRAGMA cipher_settings` 的输出是一列 `pragma` 行**（每行 `PRAGMA kdf_iter = 256000;`），
    不是"参数名 / 值"两列 —— 照文档想象去 `query_row` 会一个字段都取不到。
    解析 pragma 结果时按"列名 + 行"通用处理，别硬编码形状。
75. **cargo-deny 的图根是"manifest 指向的那个包"，不是整个 workspace**（上游：
    "that crate will be the sole root … only other workspace members that are
    dependencies of that workspace crate will be included"）。本仓库 workspace root 同时是
    真实包 `akasha`，于是 `crates/*` 里**尚无人依赖的成员连同它独有的整棵子树都在图外** ——
    症状是**静默失效**：`deny.toml` 里 `[bans] deny = ["keyring"]` 照样报 `bans ok`。
    正解是给 cargo-deny 加 `--workspace`，而且它必须放在 `check` **之前**（顶层参数）。
    诊断手法：`cargo deny -L debug … | grep 'filtered'` 会列出所有被筛掉的包。
76. **"0 字节的库"不是"空库"，是"还没有密钥"**：SQLCipher 的盐与密钥校验值只在**第一次写页**
    时落盘，在那之前文件是 0 字节，而 `SELECT count(*) FROM sqlite_master` 在空文件上**照样成功**
    （没有东西可解，KDF 根本没跑，实测 ~0.19 ms vs 真实解锁 ~105 ms）。所以
    **"打开"与"新建"必须是两条路**：把创建藏在 `open` 里，等于在新建路径上不验证口令，
    而且会把这把错口令当成创建口令。写一句 `PRAGMA user_version = 1` 就能让文件实体化并钉住口令。
77. **`cipher_memory_security` 是进程级、单向的**：上游 `sqlcipher_set_mem_security` 的实现是
    `if(on) { … }`（设 `OFF` 既不报错也不生效），而 `sqlcipher_mem_security_on` 是静态变量
    （**不是**连接级）。它读回来的值还是 `on && executed` 的合取 —— 所以测试里只能断言
    "我们打开过之后读回来是 1"，**不能**断言"默认是 0"（同进程里前一个连接开过就是 1）。
    另外：它能排在 `sqlite3_key` **之前**（不读库），而排在前面才有意义（codec context 里
    那份口令副本才落在安全分配器上）。
78. **模块内的 `#[cfg(test)] mod tests` 也要自己 `allow(clippy::unwrap_used)`**：
    workspace 把它设成 warn 而 `just clippy` 带 `-D warnings`，集成测试文件顶部的
    `#![allow(...)]` **管不到** lib 里的测试模块（症状：`just ready` 红在 clippy、
    报的却是"test profile"里的 unwrap）。写法是在 `mod tests {` 之后紧跟一行 `#![allow(...)]`。
79. **`/proc/<pid>/mem` 的读用 `FOLL_FORCE`，绕过页保护** —— `mprotect(PROT_NONE)` 挡住的是
    "本进程里的常规访存"（越界读会 SIGSEGV），**挡不住**通过 `/proc/self/mem` 的读：
    实测那一页照样整页读出来（内容就是口令）。所以 `memsafe` 这一层别写成"内存里的密钥
    读不出来" —— 它挡的是**意外**（越界读、误格式化、core dump、swap、fork），
    挡不住"已经能在你进程里跑代码的人"。plan 0406 有一条测试**专门断言"读得出来"**，
    就是不让这句话日后被说大。
80. **`/proc/self/smaps` 的字段不是处处都有，而且属性行带缩进**：
    本机的 smaps **没有 `VmLck`**（`VmFlags` 有），验 `mlock` 得看 `/proc/self/status` 的
    **进程级** `VmLck`（造口令前后各取一次，0 → 4 kB）。另外 smaps 里段的属性行是
    `    VmFlags: …`（前导空格），`line.strip_prefix("VmFlags:")` **永远返回 `None`** ——
    解析前先 `trim_start()`。第一版测试就是这么"找不到自己那一页"的，而它报的是
    "期望 1 页、实际 0 页"，完全不提缩进。
81. **子串匹配会把规则变成笑话**：`no_absolute_paths` 的第一版用"列名里含 `dir`"判路径，
    于是 **`forwards.direction` 被判成了路径**。判据自己分不清"`direction` 与 `dir`"的后果很具体：
    下一个被它拦下的人第一反应是把整条检查删掉。正解是**按 `_` 分词、整词比较**，
    并且给规则配一对负例 —— 一个该命中（`key_path`）、一个**诱饵**（`direction` 必须不命中）。
    这条与 `AGENTS.md` §6 对 ast-grep 规则的要求是同一条：**没有负例的规则不算落地**。
82. **Victauri 的 `get_registry` 在本仓库是空的**（实测：`{"result":[]}`）：注册表只收录
    **标了 `#[inspectable]`** 的命令，而本仓库六个命令一个都没标 —— 于是 `AGENTS.md` §7 里
    "新 command 应在 `get_registry` 中可见"这条**当前无法满足**，`detect_ghost_commands` 的
    `confirmed_ghosts` 也证明不了什么（它自己的 `reliability` 是 **low**，"没调用过"与"没有幽灵"
    是两回事）。**可用的替代证据**是真路径上的 `invoke_command` 成功。要真正满足它，得先给命令
    加 `#[inspectable]`（那是一次单独的改动，别混在功能里）。
    ⚠️ 沙箱里问 app 只能走 **REST 兜底**（`POST /api/tools/<tool>` + `<tmp>/victauri/<pid>/token`），
    因为 MCP 连不到**另一个 bash 命名空间**里的 app（坑 #33）—— 于是"起 app + 问 app + 收 app"
    必须在**同一次** bash 调用里。
83. **clippy 会把"两个常量比较"的断言判红**（`assertions_on_constants`），正解是搬进 `const` 块：
    `const { assert!(MAX_PEM_LEN <= 65536, "…") }`。搬到那里反而更好 —— "取值依据"从"某天有人跑测试
    才发现"升级成**编译不过**。注意 `const` 块里的消息只能是字面量（不能用 `{}` 带值进去）。
84. **E2E 配方两段可能跑在**不同**的数据目录里**：便携目录（bin 同目录的 `akasha-data/`）不存在时
    app 按 `portable.md` §4 退回 OS 数据目录，于是第二段写进便携目录的 `close_behavior` **读不到**，
    而第一段还可能读到 OS 目录里**上一次**留下的配置（"关窗即退出"）→ `window_close` 莫名其妙地红。
    正解：两段都先 `mkdir -p` 便携目录（配方里两处注释都写了理由）。
85. **集成测试的共用脚手架放 `tests/common/mod.rs`，但必须自己 `#![allow(dead_code)]`**：
    `cargo` 只把 `tests/*.rs` 当测试目标，`common/mod.rs` 是被各目标 `mod common;` 引进去的普通模块
    —— 每个目标只用到其中一部分函数，用不到的那些在 `-D warnings` 下会直接让门禁红。
    （同一类：`let rows = …; rows` 会被 clippy 判 `let_and_return`。）
86. **`pkill -f <pattern>` 会匹配到你自己那条命令行**：命令里含有那个模式（例如
    `pkill -f "node_modules/.bin/vite"`），于是脚本在收尾阶段**把自己杀掉**，退出码 143，
    看起来像"app 崩了"。要么用记录的 pid，要么先 `ps` 核对，要么用 `pkill -x <进程名>`。
