# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-12

## 一句话

**阶段 2「端到端最小终端」5/5 完成**；**阶段 3「托盘与应用生命周期」6/6 完成**；
**阶段 4「存储与凭据池」9/9 完成** —— 出口的两件事都到了：**可搬迁性验证有配方**
（plan 0405：`just portable` 把"搬走整个文件夹"跑成自动化判据），
以及 **ADR-0002 转「已定案」**（落地它的 plan 0401–0405 全部归档，本文从此不可改）。

本轮另做了一遍**文档一致性与正确性核查**（改掉若干过时说法，逐条见下），
并把 **`unsafe` 的注释规范**按 Linux 内核的做法写成规则 + 三条 clippy lint。

已落地八块：**SQLCipher 加密库能开**（plan 0401）、**口令只从一条路进来，而且能真的验证它**
（0402 —— 拆开了"打开"与"新建"，原来在没有文件的路径上**任何口令都能开**）、
**口令在内存里也受保护**（0406）、**四套池能增删改查**（0403 —— v1 的四张表、不变量写在库自己身上、
私钥读出来进受保护页）、**库里的东西拿得出去也放得回来**（0404）、
**解锁与锁定是一条完整的生命周期**（0407 —— 真 app 上实测 `VmLck` **0 → 192 → 0 kB**）、
**搬走文件夹之后数据还在且能用**（0405 —— 四套池 1/1/1/1，库侧逐项比对内容）、
**便携目录不可写时拒绝启动**（0405 —— 一条 `error` + 退出码 2，不再静默退回 OS 目录）。

**下一步是阶段 5（SSH 栈）的 plan 0501**：让 ADR-0003（`russh` + 资源模型）**进入「实现中」** ——
在那之前不动 `akasha-ssh` 的任何一行业务代码（`ROADMAP.md` 阶段 5 第一条）。

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
⚠️ **便携目录写不进去是另一回事**：那是用户明确要了便携却做不到，**拒绝启动**（退出码 2），
不是降级 —— 详见下面「可搬迁性」。

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

**可搬迁性（plan 0405）**：一条数据目录规则 + 一条配方。

| 情形 | 行为 |
|---|---|
| bin 同目录有 `akasha-data/` | 用它（便携）；库 `akasha.db` 与 `config.json` 都在里面 |
| 没有那个目录 | 退回 OS 数据目录（**那不算"要便携"**，所以写不进去也只降级） |
| 有那个目录但**写不进去** | **拒绝启动**：一条 `error`（OS 原话）+ **退出码 2**，在窗口与托盘之前 |

