# STATUS

> **唯一的现状来源。覆盖写，不追加**（追加会无限增长并变得不可读）。
> 会话结束前必须更新 —— 下一个会话（或另一个 agent）只读这个文件 + 相关 plan 就能接手，
> 不需要回溯对话历史。规则见 [`docs/README.md`](./README.md)。

**最后更新**：2026-09-12

## 一句话

**阶段 2「端到端最小终端」走到 4/5**：输出合批（0201）、IPC raw 字节通道（0202）、
前端 xterm + WebGL 渲染（0203）、**真正退出零残留（0204）** 都已落地并实测 ——
关窗口退出与 panic 两条路径**退出后一个子进程都不剩（含忽略 SIGHUP 的那种）**。
剩下那条 [`0205`](./plans/0205-sigkill-exit-residue.md)：`tauri dev` 重编译重启走的是
**SIGKILL**（上游 `SharedChild::kill()`），进程没有任何执行代码的机会 —— 需要内核级兜底
（PDEATHSIG）或监管进程；本轮只把**实测证据**留下。

本轮最值钱的一条判据是"**忽略 SIGHUP 的进程**"：前台/后台的普通 `sleep` 靠内核的
session 级 SIGHUP 本来就不会残留，**拿它当判据等于没测**。换成
`sh -c 'trap "" HUP; …'` 之后，改前实测**三条路径都残留且逐次累积**（1 → 2 → 3 个）。

`ROADMAP.md` 共 52 个条目（10 个阶段）：阶段 1 完成 5/7（CI 与 E2E 入口都待 CI 实跑），
阶段 2 完成 **4/5**。**CI 仍未真正跑过** —— 仓库没有配置任何 git remote。

## 已验证为绿（命令 + 实际结果）