判"可写"的方式是**真的写一个探针文件再删掉**（`.akasha-writable`）—— mode 位看不出 ACL /
只读挂载 / squashfs。配方 `just portable` 自动跑完 `portable.md` §5 的五步（复制 bin → A 起 →
完全退出 → 搬成 B → B 起 → 断言四套池 **1/1/1/1** + 库侧逐项比对内容）。

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
| `just test` | **186 tests run: 186 passed**（`akasha` 58 + `akasha-pty` 37 + `akasha-core` 15 + **`akasha-store` 76**）。⚠️ `akasha` 那 58 条里含 `tests/portable.rs` 的 **3 条**：没有 `VICTAURI_E2E` 时它们只打印原因并返回（`just test` 里不跑真 app） |
| ↑ 本轮新增 | **6 条**：`config` 3 条（便携目录判定 / 探针可写正负对照）+ `tests/portable.rs` 3 条 |
| ↑ **搬走整个文件夹之后，数据还在且能用**（plan 0405 判据） | ✅ 配方 `just portable`：`A 的 vault_status = {path: …-a/akasha-data/akasha.db, state: missing}` → 灌四套池 → `mv` → `B 的 vault_status = {path: …-b/…, state: present}` → **`vault_unlock` 读回 `{keys:1,hosts:1,serials:1,forwards:1}`**；库侧再用同一口令打开逐项比对**内容**（只对行数不够） |
| ↑ **"app 挑的是哪个数据目录"不靠日志反推** | ✅ 两条独立观察：`lifecycle` probe 里的 `close_behavior`（读到跟着搬走的 `config.json` = `exit`）+ `vault_status` 报的绝对路径分别在 A / B 里 |
| ↑ **便携目录不可写 → 拒绝启动**（`portable.md` §4 第 3 条） | ✅ 退出码 **2** + `portable data dir not writable err=not writable: 权限不够 (os error 13) path=…/akasha-data`；**落地前的实测是**：app 照常启动、日志只有一句 `config not found`（把"写不进去"说成了"没有配置文件"） |
| ↑ ↑ **另一半：没有便携目录时不许拒绝**（正对照） | ✅ 同一个二进制、布局里不放 `akasha-data/` → 照常起来。少了这条，上面那条分不清"检查在工作"与"检查把谁都拒了" |
| ↑ **`chmod 500` 那个前提本身有正对照** | ✅ 用例先自己试写一次：写不进去才继续（以 root 跑 / 不理会 mode 位的文件系统 → **显式跳过并打印原因**） |
| ↑ **`cipher_memory_security` 的擦零是实测的**（plan 0407） | ✅ 派生密钥用 `openssl` CLI 按 SQLCipher 4 默认参数**独立算出来**：连接活着时 2 处、drop 之后**一处不剩**。机制可逐行核对：`sqlcipher_mem_malloc/free` 每次 `mlock`、释放先擦零再 `munlock` |
| ↑ **推翻了"SQLCipher 内部留一份口令"**（plan 0407） | ✅ 解锁期间口令**只**多出一处（受保护页）—— 它手里只有派生密钥。ADR-0002 §7.5 的副本清单因此少了一行 |
| ↑ **锁定之后进程内存里一处机密都不多**（plan 0407） | ✅ 把**整个进程内存**（匿名段，含 `---p`）扫一遍找那两串字节：口令 `1 → 2 → 2 → 1` 处、派生密钥 `3 → 1` 处。**带正对照**（解锁期间必须比基线多扫到，坑 #87/#90/#91） |
| ↑ **`VmLck` 走一个来回**（plan 0407） | ✅ 库层 `0 → 4 → 152 → 4 → 0 kB`；**真 app 上 `0 → 176～192 → 0 kB`**（测试进程自己读 `/proc/<pid>/status`，不是 app 自报） |
| ↑ **导出与还原**（plan 0404） | 加密导出可在另一目录还原（逐字段一致）；明文导出两道门槛（逐字短语 / 文件名自曝 / 独立口令）；不泄密带对照组（**0 命中 vs 1 命中**）；失败不留半成品；导出件 **600** |
| `just portable`（可搬迁性；自己起 app） | 退出码 **0**，**3 passed**（搬家 / 不可写拒绝 / 没有便携目录也不拒）。⚠️ 不能与别的 akasha 同时跑（单实例）—— 配方先查一遍并说清该关掉什么 |
| `just test-e2e`（自包含：起 Vite + app → **三段** → 收尾） | 退出码 **0**：**15 个 E2E 用例**（`window_close` 内部**显式跳过**：这台机器 `tray_ready=false`）+ **第三段 3 条**（`portable`）。复用一个开发者的 app 时第三段**跳过并打印原因** |
| ↑ **§7 的 registry 那一条：当前不可满足**（实测，见坑 #82） | `get_registry` 回 **`[]`** —— 本仓库的命令**都没有 `#[inspectable]`**；`detect_ghost_commands` 的 `reliability` 是 **low**。**替代证据**是真路径上的 `invoke_command` 成功 |
| ↑ 单实例（0304，Linux 实测） | ✅ probe `{"registered":true}`；隐藏之后再来一个实例 → **204.8 ms** 后 `exit=0`、`activations=1`、`visible=true` |
| ↑ 关标签页 / 敲 `exit`（0305 / 0306，真 UI 点击） | ✅ 关标签页 → 探针 A 在 **79 ms** 内消失、探针 B 不受影响；敲 `exit` → 标签页自己关掉（app 仍在） |
| ↑ 退出零残留（0204/0205） | ✅ `app 已退出（pid 2881）`；`✅ 零残留：忽略 SIGHUP 的 3638 已随会话被收掉` |
| ↑ 终端 / 会话判据（未退化） | `renderer = webgl`；8 MB 灌流后仍可交互；raw 通道 10.73 MB / 164 批；收尾帧 1 个、console 零异常 |
| `pnpm build`（tsc + vite build） | 退出码 0；产物 **843 kB / gzip 231 kB**（本轮未改前端，数字沿用） |
| `just deny-offline` | `bans ok, licenses ok, sources ok`（`--workspace` 已加，补上了"未被人依赖的成员不在图里"那个盲区） |
| `just docs-check` | 全过（ROADMAP 条目在 3 行内且无代码块 / plan ≤200 行且索引一致） |
| `ast-grep scan` | 退出码 **0**；**六条**规则均已用正负例验证（本轮改了 `no-unsafe-outside-store` 的 note，`files:` / `ignores:` 没动，所以不必重跑负例；此前被 `no-println` 拦了一次，见坑 #94） |
| **三条 unsafe 注释 lint**（本轮新增的强制，`just lint` 的 clippy 那一步） | 退出码 **0**；三条各用一个探针证明**它们真的会红**：`undocumented_unsafe_blocks` → 把 `apply_key` 的 `// SAFETY:` 改名即报（**私有函数也报**）；`unnecessary_safety_comment` → 在安全语句上挂一条 `// SAFETY:` 即报；`unnecessary_safety_doc` → 给安全函数加 `/// # Safety` 即报。探针跑完即撤，仓库里不留 |
| **文档一致性与正确性核查**（本轮：核对了 17 份文档 —— 规范 1 + 顶层 2 + `docs/` 7 + ADR 5 + plan 索引与在办 plan 2，逐处改掉过时说法） | ✅ 相对链接 **231 条全部可解析**；`cargo nextest list --workspace` 逐 crate 计数与本文的 **58 / 37 / 15 / 76 = 186** 一致；`unsafe` **3 处**（库 1 + 它的契约测试 2，都带 `// SAFETY:`）；`BatchPolicy::DEFAULT` = 64 KiB + 16 ms、`MAX_LEN` = 256、`MAX_JUMP_DEPTH` = 32、私钥页 16384 字节逐条对上代码 |
| `cargo tree -p akasha-core` \| `grep -c tauri` | **0**（分层成立） |
| `just bench`（criterion） | 52.7 GiB/s / 14.1 ns 每批 / 9.64 GiB/s（**0201 的数字，本轮未复跑**） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。

## 待验证（本地跑不了 / 沙箱跑不了）

- **"便携目录不可写"没有 GUI 报错面**：今天的形态是**一条 `error` + 退出码 2**
  （机器可查、也是 E2E 的判据）。从桌面菜单启动的用户**看不到 stdout** —— 要弹窗得引
  `tauri-plugin-dialog`（新依赖 + 能力权限），属于真实 UI 阶段。⚠️ 别把"有日志了"说成
  "用户会知道"。
- **拒绝启动那条路径只在 Linux 上真跑过**：`chmod` 造前提在 Windows 上会**显式跳过**
  （macOS / Windows 的覆盖交给 CI 矩阵，而 CI 至今没跑过）；两个平台的"只读目录"语义不同
  （ACL / 挂载），别按 unix 的直觉推断。
- **复用别人 app 时第三段（可搬迁性）会跳过**：单实例（0304）会让它起的第二份自己退掉。
  想验就得单独跑 `just portable`（先把 app 关掉）—— 配方会打印这条原因。
- **解锁 / 锁定还没有界面**：命令、状态、生命周期都在了（plan 0407），但界面上**没有能输口令的地方**
  （设计稿未定，`AGENTS.md` §4.0）。今天的真路径证据来自 E2E 的 `invoke_command`。
- **口令经 IPC 的两份副本够不着**：tauri 的请求体缓冲与 `serde_json::Value` 在这一次调用之后被
  free 而**不擦**（ADR-0002 §7.5 的边界）。我们这一份（`PassphraseInput` → `Passphrase::new`）
  被擦零。更紧的做法是 raw body，代价是前端手写裸 `invoke` —— 记为后续。
- **`mlock` 失败那条路只有单测**：本机 `RLIMIT_MEMLOCK` 是 8 MB，造不出真的失败。
- **内存扫描只在 Linux、只扫匿名段**：`/proc/self/maps|mem` 的读走 `FOLL_FORCE`（Linux 专有）；
  派生密钥那条用例在 `openssl` CLI 缺席时**显式跳过并打印原因**。
- **导出与还原还没有 IPC 命令**：库层的函数与契约测试都在（plan 0404），
  但"导出成哪个文件、确认短语怎么问"要等文件选择器与确认界面。
- **权限位（0600）只在 unix 上有判据**：Windows 没有等价物（D12 如实记为不做）。
- **导出件在跨机器 / 跨文件系统上的表现未验**：`rename` 的原子性只在同一文件系统内成立。
- **单实例在 Windows / macOS 上未验**：机制完全不同（命名 mutex / `/tmp` 下的 unix socket），
  CI 的**类型检查挡不住运行期差异**。
- **CI 的 Linux E2E 上几条必然跳过**：`single_instance` 与 `window_close` 都要会话总线 + 可写
  `$XDG_RUNTIME_DIR`，xvfb 两样都没有（用例**显式跳过并打印判据**）。
- **托盘图标在面板里"看得见"**：机器只能证明"注册进了 watcher"（`scope.md` §5.5）。
- **CI 三个 job 是否真能变绿** —— 仓库还没有 remote，从没跑过。`just portable` 现在**挂在
  `just test-e2e` 的第三段**里，所以三平台格子会自动带上它 —— 但那是"应该会跑"，不是"跑过了"。
- **`tauri dev` 重载那条路径没有门禁**：只能手动实测（plan 0205 的实施记录里有脚本与输出）。
- **前端类型检查不在任何门禁里**：`just ready` 只覆盖 Rust + 文档，`pnpm build`（tsc）要手动跑。
- **在"有后台作业握着 PTY"的标签页里敲 `exit`**：不会有 EOF、不会关标签页（刻意，坑 #55），
  但没有用例守着它。
- **宿主 MCP 连不到沙箱内运行的 app**（私有 PID / 临时目录；坑 #33）。
- **`just dev-web` 的模拟后端没在真浏览器里点过**（本环境没有浏览器）。
- **大流量下的 JS heap 数字没取**。
- **三条 unsafe 注释 lint 的行为随 clippy 版本变**：本机 1.98 实测**也查私有项**，
  所以**没有**照抄内核的 `check-private-items`（理由见坑 #96）。若升级后私有项不再被查，
  表现是**静默失效** —— 那时才需要补一个 `clippy.toml`。