| 命令 / 检查 | 结果 |
|---|---|
| `just ready`（fmt-check + lint + test + deny-offline + **gen-types-check** + docs-check） | 退出码 **0**，6/6 全绿 |
| `just test` | **53 tests run: 53 passed**（`akasha` 7 + `akasha-core` 8 + `akasha-pty` 29 + E2E 9） |
| **E2E 入口** `just test-e2e`（自包含：起 Vite + app → 跑完全部目标 → 收尾） | 退出码 **0**，**9 个用例全绿**：`smoke` 3 / `integration` 2 / `session_channel` 1 / `terminal_render` 2 / `exit_residue` 1。目标现在**逐个串行跑**（见坑 #42） |
| ↑ 退出零残留（plan 0204） | ✅ 关窗口后 app 真退出、**忽略 SIGHUP 的探针被收掉**（`exit_residue` 的断言就是它） |
| ↑ panic 路径（plan 0204） | ✅ 临时把 `greet` 改成 panic 实测：hook 先打崩溃现场 → `shut_down=2 failures=[]` → 日志两条「会话已显式回收」→ `abort()`；探针同步消失 |
| ↑ `tauri dev` 重载路径（plan 0204） | ❌ **实测会残留**：`just dev` + 改 Rust 文件 → 新一代 app 起来了、上一轮的探针**仍活着**。原因确定（上游 SIGKILL），移交 plan 0205 |
| ↑ 终端判据 | `renderer = webgl`、`canvas` 2 块、DOM 行容器 **0** 个；按键 → `akasha-probe-42` 出现在屏幕（求值结果，不是回显）；8 MB 分 **133 批**送达、排空哨兵出现、队列归零、**之后仍可交互**；`WEBGL_lose_context` 后退到 canvas **且屏幕内容保留** |
| ↑ 会话判据 | `yes \| head -c 10000000` 的 **11 280 957** 字节分 **168 批**送达 JS，帧类型 = `ArrayBuffer`（JSON 帧 **0** 个）；关闭后收到频道**收尾帧 1 个**，console 里**零 error** |
| ↑ 平台差异 | 本机（Wayland）`smoke::screenshot_captures_window` **显式跳过**并打印原因（原生句柄是 Wayland surface，Victauri 只认 Xlib/Xcb/Win32/AppKit）；CI 矩阵在 X11 / Windows / macOS 上真跑这条。`exit_residue` 的探针**只在 Linux 起**（会话级回收只有 Linux 有实现） |
| `pnpm build`（`tsc && vite build`） | 退出码 0；产物 839 kB / gzip 229 kB。生产包里**没有** mock 与探针（`模拟后端` / `mockIPC` / `__akashaTerminal` 命中数 **0**） |
| **CSP** | `csp` 与 `devCsp` 均非 `null`；把 `devCsp` **临时设成与 `csp` 相同**也真跑过一轮 —— 渲染 / IPC / 8 MB 灌流全部照常、零 console error |
| **日志** | `tauri-plugin-log` 只挂 stdout、级别显式定在 `Info`：回收/panic 有记录（见上面 panic 行），app 日志 **234 行**（默认级别下曾刷到几十万行，见坑 #44） |
| `just gen-types` / `just gen-types-check` | 生成 `src/ipc/bindings.ts`；比对通过（本轮**没有**加减 command，生成物无差异） |
| `just bench`（criterion，配方 #20） | 52.7 GiB/s（容量路径）/ 14.1 ns 每批 / 9.64 GiB/s（**0201 的数字，本轮未复跑**） |
| `just check` / `just clippy`（`--workspace --all-targets`） | 退出码 **0** |
| `just deny-offline` | `bans ok, licenses ok, sources ok`（新增的直接依赖 `rustix` 本就在依赖树里，许可证在 allow 列表内） |
| `just docs-check` | 三部分全过（ROADMAP **52** 条目在 3 行内 / plan **45** 份 ≤200 行且索引一致） |
| `ast-grep scan` | 退出码 **0**；**四条**规则均已用正负例验证 |
| `cargo tree -p akasha-core` / `-p akasha-pty` \| `grep -c tauri` | **0** / **0**（分层成立） |
| `just dev` / `just doctor` | 起窗口；Vite 1420；Victauri 发现文件写在 `/tmp/victauri/<pid>/`；`just doctor` 13/13（沙箱内） |

> `just deny`（含 advisories）**尚未验证** —— 需要联网拉 RustSec 数据库。
> 终端的 console 输出：**零 error**；只有 xterm 自己的 2 条
> `warn task queue exceeded allotted deadline by N ms`（启动挂载时出现，本机 MESA
> 软件渲染栈下稳定复现，非 error）与降级时的 1 log + 1 warn。

## 待验证（本地跑不了 / 沙箱跑不了）

- **CI 三个 job 是否真能变绿** —— 仓库还没有 remote，从没跑过。三条只在真 runner 上见分晓的
  风险记在 [`docs/plans/0102`](./plans/0102-ci-platform-matrix.md) 的实施记录里。
- **E2E 矩阵的三个格子**（Linux/xvfb + macOS + Windows，三格跑同一条 `just test-e2e`）——
  同上：没有 remote 就没跑过。本地只覆盖 **Linux/Wayland** 这一格；`exit_residue` 在
  Windows/macOS 上**只验"关窗口 = 真退出"**，探针相关断言显式跳过（会话级回收只有 Linux 实现）。
- **宿主 MCP 连不到沙箱内运行的 app**（私有 PID / 临时目录）。沙箱内可用，
  但**必须让 app 与测试在同一次 bash 调用里**（坑 #33）。
- **`just dev-web` 的模拟后端没在真浏览器里点过**（本环境没有浏览器）：
  0203 的这条验收只做到 `pnpm build` + 代码审查 + `tsc` 类型检查。
- **大流量下的 JS heap 数字没取**：只验到"8 MB 灌完队列归零、界面仍可交互"。

## 当前基线（2026-09-12 实测，workspace root = `src-tauri/`）

| 项 | 实测结果 |
|---|---|
| workspace root | `/home/lycurgus/akasha/src-tauri`；成员 = `akasha` / `akasha-core` / `akasha-pty` |
| 二进制落点 | `src-tauri/target/debug/akasha`（另有代码生成工具 `gen-types`，故必须 `default-run`，坑 #29） |
| 增量重编译 | 6.09–6.26s（纯逻辑 crate 改动同样触发） |
| 出字节路径 | PTY read → 合批（64 KiB / 16 ms）→ `Channel<InvokeResponseBody>` **raw** → JS `ArrayBuffer` → `term.write` |
| 退出回收路径 | `RunEvent::Exit` / panic hook → `Sessions::shutdown_all()` → `Transport::shutdown()`（PTY：**收整个 session** → kill 子进程 → wait 收尸） |
| 前端渲染器 | **WebGL**（WebKitGTK + MESA 软件栈下仍拿到 WebGL2）；`canvas` 元素 2 块；DOM 渲染器未启用（`.xterm-rows` 为 0） |
| 大输出实测 | 11.18 MB / 170 批（0202）；10.50 MB / 158 批（0203 复测）；**11.28 MB / 168 批**（2026-09-12） |
| 前端产物 | 839 kB（gzip 229 kB）—— xterm + React 占绝对多数，暂不做代码分割 |
| 合批参数 | `max_bytes` = 64 KiB、`max_delay` = 16 ms（`BatchPolicy::DEFAULT`，唯一来源） |
| 日志 | `tauri-plugin-log`，**只 stdout**，级别 `Info`；`tracing` 的事件靠 `tracing/log-always` 转发成 `log` 记录 |
| CSP | `csp`：`default-src 'self'; connect-src ipc: http://ipc.localhost; img-src 'self' data:; style-src 'self' 'unsafe-inline'; font-src 'self' data:`；`devCsp` 多一个 `ws://localhost:1420 http://localhost:1420` |
| 前提条件 | **需要能写 `$HOME`**；沙箱内会刷 `dconf-CRITICAL` 与 WebKit 缓存 hard-link 告警，但 app 仍正常起窗口 |

## 进行中 / 下一步

- [~] **plan 0102（CI 平台矩阵，GitHub Actions 一份）**：本地部分完成，
  最终判据 = **推上去三个 job 全绿**，卡在没有 remote
- [~] **E2E 入口**（[plan 0107](./plans/0107-e2e-entry.md)）：本地已实测 ——
  `just test-e2e` 自包含、9 个用例全绿、自起路径零残留；剩 CI 三平台格子（同上面那条）
- [ ] **阶段 2 第五步**（也是阶段 2 唯一的缺口）：[`docs/plans/0205`](./plans/0205-sigkill-exit-residue.md)
  （被 SIGKILL 的退出路径也零残留）。开工前先定方案并写 ADR：
  Linux `PR_SET_PDEATHSIG`（要自带 spawn 辅助程序或给上游提 PR，且必须由**长驻线程**发起
  spawn —— PDEATHSIG 的"父"是线程不是进程）还是**监管进程**（更重，但阶段 5/6 也要它）
- [ ] **阶段 3 托盘**（[plan 0301](./plans/0301-tray-icon-menu.md)）：注意它建立在
  "退得干净"之上；而"点叉收托盘"要在 `ExitRequested` 里 `prevent_exit` ——
  0204 的回收**故意**挂在 `RunEvent::Exit`，别挪到 `ExitRequested`

### 本轮完成（plan 0204：真正退出零残留）

- [x] **`Sessions::shutdown_all()`**：整份搬出清单（不握锁 wait）→ 逐个 `Transport::shutdown()`
  → 一起摘牌；**幂等**（退出路径可能触发多次）；失败**记账不上抛**；单测用假载体验
  「每个会话恰好一次 shutdown + 两张表一起清空 + 失败也摘牌」