## 当前基线（2026-09-12 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` / **`akasha-store`** |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，坑 #29） |
| 后端模块 | `bindings` / `session` / `tray` / `config`（载体 + **数据目录 + 便携目录的可写性检查**）/ `lifecycle` / `single_instance` / **`vault`（库的落点、状态与解锁生命周期）** / `watchdog` |
| **关窗语义** | 判据 = `akasha-core::CloseAction::decide(close_behavior, tray_ready)`；app 侧 `CloseRequested` → **先 `hide()`、成功才 `prevent_close()`**。**不挂 `RunEvent::ExitRequested`**（理由见 `lib.rs` 注释与坑 #65） |
| **单实例** | 插件注册在**第一个插件位**；唤起 = `unminimize()` → `show()` → `set_focus()` **三步无条件都做**；`available()` 在 Linux 上 = 会话总线连得上 |
| **配置** | `<数据目录>/config.json`，`{"close_behavior":"tray"\|"exit"}`；`serde_json` + `deny_unknown_fields`；**只读不写**；在 `.setup()` 里读一次 |
| **数据目录** | bin 同目录存在 `akasha-data/` → 用它（便携）；否则 `app_data_dir()`。**不自动创建**。开发构建里便携目录 = `src-tauri/target/debug/akasha-data` |
| **可搬迁性（新）** | 数据目录推导的**唯一落点**在 `config.rs`：`portable_dir()` 一个表达式同时喂 `data_dir()`（退回逻辑）与 `portable_data_dir()`（**可写性检查只针对便携这条**）。`require_writable()` 用探针文件 `.akasha-writable` 判定可写、随后删掉；不可写 → `.setup()` 开头 `error!` + **`exit(2)`**（在窗口与托盘之前，所以拒绝是"什么都没发生"地退出）。配方 `just portable`：复制 bin 进临时布局 → A 起 → `mv` 成 B → B 起 → 四套池 1/1/1/1 + 库侧比对内容 |
| **系统托盘** | 图标 = `bundle.icon` 那张；Linux 上落盘到 `$XDG_RUNTIME_DIR/tray-icon/…`；菜单 id 是稳定字面量（`window.toggle` / `tunnels` / `tunnels.empty` / `app.quit`）；**会话表一变整份重建** |
| **probe** | `lifecycle` → `{"close_behavior":…,"tray_ready":…,"close_action":…}`，**没登记时是 `{"initialized":false}`**（它比 Victauri 自己的发现目录晚，见坑 #93）；`single_instance` → `{"registered":bool,"activations":n}`。**库没有 probe**：状态本身就是命令（`vault_status`）。⚠️ 也**不靠探针自报**来验"锁定时内存还回去了"：那条判据由测试进程自己读 `/proc/<pid>/status` |
| 出字节路径 | PTY read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write` |
| 前端结构与布局 | `src/tabs/TabStrip.tsx` + `src/App.tsx`（多面**同时挂载**、非活动的 `visibility: hidden` 叠放）+ `src/terminal/` + `src/ipc/` |
| **关闭一个标签页** | 移除 → React 卸载该面 → `attachTerminal` 清理（**先** `close_session`，**后** `surface.dispose()`）→ `Sessions::close` → `Transport::shutdown()` → 撤销看门狗登记。**实测 79–93 ms** |
| **会话自己结束** | 合批读循环结束（EOF / EIO）→ 收工 → 收尾线程 `Sessions::retire`（收尸 + 摘牌 + `registry.close` + `forget`）→ `app.emit("session_ended", …)` → 前端关掉那个标签页 |
| 回收路径（进程内 / 进程外） | `RunEvent::Exit` / panic hook / `close_session` / `retire` / 托盘退出；另有一个**进程外**看门狗读管道（`+pid` / `-pid`，EOF = app 死了） |
| 前端渲染器 | **WebGL**（WebKitGTK + MESA 软件栈下仍拿到 WebGL2）；`canvas` 元素 2 块 |
| 大输出实测 | 10.36–11.28 MB / 159–170 批（0202–0305 各轮） |
| 前端产物 | 843 kB（gzip 231 kB） |
| **存储层** | `akasha-store`：**全仓库唯一允许出现 `unsafe` 的 crate**（全库 **3 处**：生产 1 处把口令送进 `sqlite3_key()`，加契约测试 2 处直接调 `sqlite3_key` / `sqlite3_rekey` 钉上游语义；都带 `// SAFETY:`，ADR-0002 D4），由 workspace 的 `unsafe_code = "deny"` + `.ast-grep/rules/no-unsafe-outside-store.yml` 两层守。**两条路**：`create(path, &Passphrase)` = 有内容就 `VaultExists`（永不覆盖）→ 送密钥 → **一次事务里建 v1 的四张表 + 写 `user_version = 1`**；`open(path, &Passphrase)` = 不存在的/0 字节就 `NoVault` → 送密钥 → 开外键 → 读一次 `sqlite_master` 逼口令错暴露 → **校验版本号 + 四张表都在**。两条路都收 0600 |
| **unsafe 的注释** | 写法 = Linux 内核规范（`AGENTS.md` §3.4）：`// SAFETY:` 说"**为什么 sound**"、`/// # Safety` 说"调用方 / 实现方要守什么契约"，两件事不许互相替代；由 clippy 的 `undocumented_unsafe_blocks` / `unnecessary_safety_comment` / `unnecessary_safety_doc` 三条强制（跑在 `just lint` 里）。⚠️ 它们**也查私有项** —— 唯一那处生产 `unsafe` 就在私有函数 `apply_key` 里（坑 #96） |
| **四套池** | `keys` / `hosts` / `serials` / `forwards`，各 5 个函数 + 反查（`hosts_using_key` / `hosts_jumping_to` / `forwards_of_host`）。`New*`（没有 id）与 `*`（有 id）**是两种类型**；不变量写在库上（`STRICT` + `CHECK` + 外键 `RESTRICT`，D14）。⚠️ 跳板链的成环**库表达不了**：`update` 时逐跳走链挡住（`MAX_JUMP_DEPTH` = 32） |
| **私钥** | 池里存 BLOB，**出库直接进受保护页**（`PrivateKey` = `memsafe::Secret<[u8; 16384]>`）；`expose()` 返回**提权窗口**。空私钥与**超过一页**在**构造层**就被拒 |
| **口令** | `Passphrase` = 口令在进程里的唯一形态：空值**造不出来**、**没有 `Debug`**、无 `Display`/`Serialize`、**不实现 `Clone`**；本体住在 `memsafe::Secret` 的一整页**受保护内存**里（`mlock` + 静止态 `PROT_NONE` + `dd` + `wf`） |
| **导出与还原** | `to_encrypted(source, 源口令, dest, 导出口令)`：两把口令相同 → `SharedPassphrase`；`to_plaintext(source, dest, PlaintextAck)`：文件名必须含 `plain`，否则**什么都不写**；`restore*` = **替换**，只写进空的库槽。三段式：预建 **0600** 的 `…partial` → `ATTACH … KEY ?`（**口令走绑定参数**）→ `sqlcipher_export` → **显式写 `user_version = 1`** → `DETACH` → **原子改名** → 再用真读者 `open` 自检 |
| **dump** | `dump::dump(&conn)` → `Dump { format_version, keys, hosts, serials, forwards }` + `row_counts()` + `to_text()`。**结构上不含私钥** |
| **库文件的磁盘事实** | `akasha.db`；建库后 **36864 字节 = 9 页**；SQLCipher 4.5.7 + vendored OpenSSL 3.6.3 + 内嵌 SQLite **3.46**；`user_version = 1` 是格式权威（**含 0** 一律拒绝）**且要四张表都在**；盐 16 字节随机、就在文件头前 16 字节；显式收紧到 **600**；不带 `-wal` / `-shm`；解锁代价 **~105 ms**（KDF）。`cipher_memory_security` 是**进程级、只能开不能关** |
| **解锁与锁定** | app 侧 `Vault { unlocked: Mutex<Option<Unlocked>> }`，`Unlocked { conn, passphrase }` —— **同生共死**。`vault_unlock`：`Missing`/`Empty` → 建、`Present` → 开，返回四套池行数（`u32`）；已解开 → `AlreadyUnlocked`（**不替换**）。**只有显式锁**：关窗/退出不锁、无空闲超时 |
| **口令经 IPC 进来的形态** | `PassphraseInput`（newtype，`specta(transparent)` → TS `string`）：**没有 `Debug`/`Clone`**，唯一出路是 `into_bytes()`。⚠️ tauri 自己那两份够不着（ADR-0002 §7.5） |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| CSP | `default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:`；`devCsp` 多一个 `ws://localhost:1420 http://localhost:1420` |
| capabilities | 仍只有 `core:default` + `opener:default`（+测试用的 `victauri`）。**托盘、配置、单实例、便携目录检查都没有加任何 permission**（全在 Rust 侧）；库的三条命令是**我们自己的 command**，不需要 ACL permission |
| 前提条件 | **需要能写 `$HOME`**；托盘另需能写 `$XDG_RUNTIME_DIR`、单实例另需会话总线（否则各自只降级、不影响启动）。**便携目录存在时另需可写 —— 不可写是拒绝启动**（这是唯一一条"数据目录写不了就起不来"的路径，且只在用户明确要便携时成立） |

## 进行中 / 下一步

- [ ] **下一步 = 阶段 5 的 plan 0501**（[ADR-0003 进入实现中](./plans/0501-adr-0003-ssh-stack.md)）：
  动 `akasha-ssh` **之前**先把线协议与资源模型定下来（`russh` 版本结论要有可核对的依据）。
- [ ] **阶段 3 收口后的两条复核**（托盘时代带来的前提变化，都还没做）：
  - 0205 的看门狗生命周期仍然 = 一个 app 实例（ADR-0005 §6 的复审条件之一）；
  - 0305/0306 的前提 ② "关最后一个标签页 = 空状态"在"窗口隐藏"成为常态之后是否仍然合适。
- [~] **plan 0102（CI 平台矩阵）**：本地部分完成，最终判据 = **推上去三个 job 全绿**，卡在没有 remote
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：本地已实测，剩 CI 三平台格子
- [ ] **正式 UI**：等设计稿（见上面「UI 现状」）—— 没有验收标准，故**不进 ROADMAP**

### 本轮完成（文档一致性与正确性核查 + unsafe 注释按 Linux 内核规范强制）

**这一轮不动功能代码**：做的是"文档说的与代码做的是不是同一件事"，外加把 unsafe 的注释
规范从一句话（"要有 `// SAFETY:`"）变成**可按内核做法执行的规定**。

- [x] **核查方式**：代码侧一律走结构性工具 —— rust-analyzer MCP 取符号
  （`config.rs` 的 `portable_dir` / `portable_data_dir` / `require_writable` / `EXIT_NOT_WRITABLE`
  等逐一对上）、ast-grep 结构搜 `unsafe`（3 处全带 `// SAFETY:`）、`cargo nextest list` 取计数；
  **没有**用全仓库 grep 去翻代码
- [x] **过时说法按类型改掉**：① ADR-0002 已定案，而 `scope.md` 与 `docs/adr/README.md`
  还写着"实现中"；② `portable.md` / `scope.md` 提到一个**从未存在**的"便携标记文件"
  （代码里明写"目录本身就是标记"，是"或 X"自己长出来的典型）；③ `docs/just.md` 的
  `just test-e2e` 还写着"自起时跑两段"（plan 0405 之后是**三段**），guard 清单也漏了
  `E2E_SELF_APP`；④ 规则 note 与两处代码注释说"全仓库只有一处 unsafe"（实为 3 处）；
  ⑤ `logging.md` 的 `path` 只说"配置文件路径"（0405 之后它也是便携数据目录）；
  ⑥ plan 0501 的「关联」引错了 ROADMAP 原文
- [x] **刻意保留的**：ADR-0001 里 `unsafe_code = "forbid"` 那个片段、ROADMAP 里已勾选的
  旧条目 —— 都是**被取代的历史**（各自头部已写"已被 0004 取代"），不算错，也不该改