- [x] **`Sessions` 改 `Arc<Mutex<Inner>>` + `Box<dyn Transport>`**：退出钩子/panic hook 与命令
  共享同一份表（tauri 的 `State` 只借给命令）；装箱同时让 `shutdown_all` 可用假载体测，
  也为阶段 5/6 的 SSH/隧道留位
- [x] **`akasha-pty::teardown::kill_session`**：Linux 扫 `/proc` 的 session id 逐个 SIGKILL
  （`Child::kill()` 收不走走**忽略 SIGHUP** 的进程 —— 实测三条路径都残留）
- [x] **挂两条退出路径**：`RunEvent::Exit`（**不用** `ExitRequested`，理由见上）与 panic hook
  （打印崩溃现场 → 尽力回收 → `abort()`，与 release 的 `panic = "abort"` 一致）
- [x] **E2E `exit_residue`**：真 app 里开一个忽略 SIGHUP 的进程 → 关窗口 → 断言 app 退出
  **且探针被收掉**；复用别人的 app 时**显式跳过**（`AKASHA_E2E_OWNS_APP`）
- [x] **日志管道接通**：`tauri-plugin-log`（stdout、Info）+ `tracing/log-always`，
  回收与 panic 都有记录（否则那些 `tracing` 事件会**静默消失**）

### 上一轮完成（plan 0107：E2E 入口）

- [x] 探针修在源头：频道的**收尾帧**（`{index, end:true}`）不是数据帧；官方 `Channel` 先判
  `'end' in raw` 再取 `raw.message`
- [x] `just test-e2e` 自包含：有 app 就复用、没有就自起 Vite + app，跑完 reap 到真退出；
  新增 E2E 目标不接入 `E2E_TARGETS` 就红
- [x] 平台能力显式表达（Wayland 下截图用例跳过并打印原因，`--nocapture` 是必需的）

### 上一轮完成（plan 0203：前端 xterm + WebGL 渲染）

- [x] `src/terminal/surface.ts`（xterm + webgl/canvas/fit/search/serialize/unicode11；
  `term.write` 是全工程唯一调用点）、`attach.ts`（会话 ↔ 渲染面命令式接线）、
  `TerminalPane.tsx`（React 只管状态）、`src/ipc/mock.ts`（`just dev-web` 用）
- [x] `src-tauri/tests/terminal_render.rs`（画布渲染 + 按键来回 + 8 MB 不卡死；WebGL 降级）
- [x] CSP 最小化放行（`csp` + `devCsp`）

### 更早

- [x] **plan 0202**：`Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`（不是
  `Channel<Vec<u8>>`，见坑 #31）；会话命令；`tauri-specta` 生成 `src/ipc/bindings.ts`；
  ast-grep 规则 `no-string-pty-channel`
- [x] **plan 0201**：`OutputBatcher`（注入时钟）+ `spawn_batcher`；criterion 基线 + `just bench`
- [x] **CI 去 Gitea 化 + 吃透 GitHub 专属能力**；**阶段 1 布局收口**（`crates/` → `src-tauri/crates/`）

### 已定案（cyrene 裁定）