- [x] **unsafe 注释成文**（`AGENTS.md` §3.4，单独提交）：`// SAFETY:`（说明**为什么 sound**，
  紧贴每个 unsafe 块）与 `/// # Safety`（给调用方 / 实现方的**契约**）分清、不许互相替代，
  **标签**一律大写（clippy 认的就是那两个字面量），**解释写仓库的注释语言（中文）** ——
  依据是内核 `Documentation/rust/coding-guidelines.rst`；⚠️ 它那套"英文、句首大写"是
  内核自己的语种约定，不属于 unsafe 规范（第一版搬多了，见坑 #100）
- [x] **强制**：`just lint` 的 clippy 那一步加三条（内核 Makefile 里就是这三条），
  负例见上表（"探针一改名就红"）
- [x] 门禁：`just ready` **6/6**（lint 3s / test 58s —— 因 `Cargo.toml` 变了而全量重编）

### 上一轮完成（plan 0405：可搬迁性验证 + ADR-0002 定案）

**判据是 ROADMAP 那一句**：移动整个文件夹后重启，**原有主机 / 密钥 / 规则都在**
（只验证"能开"不算过）。

- [x] **先量再写**：量出"今天不可写也照样启动、而且日志把'写不进去'说成'没有配置文件'"
  （§4 第 3 条根本没实现）；也量出**复制一份 debug 二进制到别处照样跑得起来** ——
  五步因此能自动化（被测的是"bin 在哪"，不是"构建目录在哪"）
- [x] **配方 `just portable`**：自己起 app（复制 bin 进临时布局、A/B 两次），
  先用 `lifecycle` probe + `vault_status` 证明 app 挑的是那个目录，再用 `vault_unlock`
  读回四套池 **1/1/1/1**，最后库侧逐项比对**内容**；`just test-e2e` 的**第三段**调用它
  （E2E 入口仍然只有一处，CI 三个平台顺带覆盖）
- [x] **§4 第 3 条落地**：便携目录不可写 → `.setup()` 里 `error!` + **退出码 2**，在窗口与托盘之前；
  判可写用**探针文件**（mode 位看不出 ACL / 只读挂载 / squashfs）
- [x] **正对照成对**：没有便携目录时**不许**拒绝；`chmod` 那个前提自己先试写一次
  （造不出来就显式跳过并打印原因）；`require_writable` 的负例改用**结构性**造法（ENOTDIR）
- [x] **ADR-0002 转「已定案」**：plan 0401–0405 全部归档 → 状态翻转、§10 记两行
  （D12 的落地形态 + 状态翻转本身），§10 从此只读；D12 那句"尚未实现"**必须**在定案前改掉 ——
  定案之后改不动，那份 ADR 会永久地说一件不成立的事
- [x] 门禁：`just ready` **6/6**；`just test` **186 passed**（`akasha` 52 → **58**）；
  `just test-e2e` **退出码 0**（15 个 E2E 用例 + 第三段 3 条）；`just portable` **3 passed**

### 更早（各 plan 的细节在 `docs/plans/archive/` 里，这里只留结果）

- [x] **plan 0407**：解锁与锁定的生命周期 —— 四个问题各一个结论；先量再写（
  `VmLck` 0→4→152→4→0、口令/派生密钥的副本数）；`just test-e2e` 新增 `vault_unlock`；
  细节在 [archive/0407](./plans/archive/0407-unlock-lifecycle.md)
- [x] **plan 0404**：dump 与导出 / 还原；细节在 [archive/0404](./plans/archive/0404-dump-export.md)
- [x] **plan 0403**：v1 的四张表 + 四套池 CRUD；P2 从散文变成两条判据；私钥按 D13 判据表重验
- [x] **plan 0406**：口令进 `memsafe` 的受保护页 —— 把上游四条承诺变成断言，并**同时钉住边界**
- [x] **plan 0402**：口令 → KDF → 库密钥；**拆开 `open` 与 `create`**；`deny.toml` 常驻禁令
- [x] **plan 0401**：SQLCipher 打开路径 + ADR-0002 §7 的实测清单
- [x] **plan 0400**：ADR-0002 写完并进入「实现中」；ADR 三态
- [x] **plan 0301–0306 / 0201–0205 / 0107 / CI 去 Gitea 化 + 布局收口**（见 git 历史与 archive）

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。**仓库根没有 `Cargo.toml`** ——
  在根目录直接跑 `cargo …` 会失败（坑 #8），一律用 `just` 转发。⚠️ **临时脚本里也一样**。
- **四个 crate 的分工**：`akasha-core`（Session 模型 + 配置模型与判据，**零 Tauri 依赖**）、
  `akasha-pty`（`Transport` + portable-pty + 合批 + `teardown` + **`watchdog`**）、
  **`akasha-store`**（库的打开 / 创建 / 四套池 / dump / 导出与还原 —— 唯一允许 `unsafe` 的地方；
  **app 依赖它**，但只用"落点、状态与解锁"三样）、
  `akasha`（app 包 = IPC 薄壳 + 托盘 + 配置 + **数据目录推导与便携目录检查** + 关窗语义 + 单实例 +
  退出钩子 + 看门狗接线 + 事件 + **库的解锁状态**（`vault::Vault`）+ 代码生成 bin）。
- **前端四层**：`src/ipc/`（唯一允许碰后端）、`src/tabs/`、`src/terminal/`、`src/App.tsx`。
- **调试白屏**：Victauri 的 `logs {action:"console"}`；读不到"模块执行期就抛错"的失败 ——
  那时临时往 `index.html` 塞 `window.onerror` 钩子（坑 #35）。
- **调试 E2E / 真 app**：`just test-e2e` 的 app 日志落在 `$tmp/akasha-e2e-app1.log` / `-app2.log`；
  在**同一个 bash 调用**里才能同时读到 app 与测试（坑 #33）。
- **调试托盘**：走会话总线查 `org.kde.StatusNotifierWatcher` 的注册表与 `com.canonical.dbusmenu`
  的布局（坑 #62 / #63）。
- **调试"点叉之后怎么了"**：`app_state { probe: "lifecycle" }`；日志里还有 `config loaded` /
  `config not found` / `config invalid` / `close behavior degraded`。
  ⚠️ 它比 Victauri 的发现目录**晚**：没登记时读到的是 `{"initialized":false}`（坑 #93）。
- **调试"库在哪、锁上没锁"**：`vault_status` 的 `path` / `state` / `unlocked`；日志里
  `vault created` / `vault unlocked` / `vault locked` / `vault unlock failed`。
- **调试"为什么起不来"**：`portable data dir not writable`（+ 退出码 2）—— 便携目录存在但写不进去；
  日志里紧挨着的 `path=` 就是要修的那个目录。
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
    ⚠️ **它不认识"下一步才存在"的命令**：plan 里出现的 `just <新配方>` 会让 docs-check 红 ——
    展开 plan 的那次提交要么先把配方落上，要么**先不写那个命令名**（plan 落地时就地补）。
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
    `Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`。**别只看字节数验收**。
32. **`u64` 不能直接过 IPC**：改用壳层 `u32` 句柄 + **checked** 转换。⚠️ 被生成器拒绝的是
    **一整类**（`usize` / `isize` / `i64` / `u64` / `i128` / `u128`）。
33. **沙箱里 E2E 必须与 app 在**同一次** bash 调用内**（每次调用都是独立的 bwrap）。
34. **`pkill -f <模式>` 会匹配到自己** —— 用 `pkill -f '[v]ite'` 或按 PID/进程组杀。
35. **`@xterm/addon-unicode11` 需要 `allowProposedApi: true`**。
36. **不在门禁里的测试等于没测**：`just test-e2e` 既不在 `ready` 里、又要真 app。
37. **`git mv` 之后 `docs-check` 会同时验两件事**（文件在不在、索引指得对不对）。
38. **每加一个依赖就多一份要维护的放行**。
39. **"测试自己抛的异常"会污染同一 app 上后跑的用例**。
40. **vite 默认只监听 `[::1]:1420`**；`/tmp/victauri/<pid>/` 里的 `pid` **就是 app 的 pid**。
41. **`(cmd) &` 在 fish 里是命令替换，不是子 shell**。
42. **`cargo test` 一次收多个 `--test` 时按目标名字母序跑**，不按参数顺序。
43. **`tauri-plugin-log` 默认的 `TargetKind::LogDir` 会让"日志目录不可写"变成 app 打不开**。
44. **`tracing/log-always` 会把依赖树的 TRACE 一起转成 `log` 记录**。
45. **SIGKILL 的投递是异步的**。
46. **portable-pty(unix) 的 `Child::kill()` 不是纯 SIGKILL**。
47. **早于日志插件注册的 `tracing` 事件会静默消失**。
48. **`/proc/<pid>` 存在 ≠ 进程还活着**（僵尸也有目录项）。
49. **按"命令行里含某段文本"找进程会误伤**。
50. **`term.dispose()`（xterm）会抛，而它跑在 React 的 effect 清理函数里**。
51. **多标签之后 DOM 选择器不再唯一**。
52. **断言超时不一定是"慢"**：真实原因可能是**界面已经被卸载**。
53. **`tauri-specta` 的事件必须 `mount_events`**。
54. **官方 `Channel` 不会告诉你"流结束了"**。
55. **会话里还有别的进程握着 PTY 时，主端读不到 EOF**。
56. **接线一次的回调必须走 `ref`**。
57. **日志消息里的"括号解释"会自己长大**。
58. **`tauri-plugin-log` 默认 formatter 的时间戳只到秒**。
59. **`signal` 字段的值是本地化的**（zh_CN 下 `SIGKILL` 写成 `已杀死`），尚未修。
60. **托盘在 Linux 上要写盘** —— 只读 runtime dir 里建不起来，所以它只能是**可选能力**。
61. **`libayatana-appindicator3` 与老的 `libappindicator3` 都是运行时 dlopen**。
62. **dbusmenu 的 item id 会随菜单重建而改变**。
63. **SNI 注册用的是唯一名**（`:1.x`）。
64. **Victauri 的 `window` 工具能机器验证窗口状态**。
65. **`AppHandle::exit()` 也会触发 `RunEvent::ExitRequested`**。
66. **便携数据目录就在 bin 同目录**，而 `just dev` 与 `just test-e2e` **共用同一个 bin**。
67. **Victauri 的 REST 兜底接口返回的是 `{"result": …}` 包了一层**（而 `just test-e2e` 里的
    `VictauriClient` 走 MCP，`call_tool` 直接返回工具内容本身）。