| 项 | 结论 |
|---|---|
| **Rust 成员位置** | **全部收在 `src-tauri/` 下**，仓库根不放 Rust 成员或 manifest（2026-09-11，见 ADR-0004） |
| **CI** | **只维护 GitHub Actions 一份**；不做别的 forge 的兼容层（理由见 plan 0102） |
| **IPC 字节通道** | **raw**：`Channel<InvokeResponseBody>` + `InvokeResponseBody::Raw`；`Channel<Vec<u8>>` 是 JSON 数组，由规则拦下（见坑 #31） |
| **类型边界** | **Rust 是唯一真相源**：`tauri-specta` 生成 `src/ipc/bindings.ts`。唯一手写处 = raw 频道的构造 |
| **渲染器** | **WebGL**；不可用退 **canvas**；两者都不可用**明确报错**，不静默落到 DOM |
| **退出回收** | 挂 `RunEvent::Exit`（**不挂** `ExitRequested` —— 阶段 3 的收托盘在那里 `prevent_exit`）；PTY 的 `shutdown` **收整个 session**，不只是 shell |
| **CSP** | **不为 `null`**：最小放行 + `devCsp` 只多 HMR 的 `ws://` |
| **日志** | `tauri-plugin-log` **只 stdout**（LogDir 目标会让"日志目录不可写"变成 app 打不开）；级别 **Info** |
| **会话句柄** | 过 IPC 用壳层 `u32` + **checked** 转换，不过 `u64` |
| P3 | **撤销** —— 豁免 webview 及其依赖栈的一切写入；判据改为"搬走文件夹后还能开" |
| SSH 实现 | **纯 Rust `russh`**，不调系统 `ssh` |
| `~/.ssh/config` | 只支持受限子集；遇 `Match`/`Include` **显式报错** |
| Bitwarden 接入 | `bw` CLI 作**用户自备前置**（不打包）+ v1 只读导入 |
| 命名 | 后端容器叫 `Session`；字节载体叫 `Transport`；**后端类型名不得编码 UI 呈现方式** |
| 连接模型 / 生命周期 | 不复用连接；连接生命周期 = 拥有它的 `Session`；关 `Session` 立刻断连 |
| 重连 | 3 次 + 指数退避，然后标记失败 |
| 传输落盘 | 临时名 + 原子重命名；不做断点续传 |
| `libudev` | 做成 cargo feature，仅 Linux 编译时启用 |
| `akasha-vt` | 维持延后；若必要则建于 `src-tauri/crates/akasha-vt/` |
| 性能基线 | criterion 数字**不进门禁**（`AGENTS.md` §7） |

### 待实测 / 待确认

- [ ] **CI 首次推送实跑**（三平台 E2E 矩阵 + `gen-types-check` 在 runner 上的耗时）
- [ ] **非 Linux 的会话级回收**：Windows 要 Job Object、macOS 要 `proc_listpids` + `getsid`；
  现在两处的 `exit_residue` 探针会留下（**已知缺口**，不是"顺手忽略"）
- [ ] **SIGTERM / SIGINT**（`kill <pid>`、Ctrl+C）不跑钩子 —— 今天不比改前差，但也没变好；
  候选方案记在 plan 0205
- [ ] 前端渲染的内存表现（heap 数字未取）；`just dev-web` 的模拟后端未在真浏览器点过
- [ ] `bw` 对 `sshKey` 条目的非交互行为（需真实 vault）；可搬迁性收尾（见 `portable.md`）
- [ ] 托盘的 Linux 依赖 `libayatana-appindicator3` 已在 CI apt 列表里，需实测

## 结构现状（容易找错地方）

- **workspace root 在 `src-tauri/`**（ADR-0004）。`Cargo.lock` / `deny.toml` / `target/` 都在那里。
  **仓库根没有 `Cargo.toml`** —— 在根目录直接跑 `cargo …` 会失败（坑 #8），一律用 `just` 转发。
- **三个 crate 的分工**：`akasha-core`（Session 模型，零依赖）、`akasha-pty`
  （`Transport` + portable-pty + 输出合批 + **`teardown`（会话级回收）**）、
  `akasha`（app 包 = IPC 薄壳 + 退出钩子 + 代码生成 bin）。
- **前端三层**：`src/ipc/`（唯一允许碰后端；`bindings.ts` 生成物禁止手改）、
  `src/terminal/`（`surface.ts` / `attach.ts` / `TerminalPane.tsx`）、`src/App.tsx`。
- **调试白屏**：Victauri 的 `logs {action:"console"}` 读 webview console；读不到"模块执行期
  就抛错"的那种失败 —— 那时临时往 `index.html` 塞 `window.onerror` 钩子再用 `eval_js` 读（坑 #35）。
- **调试 E2E**：`just test-e2e` 的 app 日志落在 `$tmp/akasha-e2e-app.log`（失败时自动 tail）；
  在同一个 bash 调用里才能同时读到 app 与测试（坑 #33）。
- **文档三级粒度**：`ROADMAP.md`（判据）→ `docs/plans/TTxx-*`（手段）→ 本文件的坑（痕迹）。
  完成的 plan **整份移入 `docs/plans/archive/`**（不拼接、不追加）。
- 命令入口分两处：项目级在根 `justfile`，crate 级在 `src-tauri/justfile`。
  **权威清单在 `docs/just.md` §2**（21 个配方），由 `just docs-check` 强制同步。

## 踩过的坑（避免重复踩）

1. **`victauri-test` 生成的测试需要消费方自己加 `tokio` dev-dependency**。
2. **`cargo deny init` 模板里 `[licenses] allow = []` 的含义是"拒绝一切许可证"**。
3. **just 的变量写 `$p`，不是 Make 的 `$$p`**。
4. **just 用 justfile 所在目录作为配方工作目录**。
5. **系统库缺失只在 cargo 构建脚本阶段暴露**；本机是 CachyOS（Arch 系），不是 apt。
6. **Tauri 没有 Rust 热重载，Victauri 也不提供**。
7. **just 的 shebang 配方需要可写的 runtime dir**，受限环境会失败。
8. **仓库根没有 `Cargo.toml`** → 根目录下一切 cargo 命令失败。
9. **`victauri-test` 生成的 `tests/*.rs` 不符合 rustfmt 默认风格** —— 跑一次 `just fmt`。
10. **CI 里 `libappindicator3-dev` 已不存在**，要用 `libayatana-appindicator3-dev`。
11. **受限环境下"写工作区之外被拒"看起来像工具/代码故障** —— 识别 → **直接提权重试**。
12. **`git checkout <file>` 会静默丢弃未提交的改动** —— 负例自检用 `cp` 备份/还原。
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
    在 fish / bash / sh 下语义相同（用 `sh -c '…' &`）。踩到的样子极具误导性：那一行把 shell
    **挂住**（命令替换要等 `sleep 600` 结束），于是**后面所有**"敲命令"的用例一起超时，
    看起来像"终端坏了 / 前端回归了"。
42. **`cargo test` 一次收多个 `--test` 时按目标名字母序跑**，不按参数顺序 ——
    会"关掉 app"的用例（`exit_residue`）会**第一个**跑。要按顺序就得**逐个目标各跑一条**。
43. **`tauri-plugin-log` 默认的 `TargetKind::LogDir` 会让"日志目录不可写"变成 app 打不开**：
    插件初始化失败 = `build()` 失败（实测 `PluginInitialization("log", "只读文件系统")`）
    → 终端不该因为日志文件写不了就起不来（现在只留 stdout）。
44. **`tracing/log-always` 会把依赖树的 TRACE 一起转成 `log` 记录**（含平时看不见的
    `tracing::span::active`）。默认级别下 app 日志被刷到几十万行并明显拖慢 app —— 级别要显式定。
45. **SIGKILL 的投递是异步的**：`kill()` 返回后立刻读 `/proc/<pid>/stat` 会读到 `R`，
    那不是"没杀掉"。判据必须等"消失"（有截止时间的轮询），不能立刻断言。
46. **portable-pty(unix) 的 `Child::kill()` 不是纯 SIGKILL**：它先发 **SIGHUP**、等 5×50 ms
    宽限，再退到 SIGKILL；而且它只管那个 shell —— 会话里其他进程得自己收（plan 0204）。

## 环境

CachyOS（Arch 系）/ rustc 1.98.1 / cargo 1.98.1 / node 26.8.2 / pnpm 12.3.4 / mise 2026.9.1