68. **`/proc/<pid>/exe` 可能带 ` (deleted)` 后缀**。
69. **文档里的"事实"没人核对就会自己长大**。
70. **cargo 的 workspace lint 继承是"全有或全无"**，而 `forbid` 不能被 `allow` 覆盖。
71. **带 `links = "..."` 的原生库在依赖树里只能有一个版本**。
72. **SQLCipher 的空 key 不是"静默关掉加密"，而是"返回错误且不挂 codec"**。
73. **SQLite 自己建出来的库文件是 644**（umask 022），不是 0600。
74. **`PRAGMA cipher_settings` 的输出是一列 `pragma` 行**。
75. **cargo-deny 的图根是"manifest 指向的那个包"，不是整个 workspace**。
76. **"0 字节的库"不是"空库"，是"还没有密钥"**。
77. **`cipher_memory_security` 是进程级、单向的**。
78. **模块内的 `#[cfg(test)] mod tests` 也要自己 `allow(clippy::unwrap_used)`**。
79. **`/proc/<pid>/mem` 的读用 `FOLL_FORCE`，绕过页保护**。
80. **`/proc/self/smaps` 的字段不是处处都有，而且属性行带缩进**。
81. **子串匹配会把规则变成笑话**：按 `_` 分词、整词比较，并配一对命中 / 诱饵负例。
82. **Victauri 的 `get_registry` 在本仓库是空的**（命令都没标 `#[inspectable]`）。
83. **clippy 会把"两个常量比较"的断言判红**（`assertions_on_constants`）—— 搬进 `const { … }`。
84. **E2E 配方两段可能跑在**不同**的数据目录里** —— 两段都先 `mkdir -p` 便携目录。
85. **集成测试的共用脚手架放 `tests/common/mod.rs`，但必须自己 `#![allow(dead_code)]`**。
86. **`pkill -f <pattern>` 会匹配到你自己那条命令行**。
87. **"什么都没发生"这类判据最容易写成永真式** —— 先用负例确认它会红。
88. **`sqlcipher_export` 写出来的文件默认是版本 0**（它不传递 `user_version`）。
89. **"导出另开一条实现路径"是最贵的那种省事**。
90. **扫描器自己就住在它要扫的地址空间里** —— 缓冲复用 + 读完即擦 + 真随机的针。
91. **给"要扫的段"设上限 = 让该看见的副本落在窗口外**（漏扫与没泄是同一条绿）。
92. **解锁期间 `VmLck` 涨的大头不是我们那一页**：`cipher_memory_security` 会给 SQLCipher 的
    每次分配 `mlock`（实测 152 kB 里 148 kB）。
93. **"发现目录出现" ≠ "app 就绪"**：Victauri 的插件 setup 比 app 自己的 `.setup()` 早，
    所以刚连上时 `lifecycle` probe 还是 `{"initialized":false}`。**它正是我们那条拒绝启动的检查
    所在的位置之后**才登记 —— 判据要"等那个字段自己出现"（有界），别把中间态当答案。
    同一类还有第二层：`invoke_command` 走 webview bridge，是**最后**才好的一个，
    所以"能连上 MCP"也不等于"能 `invoke_command`"。
94. **`no-println` 规则豁免的是 `**/tests/**`，不是 `#[cfg(test)] mod tests`** ——
    在 `src/` 的测试模块里写 `eprintln!("跳过：…")` 会直接红在 lint 上。
    正解：单测的"跳过"换成**不依赖环境**的 fixture（见下一条），要打印就放进 `tests/`。
95. **用 `chmod` 造"不可写"在单测里站不住**（以 root 跑、或文件系统不理会 mode 位时就造不出来，
    而"跳过"又会踩上一条）。改用**结构性**造法：把路径指到一个普通文件底下 —— `ENOTDIR`
    连 root 也绕不过去。要"app 真的看见一个不可写目录"时再用 `chmod`，且**自己先试写一次**
    当正对照（写不进去才继续）。
96. **clippy 的 `undocumented_unsafe_blocks` 本来就看私有项**（1.98 实测）：去掉
    `clippy.toml` 之后，**私有**函数 `apply_key` 里的 `unsafe` 照样被报（把 `// SAFETY:`
    改名即红）。所以别照抄内核的 `check-private-items`（它服务的是另一类 lint）；
    反过来，哪天它不再报私有项，就是**静默失效** —— 那时才需要那个开关。
97. **只加 `undocumented_unsafe_blocks` 会漏掉另一半**：注释**写在了安全块上**要靠反向的
    `unnecessary_safety_comment` 才拦得住（探针：在一条安全语句上挂 `// SAFETY:` 会红）；
    `/// # Safety` 挂在安全函数上同理，靠 `unnecessary_safety_doc`。三条一起才闭环。
98. **文档里的"或 X"最容易凭空长出来**：`portable.md` 与 `scope.md` 都写着
    "（或便携标记文件）"，而代码里从来没有第二个标记 —— `config.rs` 的注释反而写着
    "目录本身就是标记，不需要额外再放一个标记文件"。**核查时把每个"或"当成一条待证断言**。
99. **"唯一一处"这类计数没有人核对就会腐烂**：规则 note 与两处代码注释都写着
    "全仓库唯一的 `unsafe` 单点"，实际是 **3 处**（生产 1 + 契约测试 2）。要么写成
    "唯一**允许**的 crate"（结构性说法，能被规则守住），要么就别在注释里写数字。
100. **照搬外部规范时要分清"结构"与"语种"**：内核的 unsafe 规范其实只有两件事 ——
    `// SAFETY:` 紧贴块前说明"为什么 sound"、`# Safety` 写明契约。第一版把
    "英文、句首大写、句末句号"也一起搬了过来，而那只是内核注释本来就写英文。
    结论：**解释写中文**（与本仓库其余注释一致），只把**标签字面量**钉死（clippy 认它）。
